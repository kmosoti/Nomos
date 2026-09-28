# Receipts

One NDJSON file per recording session, one line per executed check, written by `cargo xtask receipts record` and held to [`../receipt.schema.json`](../receipt.schema.json) and [`../checks.toml`](../checks.toml) by `cargo xtask receipts validate`. The [verification matrix](../../docs/formal/verification-matrix.md) is filled from these lines and from nothing else. See [`../README.md`](../README.md).
