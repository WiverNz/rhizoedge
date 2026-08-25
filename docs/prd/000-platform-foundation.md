# PRD 000 — Platform Foundation

**Milestone:** M0 · **Status:** PLANNED · **Depends on:** nothing

## Summary

Establish the repository, toolchain, build, lint, test, containerisation, and
observability baseline so that every later milestone adds behaviour rather than
scaffolding. No application behaviour is delivered.

## Problem

A project that grows its engineering baseline organically ends up with
inconsistent error handling, five backoff implementations, tests that need a
developer's specific machine, and a CI pipeline added after the bugs it would
have caught. The safety argument in this project depends on tests being cheap
and consistent; that is a property of the foundation, not of good intentions.

## Goals

1. A three-workspace Rust layout per [ADR-001](../adr/001-rust-workspace-and-crate-boundaries.md).
2. Rust **1.98.0** pinned so `-D warnings` is reproducible.
3. `fmt` / `clippy` / `test` green on an empty-but-real workspace.
4. Docker Compose skeleton with Mosquitto configured for authenticated access.
5. `rhizo-telemetry` providing tracing and metrics wiring.
6. `rhizo-testkit` skeleton with a `TestClock`.
7. Configuration loading with fail-fast validation.
8. A shared backoff utility.
9. CI running the full gate on every change.
10. Documentation structure with a working validator.

## Non-goals

- Any MQTT message handling (M1/M3).
- Any database schema (M3).
- Any domain logic (M1/M5/M6).
- Placeholder crates that exist only to look started. Every crate created here
  has real content: `telemetry` has a working subscriber, `testkit` has a
  working clock.

## User/system flows

Developer-facing only:

```text
clone → cargo test --workspace → green
      → docker compose up      → Mosquitto starts, auth required
      → cargo run --manifest-path tools/docscheck/Cargo.toml → docs structure validated
```

## Functional requirements

| ID | Requirement |
|---|---|
| F-000-01 | Root workspace with `exclude = ["firmware", "ui"]` and `[workspace.dependencies]` for all shared deps |
| F-000-02 | `rust-toolchain.toml` pins **Rust 1.98.0** with `rustfmt` and `clippy` |
| F-000-03 | `clippy.toml` with `disallowed-methods` reserved (populated in M1-013) |
| F-000-04 | Workspace lints: `unwrap_used` and `expect_used` denied for library crates, allowed in tests |
| F-000-05 | `rhizo-telemetry` builds a `tracing` subscriber selectable between JSON and pretty |
| F-000-06 | `rhizo-telemetry` exposes a metrics registry and Prometheus text rendering |
| F-000-07 | Edge configuration loads defaults → TOML → `RHIZO_EDGE__*` env → flags, and **exits non-zero** on invalid config |
| F-000-08 | Config `Debug` impl redacts `password`, `token`, `secret` |
| F-000-09 | Backoff utility: exponential with full jitter, configurable base/cap, reset on success |
| F-000-10 | `rhizo-testkit` provides `TestClock` with `set`/`advance` |
| F-000-11 | Docker Compose defines Mosquitto with anonymous access **disabled** and a generated password file |
| F-000-12 | Mosquitto ACL restricts each device to `rhizo/v1/devices/%u/#` |
| F-000-13 | `.env.example` documents every secret with placeholders; `.env` is gitignored |
| F-000-14 | CI runs fmt, clippy `-D warnings`, test, `docker compose config`, and `rhizo-docscheck` |
| F-000-15 | `rhizo-docscheck` (already written during planning) is adopted into the workspace and the CI gate |

## Interfaces

```rust
// rhizo-telemetry
pub fn init_tracing(format: LogFormat, filter: &str) -> Result<(), TelemetryError>;
pub fn registry() -> &'static Registry;
pub fn render_prometheus() -> String;

pub struct Backoff { base: Duration, cap: Duration, attempt: u32 }
impl Backoff {
    pub fn next_delay(&mut self) -> Duration;   // full jitter
    pub fn reset(&mut self);
}

// rhizo-testkit
pub struct TestClock { /* … */ }
impl TestClock {
    pub fn new(at: DateTime<Utc>) -> Self;
    pub fn set(&self, at: DateTime<Utc>);
    pub fn advance(&self, by: Duration);
}
```

No HTTP or MQTT interfaces exist yet.

## Data model

None. No database is created in M0 — the schema is an M3 deliverable, and
creating tables before the ingestion pipeline that fills them would be
scaffolding of exactly the kind this PRD forbids.

## State model

None. M0 delivers no stateful component.

## Failure modes

| Failure | Behaviour |
|---|---|
| Invalid configuration at startup | Fatal — log the specific key and exit non-zero |
| Missing config file | fall back to defaults + env; do not fail |
| Password-shaped key found in the TOML file | WARN and ignore (secrets belong in env, [ADR-011](../adr/011-configuration-and-secrets-model.md)) |
| Mosquitto password file absent | container fails to start with a clear message and a pointer to the generation script |

## Safety implications

No invariant is *enforced* in M0, but three are enabled:

- **SAFETY-007** — the workspace layout that lets the contract crate be shared
  with firmware is established here.
- **SAFETY-012** — the `unwrap_used`/`expect_used` deny and the fail-fast config
  posture set the "refuse rather than guess" default from the first commit.
- Mosquitto authentication and ACLs (F-000-11, F-000-12) are the identity
  boundary [ADR-012](../adr/012-device-identity-and-provisioning.md) relies on.

## Observability

`rhizo-telemetry` is delivered in M0 precisely so that no later milestone has to
invent its own logging. Metric names are not defined yet — the catalogue in
[ADR-010](../adr/010-observability-strategy.md) is populated per milestone.

## Testing strategy

- Unit: backoff bounds, jitter range, cap, reset; config layering precedence;
  redaction in `Debug`; `TestClock` arithmetic.
- Integration: `docker compose config` parses; Mosquitto rejects anonymous
  connections.
- Doc: `rhizo-docscheck` runs clean.

## Acceptance criteria

```bash
cargo fmt --all --check                                          # exit 0
cargo clippy --workspace --all-targets --all-features -- -D warnings   # exit 0
cargo test --workspace --all-features                            # exit 0
docker compose -f deploy/docker-compose.yml config               # exit 0
cargo run --manifest-path tools/docscheck/Cargo.toml                                       # exit 0
```

- [ ] Mosquitto starts and **rejects** an anonymous connection.
- [ ] `cargo run -p edge-controller -- --config missing.toml` exits non-zero with
      a specific message.
- [ ] A config containing `password` in the TOML logs a warning and is ignored.
- [ ] CI is green on a clean checkout with no local state.

## Dependencies

None. M0 is the root of the dependency graph.

## Open questions

1. **`figment` vs `config` for layered configuration.** Resolved during M0-005
   by comparing error message quality for a malformed key — a misconfigured edge
   that starts with silently wrong values is worse than one that refuses to
   start. Not a blocking question; either works.
2. **Exact pinned Rust version.** Resolved: **1.98.0**, recorded in
   `rust-toolchain.toml`. The firmware workspace may differ; see
   [ADR-007](../adr/007-esp32-rust-framework-and-toolchain.md).

## Future work

- CI caching strategy tuning (M8, once build times matter).
- systemd units for the home deployment (M13).
- Release/versioning workflow (M13).
