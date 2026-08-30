# Issue M6-022 — Implement durable pending command intents for sleeping devices

**Milestone:** M6 · **PRD:** [PRD 060](../../prd/060-irrigation-control-and-safety.md) · **Depends on:** M6-008, M6-009, M6-011, M6-016, M4-013

## Context

M6-008 and M6-009 establish the rule that makes the command pipeline safe:
persist the command row, **then** publish it, and a retry reuses the same
`command_id` rather than minting a new one. That rule assumes a device that is
listening.

A battery device is listening for a few seconds out of every fifteen minutes
([ADR-018](../../adr/018-battery-and-deep-sleep-device-mode.md)). Publishing to
it immediately means the message waits in the broker carrying a TTL minted before
the device even went to sleep, and by the time it is delivered it is expired —
which is fail-closed and therefore not dangerous, but it does mean manual
watering on a battery device would simply never work.

## Goal

Hold what the operator asked for, and mint the command when the device is
actually awake.

## Scope

- A `command_intents` table: `intent_id`, `plant_id`, `device_id`,
  `requested_ml`, `mode`, `created_at`, `intent_expires_at`, `state`,
  `command_id` (null until delivery), `refusal_reason`
- Intent states: `pending_for_device_wake` → `sent` | `refused` |
  `expired_before_wake`
- Routing at request time: a device that is `connected` takes the existing
  immediate path unchanged; a device that is `sleeping` takes the intent path
- The safety gate runs **twice** — once at request time to refuse obviously
  impossible requests early, and again in full at delivery
- Delivery on wake: allocate one `command_id`, persist the command row, publish,
  in exactly the M6-008/M6-009 order
- Ordering at wake: `edge.time` first (F-040-17), then the command; a
  `clock_unsynced` refusal inside the same awake window is a retryable delivery
  failure, not a terminal one
- At most one open water intent per plant; a second request returns 409 naming
  the pending intent
- Expiry sweep on the liveness timer: `intent_expires_at` defaults to
  `2 × wake_interval_seconds`, floor 30 minutes
- Restart reconciliation extended to intents (M6-012)
- Metrics: `command_intents_pending` gauge, `command_intents_expired_total`

## Non-goals

- Any MQTT change. No new topic, no retained command, no broker-side queue —
  delivery happens while the device is connected and is an ordinary publish.
- A queue of intents. One open water intent per plant, by design.
- Intents for `tare` and `calibrate`. They are operator diagnostics with no
  safety weight and no urgency; a battery device runs them at the next wake the
  operator is watching, or not at all.
- API and UI presentation of the pending state (M6-023, M12-018).

## Dependencies

- M6-008
- M6-009
- M6-011
- M6-016
- M4-013 (including its dated battery-compatibility report correction)

## Implementation notes

**An intent is not a command, and the distinction is the safety argument.** No
`command_id` exists until delivery, so nothing in SAFETY-001 or SAFETY-010
changes: there is still exactly one persist-before-publish moment per command,
and a delivery retry still reuses the `command_id` allocated at that moment. A
reviewer's test for whether this was implemented correctly is that
`command_intents` has a nullable `command_id` and `commands` has no new column.

**The gate must genuinely re-run at delivery, against current inputs.** This is
the part most likely to be implemented as a cheap "still allowed?" check, and it
must not be. A leak, an empty tank, an exhausted rolling window, or a stale
required measurement that appeared while the device slept all have to refuse the
dose (SAFETY-003, -004, -005, -006, -012). Running the full gate at delivery
makes this path *stricter* than the immediate path, which runs it once, and that
is a property worth stating in the code.

Two clocks, deliberately separated. The **intent** expires on the edge's clock
and is an operator-facing convenience. The **command** expires on the wire under
the unchanged 120-second TTL that the device validates against its own
synchronised clock (SAFETY-002). They are not the same mechanism and must not be
merged into one field; `intent_expires_at` never reaches a device.

The refusal-retry rule at wake needs care. A `clock_unsynced` refusal means the
device is awake but has not yet applied `edge.time`; retrying within the same
awake window is correct and terminates, because the window itself is bounded.
Every other refusal reason is terminal for the intent.

## Acceptance criteria

- [x] A `POST /water` to a sleeping device publishes **nothing** and creates one
      `pending_for_device_wake` intent, verified by a spy subscriber.
- [x] A `POST /water` to a connected device is byte-identical to M6-016's
      behaviour, with no intent row created.
- [x] At wake exactly one `command.water` is published, with one `command_id`
      allocated at that moment.
- [x] The command's `issued_at` is the wake instant, not the request instant.
- [x] `edge.time` is published before the command on every wake delivery.
- [x] A leak raised while the device slept refuses the intent at delivery with
      nothing published; likewise tank, staleness, and rolling-cap exhaustion.
- [x] An edge restart between request and wake still delivers exactly once.
- [x] A second `POST /water` while one intent is pending returns 409.
- [x] An intent past `intent_expires_at` becomes `expired_before_wake` and is
      never delivered.
- [x] `commands` gained no new column.

## Verification

```bash
cargo test -p edge-controller intents::
cargo test safety_001 safety_002 safety_010
cargo test --test integration pending_intent_survives_edge_restart
cargo test --test integration leak_during_sleep_refuses_at_delivery
```

## Tests required

- Routing by device connectivity.
- Full gate re-run at delivery for each refusal reason.
- Restart reconciliation of a pending intent.
- Single-open-intent enforcement.
- Expiry sweep.
- `clock_unsynced` retry inside one awake window, bounded.
- SCEN-113, SCEN-114, SCEN-116.

## Documentation impact

- [PRD 060](../../prd/060-irrigation-control-and-safety.md) — intent lifecycle.
- [ADR-004](../../adr/004-sqlite-edge-persistence-model.md) — the new table.
- [failure-model.md](../../architecture/failure-model.md) §3.

## Files likely affected

```text
crates/edge-controller/src/control/intents.rs
crates/edge-controller/src/control/commands.rs
crates/storage/src/intents.rs
migrations/
```
