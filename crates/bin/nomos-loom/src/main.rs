//! Nomos Loom CLI and headless IPv6 coordinator entrypoint.

use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use nomos_canon::artifact::{CanonId, Profile, Reader, decode};
use nomos_cipher::CipherVault;
use nomos_cipher_vault::VaultCipherAdapter;
use nomos_cloud::CloudAuthConfig;
use nomos_cloud_gcp::{GcpCloudAdapter, GcpTokenProvider};
use nomos_core::cipher::Secret;
use nomos_core::governor::GovernorConfig;
use nomos_mesh::{MeshIdentity, MeshPort};
use nomos_mesh_headscale::HeadscaleMeshAdapter;
use nomos_mesh_wireguard::WireGuardMeshAdapter;
use nomos_protocol::ControlTransportMetrics;
use nomos_protocol_quic::{PinnedIdentity, QuicControlNode, parse_sha256_hex};

use nomos_loom::cli::{HELP_TEXT, LoomCommand, ServeArgs, parse_args};
use nomos_loom::governor_svc::{GovernorService, spawn_psi_reader};
use nomos_loom::kernel_svc::KernelService;
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
        Ok(LoomCommand::Trace {
            target,
            canon,
            state_dir,
        }) => run_plan_command(&target, &canon, &state_dir, false),
        Ok(LoomCommand::Enforce {
            target,
            canon,
            state_dir,
        }) => run_plan_command(&target, &canon, &state_dir, true),
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

fn run_plan_command(target: &str, canon: &Path, state_dir: &Path, enforce: bool) -> ExitCode {
    let bytes = match std::fs::read(canon) {
        Ok(b) => b,
        Err(err) => {
            eprintln!("read canon `{}`: {err}", canon.display());
            return ExitCode::FAILURE;
        }
    };
    let kernel = match KernelService::open(state_dir) {
        Ok(k) => k,
        Err(err) => {
            eprintln!("open kernel state `{}`: {err}", state_dir.display());
            return ExitCode::FAILURE;
        }
    };
    let next_gen = kernel.current_generation().saturating_add(1);
    let plan_id = format!(
        "loom-{}-{next_gen}",
        if enforce { "enforce" } else { "trace" }
    );
    match kernel.submit_plan(&plan_id, next_gen, None, &bytes, enforce) {
        Ok(summary) => {
            let _ = VaultCipherAdapter::default().status(None);
            let _ = HeadscaleMeshAdapter::default().identity();
            println!(
                "mode={} target={target} plan_id={} generation={} digest={} warp_vertices={} warp_edges={} persisted={}",
                if enforce { "enforce" } else { "trace" },
                summary.plan_id,
                summary.generation,
                summary.canon_digest_hex,
                summary.warp_vertices,
                summary.warp_edges,
                summary.persisted
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
    let mut client_pins = Vec::with_capacity(args.allowed_client_pins.len());
    for pin_hex in &args.allowed_client_pins {
        match parse_sha256_hex(pin_hex) {
            Ok(pin) => client_pins.push(pin),
            Err(err) => {
                eprintln!("invalid --allowed-client-pin `{pin_hex}`: {err}");
                return ExitCode::from(2);
            }
        }
    }
    if client_pins.is_empty() {
        eprintln!(
            "serve requires at least one --allowed-client-pin <SHA256_HEX> for mandatory mTLS"
        );
        return ExitCode::from(2);
    }

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
        let kernel = match KernelService::open(&args.state_dir) {
            Ok(k) => k,
            Err(err) => {
                eprintln!("kernel store init failed: {err}");
                return ExitCode::FAILURE;
            }
        };
        let governor = GovernorService::new(GovernorConfig::default());
        let _psi_task = spawn_psi_reader(governor.clone(), Duration::from_secs(2));

        let quic_identity = match PinnedIdentity::generate(&[&args.node_name, "localhost", "::1"]) {
            Ok(id) => id,
            Err(err) => {
                eprintln!("QUIC identity init failed: {err}");
                return ExitCode::FAILURE;
            }
        };
        let quic_node =
            match QuicControlNode::bind_ipv6(args.quic_listen, quic_identity, client_pins) {
                Ok(node) => node,
                Err(err) => {
                    eprintln!("QUIC bind `{}` failed: {err}", args.quic_listen);
                    return ExitCode::FAILURE;
                }
            };
        let quic_metrics = quic_node.clone();
        let handler = Arc::new(
            LoomQuicHandler::new(args.node_name.clone(), governor.clone(), leases.clone())
                .with_kernel(kernel),
        );
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
        state.allow_plaintext_key = args.allow_plaintext_key;
        if let Ok(tok) = std::env::var("NOMOS_LOOM_CONTROL_TOKEN")
            && !tok.trim().is_empty()
        {
            state = state.with_control_token(&Secret::new(tok));
        }
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
