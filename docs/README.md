# Nomos Documentation

| Document | Contents |
| --- | --- |
| [PROJECT-SPEC.md](PROJECT-SPEC.md) | The specification: model, semantics, invariants, and phases |
| [architecture/](architecture/) | System components, hexagonal layout, and runtime flows |
| [formal/](formal/) | Algorithms, invariants, and proof sketches |
| [adr/](adr/) | Architecture decision records |
| [style/](style/) | Prose style specification and linting |
| [CANON.md](CANON.md) | Canon language reference (not yet written) |
| [PROTOCOL.md](PROTOCOL.md) | Loom ↔ Cell control protocol (not yet written) |
| [security-model.md](security-model.md) | Security model and hardening (not yet written) |

## Building

The toolchain is pinned to Rust 1.98.1 in `rust-toolchain.toml`, and rustup installs it automatically.

```sh
cargo check  --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt    --all --check
cargo test   --workspace
```
