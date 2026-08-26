# ADR-003 — Edge-first ownership and edge/cloud consistency model

## Status

Accepted — 2026-08-25. Structural; enforced from M3 onward.

**Amended 2026-08-26 by [ADR-015](015-device-offline-autonomy.md).** This ADR
originally said the Edge is the only component capable of making irrigation
decisions. That is now true only while the device can reach the Edge. A device
explicitly provisioned with a validated offline policy may act alone when
isolated — see §Device isolation below. The edge/cloud relationship described
here is unchanged.

## Context

The project's governing requirement is that a plant stays safely monitored and
controllable when the Internet is gone. That requirement is easy to state and
easy to violate by accident: a single `await` on an HTTP call inside a control
path converts a cloud outage into a stalled pump decision.

We need to decide, explicitly and structurally, who owns what state and which
direction authority flows.

## Decision

### The edge is the source of truth. The cloud is an append-only replica.

| State | Owner | Cloud's role |
|---|---|---|
| Measurements | Edge SQLite | receives copies |
| Device registry, health | Edge | receives copies |
| Plant profiles, plant config | Edge | receives copies (read-only) |
| Irrigation state, lockouts | Edge | receives copies |
| Commands and their results | Edge | receives copies |
| Cross-site history, long-term analytics | Cloud | owns |

The cloud never originates a command, never pushes configuration, and is never
consulted during a decision. In V1 the only cloud→edge traffic is an HTTP
response code acknowledging an event batch.

### Enforced structurally, not by discipline

Three mechanisms make "cloud cannot affect safety" a property of the code rather
than a rule people remember:

1. **`rhizo-domain` cannot depend on `rhizo-cloud-client`.** The dependency graph
   in [ADR-001](001-rust-workspace-and-crate-boundaries.md) forbids it. A
   decision function that wanted cloud state could not compile.
2. **`IrrigationInputs` has no cloud-derived field.** The struct is the complete
   set of things a watering decision may consider. Adding a cloud field would be
   a visible, reviewable change to a type named in the safety invariants.
3. **The outbox pattern decouples timing.** Events are written to
   `pending_cloud_events` inside the transaction that produced them. A separate
   task drains that table. The control loop never awaits a network call, so
   cloud latency cannot enter a control path even accidentally.

This satisfies SAFETY-008 and SAFETY-009 by construction, and is why
`safety_009_decisions_identical_with_cloud_down` can be written as a
straightforward differential test.

### Consistency model

**Eventual consistency, edge-authoritative, no conflict resolution needed.**

Because the cloud is append-only and the edge is the only writer for a given
`edge_id`, there is no concurrent-write conflict to resolve. Each event carries
an `event_id` (UUIDv7) that is unique per edge instance; the cloud stores
`(edge_id, event_id)` with a unique constraint. Replay is a no-op.

There is deliberately **no** "cloud has newer state" case, because there is no
path by which the cloud could acquire newer state.

### Multiple edge instances

The cloud data model is partitioned by `edge_id` from day one, even though V1
has exactly one edge. Retrofitting a tenant key into a schema after it has
history is far more expensive than carrying an extra column now. See
[ADR-005](005-cloud-event-model-and-idempotency.md).

### Device isolation (added by ADR-015)

This ADR's original framing had one axis — cloud reachable or not. There are
three, defined in
[connectivity-modes.md](../architecture/connectivity-modes.md):

| Mode | Meaning | Who decides irrigation |
|---|---|---|
| A — cloud offline | cloud unreachable, LAN fine | **Edge**, unchanged |
| B — site offline | no internet, LAN fine | **Edge**, unchanged — devices take wall time from the Edge over MQTT, so no internet is needed for clocks |
| C — device isolated | device cannot reach the Edge | **Device**, from a persisted validated policy, or not at all |

The ownership statement below therefore becomes:

```text
Edge   = source of truth, and the primary controller whenever reachable
Device = final hardware safety boundary, ALWAYS
       + restricted fallback controller when isolated AND explicitly provisioned
Cloud  = append-only replica; never a controller in any mode
```

