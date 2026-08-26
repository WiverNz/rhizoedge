# Issue M0-006 — Implement tracing subscriber and metrics registry

**Milestone:** M0 · **PRD:** [PRD 000](../../prd/000-platform-foundation.md) · **Depends on:** M0-002

## Context

ADR-010 requires structured logging with correlation fields and a Prometheus
text endpoint. Delivering this in M0 means no later milestone invents its own
logging, and field names stay consistent across all three binaries.

## Goal

Provide the shared observability wiring every binary uses.

## Scope

- `init_tracing(format, filter)` supporting JSON and pretty output
- A metrics registry with counter, gauge, and histogram helpers
- `render_prometheus()` producing the text exposition format
- Metric name constants module (empty; populated per milestone)
- `RUST_LOG`-compatible filtering

## Non-goals

- Any specific metric (added per milestone).
- OpenTelemetry export (explicitly deferred in ADR-010).

## Dependencies

- M0-002

## Implementation notes

JSON in production, pretty in development, selected by
`RHIZO_EDGE__LOG__FORMAT`. The subscriber must be structured so an OTel layer
could be added later without touching call sites.

Metric names live as constants in one module so a typo is a compile error rather
than a silently missing series. Keep the catalogue empty here — ADR-010 is
explicit that metrics are added when a real question needs them.

## Acceptance criteria

- [x] JSON output parses as JSON and includes level, target, and fields.
- [x] Pretty output is readable in a terminal.
- [x] `render_prometheus()` output parses as valid exposition format.
- [x] `RUST_LOG=debug` raises verbosity.
- [x] A structured field appears as a field, not interpolated into the message.

## Verification

```bash
cargo test -p rhizo-telemetry
RHIZO_EDGE__LOG__FORMAT=json cargo run -p edge-controller | head -3 | jq .
```

## Tests required

- JSON line shape.
- Prometheus rendering for each metric type.
- Filter application.

## Documentation impact

- Crate docs showing the correct structured-field idiom versus the wrong interpolated one.

## Files likely affected

```text
crates/telemetry/src/lib.rs
crates/telemetry/src/tracing_setup.rs
crates/telemetry/src/metrics.rs
crates/telemetry/src/names.rs
```
