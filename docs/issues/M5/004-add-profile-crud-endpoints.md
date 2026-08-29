# Issue M5-004 — Implement the profile REST endpoints

**Milestone:** M5 · **PRD:** [PRD 050](../../prd/050-plant-model-and-recommendations.md) · **Depends on:** M5-001, M5-003

## Context

http-api-boundaries section 2.7. Profiles carry the values the safety gate
consumes, so their editing surface matters.

## Goal

Expose profile CRUD with validation errors that teach.

## Scope

- `GET/POST /profiles`, `GET/PUT /profiles/{id}`
- 422 with the specific violated rule on invalid input
- A default profile seeded on first run

## Non-goals

- The validation rules themselves (M5-003).

## Dependencies

- M5-001
- M5-003

## Implementation notes

The 422 body must name the rule and the limit, e.g. `dose_ml (200) exceeds
the device hard limit FIRMWARE_MAX_ML_PER_RUN (80)`. ADR-011's reasoning: an
error at edit time teaches the real limit while the operator is paying
attention.

Seed one sensible default profile so a first-run system is usable without the
operator inventing numbers.

## Acceptance criteria

- [x] All endpoints return the documented shapes.
- [x] An invalid profile returns 422 naming the violated rule and the limit.
- [x] A default profile exists on first run.
- [x] Updating a profile affects plants using it on the next evaluation.

## Verification

```bash
cargo test -p edge-controller api::profiles
curl -s -X POST localhost:8080/api/v1/profiles -d '{"dose_ml":200,...}' | jq
```

## Tests required

- Each endpoint.
- 422 message content.
- Default profile seeding.

## Documentation impact

- None.

## Files likely affected

```text
crates/edge-controller/src/api/profiles.rs
```