The cloud's role is untouched by this amendment. It remains incapable of
originating a command in every mode, which is what SAFETY-008 and SAFETY-009
protect.

### What "offline" is allowed to degrade

| Capability | Cloud down | Broker down | Device down |
|---|---|---|---|
| Telemetry ingestion | ✅ works | ❌ stops | ❌ stops for that device |
| Local storage | ✅ works | ✅ works | ✅ works |
| Recommendations | ✅ works | ⚠️ data ages out | ⚠️ ages out |
| Automatic watering | ✅ works | ⛔ locks out (stale) | ⛔ locks out |
| Manual watering | ✅ works | ❌ cannot deliver | ❌ cannot deliver |
| Local REST API | ✅ works | ✅ works | ✅ works |
| Long-term history view | ⚠️ local only | ✅ works | ✅ works |

The only capability the cloud outage removes is the cross-site historical view.
Every degradation caused by broker or device loss is a *lockout*, never a
silently continuing automation.

## Alternatives considered

**Cloud-first with an edge cache.** Rejected. It inverts the failure mode: the
common case (home Internet flapping) becomes the dangerous case. It also makes
the cloud a hard dependency for a device that is three metres away from the
controller, which is absurd on the merits.

**Bidirectional sync with conflict resolution (CRDTs, vector clocks).** Rejected
as unjustified complexity. There is one writer per partition. Introducing merge
semantics would add a class of bug — a merge that resurrects a cleared lockout —
in exchange for a capability nothing needs.

**Cloud-pushed desired state (config from cloud).** Deferred, not rejected. It
is a genuinely useful feature for a multi-site deployment. It is excluded from
V1 because it introduces split-brain between local edits and cloud desired
state, and because it creates a network path that can alter device behaviour —
which needs an authentication story that V1 does not have
([ADR-011](011-configuration-and-secrets-model.md) §5). Revisit in M14.

**Edge as a thin MQTT-to-HTTP bridge.** Rejected: it is the cloud-first design
wearing a different hat.

## Consequences

Positive:

- The core requirement is satisfied structurally; there is no code path to audit
  for accidental cloud dependence, because the type system forbids it.
- Cloud development can lag freely. M0–M6 deliver a fully useful system with no
  cloud at all, and `cloud.enabled` defaults to `false`.
- Testing is simple: "run the scenario with the cloud container stopped" is a
  valid and cheap test.

Negative, accepted:

- The edge must implement durable state, idempotency, and crash recovery itself —
  work that a cloud-first design would push to a managed database. This is the
  substance of M3 and M6 and is a deliberate cost.
- Two schemas (SQLite and PostgreSQL) must be kept semantically aligned.
  Mitigated by generating both from the same conceptual model in the PRDs and by
  a round-trip test (M7-011).
- No cross-edge coordination is possible in V1. Acceptable: V1 has one edge.

## Risks

- **Accidental cloud coupling via a shared type.** Someone adds a
  `cloud_status` field to a struct that reaches `IrrigationInputs`.
  *Mitigation:* `IrrigationInputs` is constructed in exactly one function, which
  is reviewed as safety-critical, and the differential test M7-010 would fail.
- **Outbox unbounded growth during a long outage.** *Mitigation:* the cap and
  value-tiered pruning specified in
  [failure-model.md](../architecture/failure-model.md) §4.5. Issue M7-008.
- **Operators assuming cloud data is authoritative** and editing there.
  *Mitigation:* the cloud exposes no write API for configuration in V1 — there is
  nothing to edit.

## Follow-up

- [ADR-005](005-cloud-event-model-and-idempotency.md) — event schema and idempotency.
- [ADR-014](014-failure-and-retry-policy.md) — outbox backoff parameters.
- [PRD 070](../prd/070-cloud-sync-and-storage.md) — cloud sync requirements.
- M7-010 implements the differential cloud-up/cloud-down test.
