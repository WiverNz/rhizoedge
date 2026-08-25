# Issue M7-006 — Implement the outbox drain with backoff

**Milestone:** M7 · **PRD:** [PRD 070](../../prd/070-cloud-sync-and-storage.md) · **Depends on:** M7-005, M0-007

## Context

**SAFETY-008's mechanism.** The drain task is fully decoupled from the control
loop: the control loop writes a row and moves on, so cloud latency cannot enter a
control path even accidentally.

## Goal

Ship outbox events to the cloud without ever blocking control.

## Scope

- A drain task selecting pending rows where `next_attempt_at <= now`
- Batch up to 500, ordered by `created_at`
- 2xx: mark synced. 4xx per event: quarantine. 5xx/timeout: attempts++, `next_attempt_at = now + full_jitter(backoff)`
- Backoff base 1 s, cap 300 s, unlimited attempts
- 429 honoured via `Retry-After`
- **No shared lock or awaited call with the control loop**

## Non-goals

- Batch adaptation (M7-007).
- The outbox cap (M7-008).

## Dependencies

- M7-005
- M0-007

## Implementation notes

The decoupling is the requirement. Assert it: `control_tick_duration_seconds`
must be unaffected by a stopped cloud, and there must be no `.await` on the
client inside any control path.

Retry forever is correct for durable history, but it needs the visibility of
M7-009's metrics so a permanently misconfigured URL is obvious rather than
silent.

Log the **first** failure of an outage at ERROR and subsequent retries at WARN,
so a week-long outage does not produce a week of ERROR lines.

## Acceptance criteria

- [ ] Events sync when the cloud is available.
- [ ] **The control loop is unaffected while the cloud is down**, asserted by tick duration.
- [ ] Backoff delays increase within bounds and reset on success.
- [ ] 429 honours `Retry-After`.
- [ ] Per-event 4xx quarantines that event only.
- [ ] The first failure logs ERROR, subsequent WARN.
- [ ] No control path awaits the cloud client.

## Verification

```bash
cargo test -p edge-controller outbox::
cargo test --test integration cloud_outage
grep -rn 'cloud_client' crates/edge-controller/src/control/  # expect none
```

## Tests required

- Drain success.
- **Control loop unaffected during an outage.**
- Backoff bounds.
- 429 handling.
- Per-event quarantine.

## Documentation impact

- None.

## Files likely affected

```text
crates/edge-controller/src/cloud/drain.rs
```
