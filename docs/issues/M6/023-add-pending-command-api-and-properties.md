# Issue M6-023 — Expose pending-command state and add its safety properties

**Milestone:** M6 · **PRD:** [PRD 060](../../prd/060-irrigation-control-and-safety.md) · **Depends on:** M6-022, M6-018

## Context

M6-022 holds a dose for a sleeping device. A caller that cannot tell "held" from
"sent" will read the 202 as "the pump is about to run", and when nothing happens
for fifteen minutes will press the button again — which is the behaviour the
single-open-intent rule exists to survive, but not the behaviour the API should
invite.

This issue makes the distinction explicit at the boundary, and turns M6-022's
claims into properties rather than examples.

## Goal

`pending_for_device_wake` is a first-class, observable state, and the delivery
path's safety claims hold under property testing.

## Scope

- `POST /plants/{id}/water` returns 202 with `status: "pending_for_device_wake"`,
  `intent_id`, `expected_delivery_after`, `intent_expires_at`, and **no**
  `command_id`, when the device is sleeping
- `GET /api/v1/commands/{command_id}` unchanged; `GET /api/v1/intents/{intent_id}`
  added for the pre-delivery window, and the intent response carries the
  `command_id` once one exists so a caller can follow the handover
- Plant and device responses expose the pending intent, so the UI does not have
  to poll a separate endpoint to know a dose is waiting
- 409 on a second request names the pending `intent_id` and its
  `expected_delivery_after`
- Property tests over the intent lifecycle
- The M8 mutation set extended: an implementation that publishes immediately to a
  sleeping device must turn the suite red

## Non-goals

- UI rendering (M12-018).
- Any override, force, or expedite parameter. There is no way to ask the edge to
  wake a device, because there is no mechanism by which it could.
- Cancelling an intent from the API. Deliberately deferred: a cancel that races
  a wake is a distributed-consensus problem for a feature nobody has asked for,
  and `intent_expires_at` already bounds the exposure. Recorded as an open
  question in PRD 060.

## Dependencies

- M6-022
- M6-018

## Implementation notes

The response shapes must make the difference visible at a glance, because the
difference is the whole point:

```json
{ "command_id": "018fd7b1-…", "status": "issued",
  "expires_at": "2026-08-28T11:32:00Z" }

{ "intent_id": "018fd7c9-…", "status": "pending_for_device_wake",
  "expected_delivery_after": "2026-08-28T11:45:00Z",
  "intent_expires_at": "2026-08-28T12:15:00Z" }
```

The absence of `command_id` in the second is load-bearing and worth a test of its
own. A client that reads `command_id` unconditionally should fail loudly rather
than poll a null id, which is why the field is absent rather than null.

Properties worth stating, over an arbitrary interleaving of requests, wakes,
restarts, refusals, and expiries:

```text
∀ intents:  delivered_commands(intent) ≤ 1
∀ intents:  intent reached `sent`  ⇒  exactly one command row exists
∀ plants:   open intents ≤ 1 at all times
∀ intents:  state is terminal ⇒ it never leaves that state
∀ deliveries: gate(inputs at delivery) = Allow
∀ deliveries: issued_at ≥ wake_at   (never the request instant)
```

The last two are the safety-relevant ones. The fifth says a dose is never
delivered on the strength of a gate result computed before the device slept; the
sixth says the TTL the device evaluates was minted while it was awake, which is
what keeps SAFETY-002 intact without changing SAFETY-002.

Reuse M6-018's `proptest` harness and `TestClock` rather than building a second
one; the interleaving generator is the only new part.

## Acceptance criteria

- [ ] A sleeping device's 202 carries `pending_for_device_wake` and **no**
      `command_id`.
- [ ] A connected device's 202 is unchanged from M6-016.
- [ ] `GET /intents/{id}` reports every lifecycle state, and carries the
      `command_id` once delivery has happened.
- [ ] The 409 body names the pending intent.
- [ ] No endpoint accepts an override, force, expedite, or wake parameter.
- [ ] All six properties pass at `PROPTEST_CASES=10000`.
- [ ] A mutation that publishes immediately to a sleeping device turns the suite
      red.
- [ ] `cargo test safety_` is fully green.

## Verification

```bash
cargo test -p edge-controller api::intents
PROPTEST_CASES=10000 cargo test intent_lifecycle_properties
cargo test safety_
grep -rn 'override\|force\|expedite' crates/edge-controller/src/api/ | grep -v '//'
```

## Tests required

- The six lifecycle properties.
- Response-shape tests for both paths, asserting `command_id` presence and
  absence.
- The added M8 mutation.
- SCEN-113, SCEN-116.

## Documentation impact

- [http-api-boundaries.md](../../protocol/http-api-boundaries.md) §2.6.
- [PRD 060](../../prd/060-irrigation-control-and-safety.md) — acceptance criteria
  and the cancel open question.

## Files likely affected

```text
crates/edge-controller/src/api/intents.rs
crates/edge-controller/src/api/watering.rs
crates/edge-controller/tests/intent_lifecycle_properties.rs
```
