# Architecture

How Nomos is built, from the whole system down to the crate boundaries.

| Document | Question it answers |
| --- | --- |
| [system.md](system.md) | What are the components, and how does the control loop move between them? |
| [hexagon.md](hexagon.md) | How is the code split into domain, ports, application, and adapters? |
| [runtime.md](runtime.md) | What happens at runtime: Canon compilation, Trace and Enforce, the Action lifecycle, and Loom ↔ Cell? |

Formal statements of the algorithms and invariants live in [formal/](../formal/). Decisions and their reasons live in [adr/](../adr/).
