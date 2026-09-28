# Accept

Kubernetes gives you a reconciliation-based control plane for scheduling and maintaining container workloads across machines. The interesting part is the reconciliation model: you declare desired state, controllers observe actual state, and repeatedly drive the system toward convergence.

Rust buys us tighter control over memory layout, allocation, and runtime behavior. Go buys us a simpler concurrency model and usually lower implementation cost. For the data plane, those tradeoffs may point in a different direction than they do for the control plane.

The architecture removes the coordinator from the steady-state data path, so adding collectors does not require all events to traverse one central process. That reduces one obvious scaling bottleneck. It does not, by itself, solve metadata coordination or backpressure.

Before choosing Raft, define what actually needs consensus. If the system only needs single-writer metadata with failover, the problem is narrower than general replicated state-machine consensus. Raft becomes relevant once multiple nodes must agree on an ordered sequence of state changes despite failures.
