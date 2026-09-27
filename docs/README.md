# Nomos documentation

| Document | Contents |
|---|---|
| [PROJECT-SPEC.md](PROJECT-SPEC.md) | The full specification: model, semantics, invariants, phases |
| [ARCHITECTURE.md](ARCHITECTURE.md) | Workspace layout, crates, ports and adapters, dependency rule |
| [adr/](adr/) | Architecture Decision Records |
| [CANON.md](CANON.md) | Canon language reference (to be written) |
| [PROTOCOL.md](PROTOCOL.md) | Loom/Cell control protocol (to be written) |
| [SECURITY.md](SECURITY.md) | Security model and hardening (to be written) |

## Building

The toolchain is pinned to Rust 1.98.1 (`rust-toolchain.toml`) and installs
automatically through rustup.

```
cargo check  --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt    --all --check
cargo test   --workspace
```
