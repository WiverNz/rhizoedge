# ADR-014 — Failure classification and retry policy

## Status

Accepted — 2026-08-25. Applied from M3; cloud parameters in M7.

**Extended 2026-08-26** with the device-side bounded event buffer and its tiered
overflow policy ([ADR-015](015-device-offline-autonomy.md) §6) — the same
value-tier reasoning as the edge outbox, applied under far tighter storage.

## Context

The system retries in five different places: MQTT connection, MQTT publish,
SQLite transactions, cloud sync, and device-side Wi-Fi. Without a shared policy
these drift into five subtly different implementations, and each one is an
opportunity to either give up too early (losing data) or retry too aggressively
(a thundering herd against a recovering service, or a hot loop that burns a
Raspberry Pi's CPU).

There is also a correctness dimension: **retrying an operation that is not
idempotent causes duplicate watering.** The policy must distinguish what may be
retried from what may not.

## Decision

### Classify every failure into exactly one of three kinds

```rust
pub enum FailureKind {
    Transient,   // retry with backoff — the operation may succeed later
    Permanent,   // never retry — quarantine and surface
    Fatal,       // the process cannot continue correctly — exit non-zero
}
```

| Failure | Kind |
|---|---|
| MQTT connection refused / dropped | Transient |
| MQTT publish buffer full | Transient (bounded, then Permanent for that command) |
| `SQLITE_BUSY` | Transient |
| `SQLITE_FULL` / disk full | Fatal for the write; the controller stops issuing commands |
| Malformed MQTT payload | Permanent (quarantine) |
| Envelope/topic `device_id` mismatch | Permanent (quarantine) |
| Cloud 5xx, timeout, DNS failure | Transient |
| Cloud 429 | Transient, honour `Retry-After` |
| Cloud 4xx (other) | Permanent (quarantine the event) |
| Migration failure at startup | Fatal |
| Invalid configuration at startup | Fatal |
| Control-loop task panic | Fatal |

The classification is a function, not a convention:
`fn classify(&Error) -> FailureKind`, unit-tested per error variant. This is
what keeps the five retry sites consistent.

### Backoff: exponential with full jitter

```text
delay(attempt) = random_uniform(0, min(cap, base * 2^attempt))
```

**Full jitter**, not "exponential plus a small random addition". With a fleet of
devices and an edge all reconnecting after a broker restart, the naive form
retains the synchronised retry pattern that caused the problem; full jitter
spreads attempts uniformly across the whole window. The cost — an occasional
very short delay — is irrelevant here and the benefit is real once there is more
than one client.

| Site | base | cap | max attempts |
|---|---|---|---|
| MQTT connection (edge) | 1 s | 60 s | unlimited |
| MQTT connection (device) | 2 s | 300 s | unlimited |
| MQTT publish (command) | 200 ms | 2 s | **3, then fail the command** |
| SQLite transaction on BUSY | 50 ms | 500 ms | 3 |
| Cloud sync batch | 1 s | 300 s | unlimited (see below) |
| Device Wi-Fi association | 2 s | 300 s | unlimited |

The attempt counter resets on success.

### Why cloud sync retries forever

The outbox is durable history. Dropping an event because the cloud was down for
a week would silently lose the record of what the machine did to a plant.

But "retry forever" needs two guards:

1. **Attempt-count visibility.** After 10 failed attempts an event is still
   retried at the cap, but `cloud_sync_failures_total` and
   `cloud_last_success_timestamp_seconds` make the situation obvious. There is
   no silent forever-loop.
2. **A bounded queue.** At `outbox_max_rows` (default 500 000), pruning begins —
   **value-tiered**, oldest first:
   - `value_tier = 'low'` (measurements) are pruned;
   - `value_tier = 'high'` (watering events, commands, lockouts, device faults)
     are preserved and never pruned.

   History is nice to have. The ledger of what the machine did to a living thing
   is not optional, and that distinction is encoded as a column
   ([ADR-004](004-sqlite-edge-persistence-model.md)).

### What must never be retried blindly

**A watering command publish is retried at most 3 times, and a failure marks the
command `failed` rather than re-issuing a new one.**

The reasoning: the edge cannot distinguish "the publish failed" from "the
publish succeeded and the acknowledgement was lost". If it were to issue a
*fresh* command with a new `command_id` after a publish failure, and the
original had in fact been delivered, the device would see two distinct commands
and water twice — the device's dedup ring keys on `command_id` and would not
catch it.

So the rule is: **retry the same `command_id`, never generate a new one.** MQTT
QoS 1 redelivery of the same payload is safe because the device deduplicates on
`command_id` (SAFETY-001). After 3 attempts the command is marked `failed`, the
irrigation state returns to `Recheck`, and the next tick re-evaluates from fresh
soil data. A failed publish is **never** recorded as a watering event.

This is the single most important paragraph in this ADR.

### Quarantine rather than infinite retry for permanent failures

A permanently-failing item at the head of a queue blocks everything behind it.
Both queues therefore quarantine:

- Malformed MQTT messages → `quarantined_messages`, capped at 1000 rows,
  rate-limited to 10/min per device so a babbling device cannot fill the disk.
- Cloud-rejected events → `pending_cloud_events.status = 'quarantined'`, with
  the batch continuing past them.

Quarantined items are visible through the API and are an operator decision, not
a system one.

### Fatal means exit

A process that cannot do its job must not appear healthy. On a Fatal failure the
edge logs at ERROR with full context and exits non-zero, letting the supervisor
restart it. Applies to migration failure, invalid configuration, and any
long-running task panic.

The alternative — continuing with a dead control loop — is worse, because
monitoring reports "up" while nothing is watching the plant
([ADR-010](010-observability-strategy.md)).

### Error types

`thiserror` for library crates (typed, matchable, classifiable), `anyhow` only
at binary top level where the error is about to be logged and the process is
about to exit.

**No `unwrap()` or `expect()` in long-running paths.** Where an invariant is
genuinely impossible to violate, `expect()` is permitted with a message stating
*why* it cannot fail — and that message is the documentation. A clippy lint
(`unwrap_used`, `expect_used`) is enabled at `deny` for the library crates and
allowed in tests.

### Device-side event buffer while isolated

An ESP32 cannot retain unbounded history, and a design that pretends otherwise
fails silently in the field. The buffer is a bounded NVS ring with **tiered
retention**, mirroring the edge outbox's `value_tier`:

| Tier | Kinds | Overflow behaviour |
|---|---|---|
| **audit** | autonomous dose, refusal + reason, lockout set/cleared, policy activation, pump fault, leak | evict oldest audit event **and record a gap marker** |
| **telemetry** | measurement samples | evict oldest silently |

Audit events are never evicted to make room for telemetry. The record of what
the machine did to a living plant outranks a missing point on a chart — the same
judgement as `value_tier` at the edge, made under tighter constraints.

**A gap is data.** Eviction records the lost `device_seq` range and count, which
is replayed on reconnect, stored in `history_gaps`, and shown in the plant's
history. It is never silently absorbed (SAFETY-020).

Replayed events are retained until the edge acknowledges them with an
`event.ack` ([mqtt-v1.md](../protocol/mqtt-v1.md) §5.13) — not merely until the
broker acks the publish, which is a different fact — so an edge crash
mid-reconciliation loses nothing — the device simply replays again. Replay is
idempotent on the device-generated `event_id` (SAFETY-016).

### Device-side retry

Different constraints: no disk, limited RAM, and the pump must fail closed.

- Wi-Fi and MQTT reconnect with the same full-jitter policy, unlimited.
- Telemetry is **not** buffered across a disconnect beyond a small ring (16
  samples). Telemetry is a sample stream; stale samples on reconnect are of
  little value and unbounded buffering would exhaust RAM.
- **Command results are retried until acknowledged**, up to 60 s, because a
  result is ledger data — the edge needs to know what the pump did. If it
  ultimately fails to publish, the outcome is recorded in NVS and re-published
  after the next boot.
- The pump is never retried. A failed dose is reported, not repeated; the edge
  decides what happens next with fresh data.

## Alternatives considered

**Fixed-interval retry.** Rejected: either too slow to recover or too aggressive
during a long outage, and synchronised across clients.

**Exponential backoff without jitter.** Rejected: preserves the synchronised
retry storm that a broker restart creates.

**"Decorrelated jitter"** (AWS variant). A reasonable alternative; full jitter
was chosen for being simpler to reason about and to test, with no meaningful
difference at this scale.

**Dropping events after N attempts.** Rejected for high-tier events — that is
silent data loss of exactly the records that matter. Accepted for low-tier
events only under queue pressure, which is the value-tier design.

**Issuing a new `command_id` on publish failure.** Rejected — see above. It is
the most plausible route to duplicate watering in the whole design.

**Retrying pump actuation on the device.** Rejected: the device does not know
why the dose failed and cannot see the soil. Reporting and deferring to the edge
is the safe behaviour.

## Consequences

Positive:

- One backoff implementation in `rhizo-telemetry`, used by all five sites, with
  its own unit tests.
- Failure classification is testable per error variant rather than being
  scattered `match` arms.
- Command retry semantics make duplicate watering from a publish failure
  structurally impossible.
- A stuck queue cannot be caused by one bad item.

Negative, accepted:

- Unlimited cloud retry means a permanently misconfigured cloud URL retries
  forever at the 300 s cap. Visible in metrics, but it does not self-heal.
- The 3-attempt command publish limit means a brief broker hiccup can cause a
  missed dose. Correct: a missed dose is recoverable, a double dose is not.
- Quarantine requires operator attention; nothing drains it automatically.

## Risks

- **Classification drift** — a new error variant defaults to the wrong kind.
  *Mitigation:* `classify` matches exhaustively with no catch-all, so a new
  variant fails to compile until classified. Issue M3-013.
- **Outbox pruning removing something valuable** because its tier was set wrong.
  *Mitigation:* the tier is assigned at the single call site that writes the
  outbox row, and defaults to `'high'` — the safe default is to keep.
- **Retry loops masking a real bug** by making it look transient. *Mitigation:*
  `cloud_last_success_timestamp_seconds` and attempt counts are exported, and a
  first-failure ERROR log precedes the quieter retry WARNs.

## Follow-up

- M0-007 implements the backoff utility and its tests.
- M3-013 implements `classify` and the exhaustive-match guard.
- M6-011 implements command publish retry with the fixed `command_id` rule.
- M7-006 implements outbox drain backoff; M7-008 the value-tiered cap.
