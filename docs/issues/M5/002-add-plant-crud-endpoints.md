# Issue M5-002 — Implement the plant REST endpoints

**Milestone:** M5 · **PRD:** [PRD 050](../../prd/050-plant-model-and-recommendations.md) · **Depends on:** M5-001, M4-009

## Context

http-api-boundaries section 2.4. Plants are the operator's primary object.

## Goal

Expose plant CRUD and reads over HTTP.

## Scope

- `GET/POST /plants`, `GET/PATCH/DELETE /plants/{id}`
- `GET /plants/{id}/measurements` with `from`, `to`, `resolution`, and a 5000-point cap
- `GET /plants/{id}/watering-events`
- Cursor pagination on list endpoints
- The documented response shape

## Non-goals

- Recommendations (M5-012).
- Watering actions (M6-016).

## Dependencies

- M5-001
- M4-009

## Implementation notes

The raw-resolution cap prevents a year-long request from exhausting memory.
Return an error naming the cap rather than silently truncating, so the caller
knows the series is incomplete.

`resolution` accepts `raw|minute|hour|day`; only `raw` is implemented in M5, and
the others return 501 until M13-010 adds downsampling. Reserving the parameter
now keeps the API stable.

## Acceptance criteria

- [ ] All endpoints return the documented shapes.
- [ ] A new plant defaults to `auto_watering_enabled: false`.
- [ ] Measurement queries respect `from`/`to` and the 5000-point cap.
- [ ] Exceeding the cap returns a specific error, not truncated data.
- [ ] Cursor pagination works on list endpoints.
- [ ] 404 for unknown plants.

## Verification

```bash
cargo test -p edge-controller api::plants
curl -s localhost:8080/api/v1/plants | jq
```

## Tests required

- Each endpoint.
- Cap behaviour.
- Pagination.
- Default-off on create.

## Documentation impact

- http-api-boundaries.md verified.

## Files likely affected

```text
crates/edge-controller/src/api/plants.rs
```
