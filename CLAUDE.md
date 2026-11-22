# Rhizo Edge — Claude development instructions

Follow [AGENTS.md](AGENTS.md) for shared development, safety, testing and Git
rules. Follow [ROADMAP.md](ROADMAP.md) for current priority and scoped acceptance.
The permanent strategy is working functionality first, essential safety retained,
and deeper validation after practical use. M0–M8 are delivered; M9 remains
incomplete; **M10-001 is next**. M10/M11/M12 implementation does not wait for
exhaustive M9 closure. No product feature was implemented by this planning update.

Implement small useful changes; investigate only relevant code and contracts.
Reuse existing Rust/MQTT/SQLite/REST infrastructure and passing evidence. Avoid
repeated audits, unnecessary web research, speculative edge cases and extensive
reports. Make routine choices autonomously and finish the requested scope.
Run relevant tests and a smoke test, not blanket project gates after each task.
Report only what changed, actual checks and remaining work.

The host and firmware are separate workspaces. Host Rust is pinned in
`rust-toolchain.toml`; ESP uses the documented exception in ADR-007.
`firmware/node-app` contains host-testable application safety logic; root
workspace tests do not run it. See [firmware/README.md](firmware/README.md) for
proven build commands and hardware status procedures. No hardware operations
without the local inventory and status check; no pump power before essential
control/reset safety checks. Local machine instructions remain untracked.

For changes to telemetry metrics, remember that `Metrics::new()` shares a
process-wide singleton; use the existing test gauge lock when asserting absolute
values. Consult existing code before inventing replacement infrastructure.

Historical implementation results remain in [milestone reports](docs/reports/)
and the [M9 acceptance inventory](docs/reports/M9-acceptance-freeze.md).
These record evidence and gaps; their old M10 restriction is superseded by the
2026-11-22 roadmap. Never convert incomplete evidence into a pass.
