# Issue M6-016 — Implement the watering and lockout REST endpoints

**Milestone:** M6 · **PRD:** [PRD 060](../../prd/060-irrigation-control-and-safety.md) · **Depends on:** M6-011, M6-003

## Context

http-api-boundaries section 2.6 — the safety-critical endpoints. **409 is the
safety response**, carrying the structured lockout reason so the UI can explain
why. There is no override parameter, anywhere.

## Goal

Expose watering actions that cannot bypass the gate.

## Scope

- `POST /plants/{id}/water` running the gate and returning 409 or 202
- `POST /plants/{id}/auto-watering/enable` and `/disable`
- `POST /plants/{id}/lockout/clear` returning 409 if the condition is still active
- `GET /commands/{command_id}`
- `POST /devices/{id}/commands/tare` and `/calibrate`
- **No override, force, or bypass parameter on any endpoint**

## Non-goals

- Any endpoint that skips the domain gate — forbidden.

## Dependencies

- M6-011
- M6-003

## Implementation notes

Every actuation request goes through `rhizo_domain::evaluate`. An HTTP
handler that published MQTT directly would nullify SAFETY-003 and SAFETY-004,
and it would be easy to write by accident while adding a 'quick manual test'
endpoint.

409 bodies carry `{ reason, since, clearable, message }` so the UI can render
what will clear the lockout (PRD 120 F-120-21).

`lockout/clear` on an active leak returns 409 — that is the explicit reset
SAFETY-003 requires, and it must verify the signal is gone.

## Acceptance criteria

- [ ] `POST /water` during a leak returns **409** and publishes nothing.
- [ ] The 409 body names the reason and whether it is clearable.
- [ ] A permitted manual dose returns 202 with the `command_id`.
- [ ] Manual watering succeeds under sensor fault but not under leak.
- [ ] `lockout/clear` on an active condition returns 409.
- [ ] **No endpoint accepts an override or force parameter.**
- [ ] Every actuation path calls `evaluate`.

## Verification

```bash
cargo test -p edge-controller api::water
cargo test safety_003_leak_blocks_manual_api
grep -rn 'force\|override' crates/edge-controller/src/api/   # expect none
```

## Tests required

- **`safety_003_leak_blocks_manual_api`.**
- 409 body content.
- Manual permitted under sensor fault.
- Clear refused while active.
- A grep-based test for override parameters.

## Documentation impact

- http-api-boundaries.md verified.

## Files likely affected

```text
crates/edge-controller/src/api/watering.rs
crates/edge-controller/src/api/lockout.rs
```
