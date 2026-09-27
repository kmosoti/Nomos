# Nomos documentation

| Document | Contents |
|---|---|
| [PROJECT-SPEC.md](PROJECT-SPEC.md) | The full specification: model, semantics, invariants, phases |
| [architecture/](architecture/) | System components, hexagonal layout, runtime flows (Mermaid diagrams) |
| [formal/](formal/) | Algorithms, invariants and proof sketches |
| [adr/](adr/) | Architecture Decision Records |
| [CANON.md](CANON.md) | Canon language reference (to be written) |
| [PROTOCOL.md](PROTOCOL.md) | Loom/Cell control protocol (to be written) |
| [security-model.md](security-model.md) | Security model and hardening (to be written) |

## Building

The toolchain is pinned to Rust 1.98.1 (`rust-toolchain.toml`) and installs
automatically through rustup.

```sh
cargo check  --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt    --all --check
cargo test   --workspace
```
