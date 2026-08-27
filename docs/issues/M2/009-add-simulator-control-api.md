# Issue M2-009 — Add the simulator control API

**Milestone:** M2 · **PRD:** [PRD 020](../../prd/020-device-simulator.md) · **Depends on:** M2-001

## Context

Scenario tests need to inject faults and set state mid-run without
restarting. The control API is simulator-only and must be visibly separate from
protocol code so nobody mistakes it for a device capability.

## Goal

Expose a small HTTP control surface for tests.

## Scope

- Axum server on `--control-port` (default 9090)
- `POST /sim/fault`, `POST /sim/state`, `GET /sim/state`, `POST /sim/restart`, `GET /sim/scale`
- Bound to loopback only
- In a module clearly marked simulator-only

## Non-goals

- Any equivalent on real firmware — it must not exist there.

## Dependencies

- M2-001

## Implementation notes

Keep it in `src/control/` with a module doc stating it is a test affordance
with no firmware counterpart. The risk is a future reader inferring that devices
have an HTTP control surface.

`GET /sim/scale` exists so M8-004 can assert the edge and simulator agree on the
time scale.

Consider feature-gating it out of release builds — the simulator is never
deployed, so this is tidiness rather than security. Decide and note the choice.

**Decided: not feature-gated.** `--no-control-api` disables it at runtime
instead. A feature gate would produce two builds of the component whose whole
job is to be the reference device, and a scenario suite run against the
gated-out build would lose fault injection silently — a green suite testing less
than it claims. Loopback-only binding is the containment that matters, and it is
unconditional. Recorded in the module documentation.

## Acceptance criteria

- [x] Faults can be enabled and disabled at runtime.
- [x] State can be set (e.g. moisture) and read back.
- [x] `POST /sim/restart` restarts with a new `boot_id`.
- [x] `GET /sim/scale` reports the configured factor.
- [x] The server binds to loopback only.
- [x] The module documents that it is simulator-only.

## Verification

```bash
curl -X POST localhost:9090/sim/fault -d '{"fault":"leak","enabled":true}'
curl localhost:9090/sim/state
curl localhost:9090/sim/scale
```

## Tests required

- Each endpoint.
- Loopback-only binding.

## Documentation impact

- docs/testing/local-development.md section 7 already documents it.

## Files likely affected

```text
crates/device-simulator/src/control/mod.rs
```
