# Issue M4-009 — Add the Axum API server with CORS configuration

**Milestone:** M4 · **PRD:** [PRD 040](../../prd/040-device-registry-and-health.md) · **Depends on:** M4-008

## Context

ADR-009 keeps the Edge API transport-agnostic and CORS-capable so a
browser-hosted frontend could be added later without Edge Controller changes.
V1 ships with CORS off.

## Goal

Serve the API with correct binding and optional CORS.

## Scope

- Axum server bound per `api.bind`, default `127.0.0.1:8080`
- CORS disabled by default; `RHIZO_EDGE__API__CORS_ALLOWED_ORIGINS` enables specific origins
- Request logging with `http_request_duration_seconds{route,status}`
- A request timeout and a body size limit
- The consistent error envelope on every route

## Non-goals

- Authentication — deferred, and stated as a V1 limitation.

## Dependencies

- M4-008

## Implementation notes

Loopback by default, and widening to a LAN address is an explicit
configuration act. With no authentication, the bind address *is* the security
boundary (ADR-011 section 5) and defaulting to `0.0.0.0` would quietly expose a
pump-control API.

CORS is wildcard-free: only named origins, never `*`.

## Acceptance criteria

- [ ] The server binds to loopback by default.
- [ ] A configured bind address is honoured.
- [ ] CORS is off by default.
- [ ] Named origins are permitted when configured; `*` is not accepted.
- [ ] Request metrics are recorded with route and status labels.
- [ ] Oversized bodies are rejected.

## Verification

```bash
cargo test -p edge-controller api::server
curl -s -H 'Origin: http://evil' -i localhost:8080/api/v1/devices | grep -i access-control  # absent
```

## Tests required

- Default bind.
- CORS off by default and on when configured.
- Wildcard origin rejected.
- Body limit.

## Documentation impact

- None.

## Files likely affected

```text
crates/edge-controller/src/api/mod.rs
crates/edge-controller/src/api/server.rs
```
