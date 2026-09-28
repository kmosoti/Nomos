//! # nomos-warp
//!
//! **Hexagon: domain service.** Dependency and execution-graph engine.
//!
//! Compiles Variances and Obligations, with the Canon's relationships,
//! capabilities, and policy, into an Action DAG: dependency resolution, cycle
//! detection, topological ordering, `requires` / `after` / `on_change` edges,
//! conflict keys, and execution-frontier calculation.
