# Issue M7-001 — Create the cloud-api binary and PostgreSQL service

**Milestone:** M7 · **PRD:** [PRD 070](../../prd/070-cloud-sync-and-storage.md) · **Depends on:** M6-019

## Context

ADR-005. The cloud is an append-only history sink, deliberately limited: it
originates no command and pushes no configuration.

## Goal

Create the cloud service skeleton with PostgreSQL.

## Scope

- `cloud-api` binary with Axum, config, and telemetry
- PostgreSQL added to Compose with a healthcheck and a named volume
- `sqlx` PostgreSQL pool
- `/health/live`, `/health/ready`, `/metrics`
- Task supervision matching the edge's

## Non-goals

- The schema (M7-002).
- Ingestion (M7-003).

## Dependencies

- M6-019

## Implementation notes

Reuse `rhizo-telemetry` so log field names and metric conventions match the
edge. Two services with different logging conventions make correlation
needlessly hard.

The cloud's readiness legitimately includes database reachability, unlike the
edge's exclusion of cloud reachability — the cloud genuinely cannot function
without its database.

## Acceptance criteria

- [ ] The binary starts and connects to PostgreSQL.
- [ ] Health endpoints respond.
- [ ] `/health/ready` is 503 when the database is unreachable.
- [ ] Compose starts PostgreSQL with a healthcheck.
- [ ] The volume persists across restarts.

## Verification

```bash
docker compose up -d postgres cloud-api
curl -s localhost:8081/health/ready | jq
```

## Tests required

- Startup.
- Readiness with the database down.

## Documentation impact

- None.

## Files likely affected

```text
crates/cloud-api/src/main.rs
crates/cloud-api/src/config.rs
deploy/docker-compose.yml
```
