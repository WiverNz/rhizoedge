# Issue M13-007 — Add the notification dispatcher

**Milestone:** M13 · **PRD:** [PRD 130](../../prd/130-multi-plant-home.md) · **Depends on:** M13-001

## Context

PRD 130 F-130-34, which restates SAFETY-008's principle for a new outbound
dependency: **a notification failure must never affect control**.

## Goal

Alert the operator without adding a way for the control loop to block.

## Scope

- Dispatch on: lockout set, device offline beyond threshold, pump fault, no-delivery, cloud sync broken beyond threshold
- Channels: ntfy, generic webhook, SMTP
- **Rate-limited and deduplicated** — one leak, one notification
- Configurable per severity
- **Fire-and-forget from a separate task**, exactly like the cloud outbox
- `notification_log` recording every attempt

## Non-goals

- Cloud-based notification services.

## Dependencies

- M13-001

## Implementation notes

The separate task is the requirement. A notification sent inline from the
control loop would let a slow SMTP server delay a watering decision — precisely
the coupling the outbox pattern exists to prevent.

`notification_log` distinguishes 'the alert was never generated' from 'it was
generated and delivery failed', which are different bugs.

## Acceptance criteria

- [ ] Each trigger dispatches a notification.
- [ ] One leak produces exactly one notification, not one per tick.
- [ ] All three channels work.
- [ ] Severity filtering works.
- [ ] **A dead channel does not delay the control loop**, asserted by tick duration.
- [ ] Every attempt is recorded in `notification_log`.
- [ ] A notification storm is coalesced.

## Verification

```bash
cargo test -p edge-controller notify::
curl -X POST localhost:8080/api/v1/notifications/test
```

## Tests required

- Per-channel dispatch.
- Deduplication.
- **Dead channel does not affect tick duration.**
- Storm coalescing.

## Documentation impact

- Notification configuration documentation.

## Files likely affected

```text
crates/edge-controller/src/notify/mod.rs
migrations/edge/0004_notifications.sql
```
