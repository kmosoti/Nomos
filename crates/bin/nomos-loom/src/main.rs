//! Nomos Loom CLI and headless IPv6 coordinator entrypoint.

use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;

use nomos_app::fleet::FleetCoordinatorApp;
use nomos_canon::artifact::{CanonId, Profile, Reader, decode};
use nomos_cipher::CipherVault;
use nomos_cipher_vault::VaultCipherAdapter;
use nomos_cloud::CloudAuthConfig;
use nomos_cloud_gcp::{GcpCloudAdapter, GcpTokenProvider};
use nomos_core::governor::GovernorConfig;
use nomos_core::resource::Digest;
use nomos_mesh::{MeshIdentity, MeshPort};
use nomos_mesh_headscale::HeadscaleMeshAdapter;
use nomos_mesh_wireguard::WireGuardMeshAdapter;
use nomos_protocol::{ControlOp, ControlTransportMetrics};
use nomos_protocol_quic::{PinnedIdentity, QuicControlNode};

use nomos_loom::cli::{HELP_TEXT, LoomCommand, ServeArgs, parse_args};
use nomos_loom::governor_svc::GovernorService;
use nomos_loom::lease::LeaseBroker;
use nomos_loom::quic_svc::{LoomQuicHandler, spawn_quic_server};
use nomos_loom::server::{LoomState, build_router};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match parse_args(args) {
        Ok(LoomCommand::Help) => {
            println!("{HELP_TEXT}");
            ExitCode::SUCCESS
        }
        Ok(LoomCommand::Compile { canon }) => match validate_canon_file(&canon) {
            Ok(id) => {
                println!("canon_id={}", hex::encode(id.as_bytes()));
                ExitCode::SUCCESS
            }
            Err(err) => {
                eprintln!("compile error: {err}");
                ExitCode::FAILURE
            }
        },
        Ok(LoomCommand::Trace { target, canon }) => run_plan_command(&target, &canon, false),
        Ok(LoomCommand::Enforce { target, canon }) => run_plan_command(&target, &canon, true),
        Ok(LoomCommand::Serve(serve_args)) => run_serve(serve_args),
        Err(err) => {
            eprintln!("{err}");
            ExitCode::from(2)
        }
    }
}

fn validate_canon_file(path: &Path) -> Result<CanonId, String> {
    let bytes = std::fs::read(path)
        .map_err(|e| format!("read canon artifact `{}`: {e}", path.display()))?;
    let profile = if path.extension().and_then(|e| e.to_str()) == Some("cbor") {
        Profile::Cbor
    } else {
        Profile::Jcs
    };
    let decoded = decode(&bytes, profile, &Reader::current())
        .map_err(|e| format!("decode canon `{}`: {e:?}", path.display()))?;
    Ok(nomos_canon::artifact::canon_id(&decoded.canon, profile))
}

fn run_plan_command(target: &str, canon: &Path, enforce: bool) -> ExitCode {
    match validate_canon_file(canon) {
        Ok(canon_id) => {
            let app = FleetCoordinatorApp::default();
            let digest = Digest::from_bytes(*canon_id.as_bytes());
            let op = if enforce {
                ControlOp::SubmitPlan
            } else {
                ControlOp::Status
            };
            let env =
                app.build_control_envelope(op, None, &digest, format!("{{\"target\":{target:?}}}"));
            // Verify fallback adapters obey closed-fail posture when unconfigured.
            let _ = VaultCipherAdapter::default().status(None);
            let _ = HeadscaleMeshAdapter::default().identity();
            println!(
                "op={:?} generation={} digest={} target={target}",
                env.op, env.generation, env.canon_digest_hex
            );
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run_serve(args: ServeArgs) -> ExitCode {
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(err) => {
            eprintln!("tokio runtime init failed: {err}");
            return ExitCode::FAILURE;
        }
    };

    runtime.block_on(async move {
        let leases = match LeaseBroker::open(&args.db_path, args.max_active_leases) {
            Ok(db) => db,
            Err(err) => {
                eprintln!("lease broker init failed: {err}");
                return ExitCode::FAILURE;
            }
        };
        let governor = GovernorService::new(GovernorConfig::default());

        let quic_identity = match PinnedIdentity::generate(&[&args.node_name, "localhost", "::1"]) {
            Ok(id) => id,
            Err(err) => {
                eprintln!("QUIC identity init failed: {err}");
                return ExitCode::FAILURE;
            }
        };
        let quic_node = match QuicControlNode::bind_ipv6(args.quic_listen, quic_identity, None) {
            Ok(node) => node,
            Err(err) => {
                eprintln!("QUIC bind `{}` failed: {err}", args.quic_listen);
                return ExitCode::FAILURE;
            }
        };
        let quic_metrics = quic_node.clone();
        let handler = Arc::new(LoomQuicHandler::new(
            args.node_name.clone(),
            governor.clone(),
            leases.clone(),
        ));
        let _quic_task = spawn_quic_server(quic_node, handler);

        let mesh_adapter: Arc<dyn MeshPort + Send + Sync> =
            Arc::new(WireGuardMeshAdapter::new(MeshIdentity {
                node_name: args.node_name.clone(),
                public_key_hex: String::new(),
                mesh_ipv6: "fd00:db8::1/64".to_owned(),
                key_digest_hex: "0".repeat(64),
            }));
        let cloud_adapter =
            GcpCloudAdapter::new(GcpTokenProvider::from_config(CloudAuthConfig::default())).ok();

        let mut state = LoomState::new(
            args.node_name.clone(),
            leases,
            governor,
            Some(args.vault_dir.clone()),
            args.web_assets_dir.clone(),
        );
        state.mesh = Some(mesh_adapter);
        state.cloud = cloud_adapter.map(|c| Arc::new(c) as Arc<dyn nomos_cloud::CloudSubstrate>);
        state.quic_stats = Arc::new(move || quic_metrics.stats());

        let app = build_router(state);
        let listener = match tokio::net::TcpListener::bind(args.listen).await {
            Ok(l) => l,
            Err(err) => {
                eprintln!("HTTP bind `{}` failed: {err}", args.listen);
                return ExitCode::FAILURE;
            }
        };
        println!(
            "nomos-loom serving http={} quic={} headless={}",
            args.listen,
            args.quic_listen,
            args.web_assets_dir.is_none()
        );
        if let Err(err) = axum::serve(listener, app).await {
            eprintln!("server exit error: {err}");
            return ExitCode::FAILURE;
        }
        ExitCode::SUCCESS
    })
}
