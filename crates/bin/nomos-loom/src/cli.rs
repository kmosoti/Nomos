//! Command-line parsing for `nomos-loom` (`compile`, `trace`, `enforce`, `serve`).

use std::net::SocketAddrV6;
use std::path::PathBuf;

/// Configuration options for headless `nomos-loom serve`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServeArgs {
    /// Pure-IPv6 HTTP API bind address (defaults to `[::1]:7440`).
    pub listen: SocketAddrV6,
    /// Pure-IPv6 QUIC RPC/datagram bind address (defaults to `[::1]:7443`).
    pub quic_listen: SocketAddrV6,
    /// SQLite WAL database path for credential and workload leases.
    pub db_path: PathBuf,
    /// Local age/PQ vault directory for metadata posture reporting.
    pub vault_dir: PathBuf,
    /// Local `FsContentStore` + `FileLog` state directory for kernel persistence.
    pub state_dir: PathBuf,
    /// Optional prebuilt static web assets directory served via HTTP fallback.
    pub web_assets_dir: Option<PathBuf>,
    /// Logical coordinator node name reported in telemetry and QUIC heartbeats.
    pub node_name: String,
    /// Maximum active leases permitted simultaneously.
    pub max_active_leases: usize,
    /// Mandatory mTLS client SPKI/cert SHA-256 hex pins allowed to connect over QUIC.
    pub allowed_client_pins: Vec<String>,
    /// Explicit `--allow-plaintext-key` opt-in for non-TPM2 dev/CI vault keys.
    pub allow_plaintext_key: bool,
}

impl Default for ServeArgs {
    fn default() -> Self {
        Self {
            listen: "[::1]:7440".parse().expect("valid IPv6 default"),
            quic_listen: "[::1]:7443".parse().expect("valid IPv6 default"),
            db_path: PathBuf::from("/var/lib/nomos/loom-leases.sqlite"),
            vault_dir: PathBuf::from("/var/lib/nomos/vault"),
            state_dir: PathBuf::from("/var/lib/nomos/state"),
            web_assets_dir: None,
            node_name: "nomos-loom".to_owned(),
            max_active_leases: 64,
            allowed_client_pins: Vec::new(),
            allow_plaintext_key: false,
        }
    }
}

/// Parsed `nomos-loom` command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoomCommand {
    /// Prints CLI usage summary.
    Help,
    /// Validates a Canon artifact file and prints its digest.
    Compile {
        /// Path to compiled Canon JSON/CBOR file.
        canon: PathBuf,
    },
    /// Evaluates a non-mutating kernel + Warp trace against target predicate.
    Trace {
        /// Target predicate expression.
        target: String,
        /// Path to Canon artifact.
        canon: PathBuf,
        /// State directory for read-only Fence inspection.
        state_dir: PathBuf,
    },
    /// Dispatches fenced kernel + Warp convergence with durable `FsStore` persistence.
    Enforce {
        /// Target predicate expression.
        target: String,
        /// Path to Canon artifact.
        canon: PathBuf,
        /// State directory for `FsContentStore` and `FileLog`.
        state_dir: PathBuf,
    },
    /// Runs the headless IPv6 HTTP + SQLite WAL lease + PSI governor + mTLS QUIC RPC server.
    Serve(ServeArgs),
}

/// Usage help text printed for `--help` and invalid subcommands.
pub const HELP_TEXT: &str = "\
nomos-loom <COMMAND>

Commands:
  compile --canon <PATH>
  trace   --target <EXPR> --canon <PATH> [--state-dir <DIR>]
  enforce --target <EXPR> --canon <PATH> [--state-dir <DIR>]
  serve   [--listen <[v6]:port>] [--quic-listen <[v6]:port>] [--db-path <PATH>]
          [--vault-dir <PATH>] [--state-dir <DIR>] [--web-assets-dir <DIR>]
          [--node-name <NAME>] [--max-leases <N>] [--allowed-client-pin <SHA256_HEX>]...
          [--allow-plaintext-key]
";

