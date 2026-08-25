# Issue M7-013 — Implement the cloud read endpoints

**Milestone:** M7 · **PRD:** [PRD 070](../../prd/070-cloud-sync-and-storage.md) · **Depends on:** M7-004

## Context

http-api-boundaries section 3.2. What matters as much as what these endpoints
do is what the cloud API **must not** have: no command endpoints, no config
writes, and no endpoint the edge polls for instructions.

## Goal

Expose historical reads, and nothing that could influence a device.

## Scope

- `GET /edges`, `/edges/{id}/devices`, `/edges/{id}/plants`
- `/edges/{id}/plants/{id}/measurements` with time range and resolution
- `/edges/{id}/plants/{id}/watering-events`
- Cursor pagination and range caps
- **No command endpoints, no config writes, no polling endpoint**

## Non-goals

- Any write path toward devices — architecturally forbidden.

## Dependencies

- M7-004

## Implementation notes

Add a test that enumerates the router's routes and asserts none matches a
command or config-write pattern. The absence is the architecture (ADR-003), and
an absence is not something review reliably catches.

## Acceptance criteria

- [ ] All read endpoints return the documented shapes.
- [ ] Time ranges and pagination work.
- [ ] **A route-enumeration test asserts no command or config-write endpoint exists.**
- [ ] 404 for unknown edges or plants.
- [ ] Range caps prevent unbounded responses.

## Verification

```bash
cargo test -p cloud-api api::
curl -s localhost:8081/api/v1/edges | jq
```

## Tests required

- Each endpoint.
- **The route-enumeration absence test.**
- Pagination and caps.

## Documentation impact

- http-api-boundaries.md verified.

## Files likely affected

```text
crates/cloud-api/src/api/read.rs
```
