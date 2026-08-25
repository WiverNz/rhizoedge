# Issue M4-008 — Implement the device REST endpoints

**Milestone:** M4 · **PRD:** [PRD 040](../../prd/040-device-registry-and-health.md) · **Depends on:** M4-004, M4-005, M4-006

## Context

http-api-boundaries section 2.3. The first operator-facing surface.

## Goal

Expose the device registry over HTTP.

## Scope

- `GET /devices`, `GET /devices/{id}`, `PATCH /devices/{id}`, `GET /devices/{id}/events`
- `GET /quarantined-messages`
- The response shape from http-api-boundaries section 2.3
- The consistent error envelope
- **`device_id` is immutable**; PATCH changes the display name only

## Non-goals

- Config and command endpoints (M6-013, M6-016).

## Dependencies

- M4-004
- M4-005
- M4-006

## Implementation notes

There must be no endpoint that changes `device_id`. ADR-012 makes it a
one-way decision that orphans history; the API offers a rename of the *display
name* so the common need does not require the dangerous operation.

Timestamps are RFC 3339 in the API even though storage is integer millis
(ADR-013) — do the conversion in one place.

## Acceptance criteria

- [ ] All endpoints return the documented shapes.
- [ ] 404 for an unknown device with the error envelope.
- [ ] PATCH changes the display name.
- [ ] **No endpoint changes `device_id`.**
- [ ] Timestamps are RFC 3339 with `Z`.
- [ ] Event listing supports `since` and `limit`.

## Verification

```bash
cargo test -p edge-controller api::devices
curl -s localhost:8080/api/v1/devices | jq
```

## Tests required

- Each endpoint's shape.
- 404 handling.
- An explicit test that device_id cannot be changed.

## Documentation impact

- http-api-boundaries.md verified accurate.

## Files likely affected

```text
crates/edge-controller/src/api/devices.rs
crates/edge-controller/src/api/error.rs
```