/// Parses command-line arguments without external dependencies.
pub fn parse_args<I>(args: I) -> Result<LoomCommand, String>
where
    I: IntoIterator<Item = String>,
{
    let mut iter = args.into_iter();
    let Some(cmd) = iter.next() else {
        return Ok(LoomCommand::Help);
    };
    match cmd.as_str() {
        "-h" | "--help" | "help" => Ok(LoomCommand::Help),
        "compile" => {
            let mut canon = None;
            while let Some(flag) = iter.next() {
                match flag.as_str() {
                    "--canon" => canon = Some(PathBuf::from(next_val(&mut iter, "--canon")?)),
                    other => return Err(format!("unknown compile flag `{other}`")),
                }
            }
            let canon = canon.ok_or_else(|| "compile requires --canon <PATH>".to_owned())?;
            Ok(LoomCommand::Compile { canon })
        }
        "trace" | "enforce" => {
            let is_enforce = cmd == "enforce";
            let mut target = None;
            let mut canon = None;
            let mut state_dir = PathBuf::from("/var/lib/nomos/state");
            while let Some(flag) = iter.next() {
                match flag.as_str() {
                    "--target" => target = Some(next_val(&mut iter, "--target")?),
                    "--canon" => canon = Some(PathBuf::from(next_val(&mut iter, "--canon")?)),
                    "--state-dir" => {
                        state_dir = PathBuf::from(next_val(&mut iter, "--state-dir")?);
                    }
                    other => return Err(format!("unknown {cmd} flag `{other}`")),
                }
            }
            let target = target.ok_or_else(|| format!("{cmd} requires --target <EXPR>"))?;
            let canon = canon.ok_or_else(|| format!("{cmd} requires --canon <PATH>"))?;
            Ok(if is_enforce {
                LoomCommand::Enforce {
                    target,
                    canon,
                    state_dir,
                }
            } else {
                LoomCommand::Trace {
                    target,
                    canon,
                    state_dir,
                }
            })
        }
        "serve" => parse_serve(iter),
        other => Err(format!("unknown command `{other}`\n\n{HELP_TEXT}")),
    }
}

fn parse_serve<I>(mut iter: I) -> Result<LoomCommand, String>
where
    I: Iterator<Item = String>,
{
    let mut args = ServeArgs::default();
    while let Some(flag) = iter.next() {
        match flag.as_str() {
            "--listen" => {
                let raw = next_val(&mut iter, "--listen")?;
                args.listen = raw
                    .parse()
                    .map_err(|e| format!("invalid IPv6 --listen `{raw}`: {e}"))?;
            }
            "--quic-listen" => {
                let raw = next_val(&mut iter, "--quic-listen")?;
                args.quic_listen = raw
                    .parse()
                    .map_err(|e| format!("invalid IPv6 --quic-listen `{raw}`: {e}"))?;
            }
            "--db-path" => args.db_path = PathBuf::from(next_val(&mut iter, "--db-path")?),
            "--vault-dir" => args.vault_dir = PathBuf::from(next_val(&mut iter, "--vault-dir")?),
            "--state-dir" => args.state_dir = PathBuf::from(next_val(&mut iter, "--state-dir")?),
            "--web-assets-dir" => {
                args.web_assets_dir = Some(PathBuf::from(next_val(&mut iter, "--web-assets-dir")?));
            }
            "--node-name" => args.node_name = next_val(&mut iter, "--node-name")?,
            "--max-leases" => {
                let raw = next_val(&mut iter, "--max-leases")?;
                args.max_active_leases = raw
                    .parse()
                    .map_err(|e| format!("invalid --max-leases `{raw}`: {e}"))?;
            }
            "--allowed-client-pin" => {
                args.allowed_client_pins
                    .push(next_val(&mut iter, "--allowed-client-pin")?);
            }
            "--allow-plaintext-key" => args.allow_plaintext_key = true,
            "--help" | "-h" => return Ok(LoomCommand::Help),
            other => return Err(format!("unknown serve flag `{other}`")),
        }
    }
    Ok(LoomCommand::Serve(args))
}

fn next_val<I>(iter: &mut I, flag: &str) -> Result<String, String>
where
    I: Iterator<Item = String>,
{
    iter.next()
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| format!("{flag} requires a value"))
}
