//! # nomos-warp
//!
//! **Hexagon: domain service.** Dependency and execution-graph engine.
//!
//! Compiles resources and Variance into an Action DAG: dependency resolution,
//! cycle detection, topological ordering, `requires` / `after` / `on_change`
//! edges, conflict keys, and execution-frontier calculation.
