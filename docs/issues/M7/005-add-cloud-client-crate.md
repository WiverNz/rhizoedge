# Issue M7-005 — Create the cloud-client crate

**Milestone:** M7 · **PRD:** [PRD 070](../../prd/070-cloud-sync-and-storage.md) · **Depends on:** M7-003, M3-013

## Context

The HTTP client the edge uses. Its error classification feeds ADR-014's
retry policy, so the variants must map cleanly onto Transient, Permanent, and
rate-limited.

## Goal

Provide a typed, classifiable client for the cloud API.

## Scope

- `CloudClient::send_batch` returning per-event results
- `CloudError` with Transport, Server, BadRequest, RateLimited variants
- `Classify` impl with an exhaustive match
- Request timeout and connection pooling
- Time conversion: integer millis to RFC 3339, in this crate only

## Non-goals

- The drain loop (M7-006).

## Dependencies

- M7-003
- M3-013

## Implementation notes

ADR-013 notes the two time representations (integer ms on MQTT, RFC 3339 on
HTTP) as a real seam where bugs hide. Doing the conversion in exactly one crate
and round-trip testing it (M7-011) is the mitigation.

`RateLimited` must carry `Retry-After` when present so M7-006 can honour it
rather than applying its own backoff over the server's instruction.

## Acceptance criteria

- [ ] `send_batch` returns per-event results.
- [ ] Each error condition maps to the correct variant.
- [ ] `Classify` matches exhaustively with no catch-all.
- [ ] 429 carries `Retry-After` when present.
- [ ] Timeouts produce `Transport`, classified Transient.
- [ ] Time conversion happens only in this crate.

## Verification

```bash
cargo test -p rhizo-cloud-client
```

## Tests required

- Each error variant's classification.
- 429 parsing.
- Timeout handling.
- Result mapping.

## Documentation impact

- None.

## Files likely affected

```text
crates/cloud-client/src/lib.rs
crates/cloud-client/src/error.rs
```
