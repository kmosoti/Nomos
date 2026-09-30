//! # nomos-cell
//!
//! **Driving adapter + composition root.** Node-resident executor CLI:
//! `traits`, `trace`, `enforce`, `import`, `events`
//! (`docs/formal/cell-commands.md`).
//!
//! Wires `nomos-app` to the Linux Substrate and the Cell's local store.

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let status = nomos_cell::cli::run(&args, &mut std::io::stdout(), &mut std::io::stderr());
    std::process::exit(status);
}
