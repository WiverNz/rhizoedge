# PRD 070 — Cloud Sync and Storage

**Milestone:** M7 · **Status:** PLANNED · **Depends on:** M6

## Summary

Add an optional Rust cloud API backed by PostgreSQL, plus the edge-side outbox
drain that ships history to it idempotently — without the cloud ever becoming a
dependency of local plant safety.

## Problem

Local SQLite is fine for one edge but cannot serve long-term history across
sites, survive the loss of the Pi, or be queried from elsewhere. The danger is
that adding a cloud quietly makes it load-bearing: one `await` on an HTTP call
inside a control path turns an Internet outage into a stalled watering decision.

## Goals

1. A cloud API accepting idempotent event batches.
2. A PostgreSQL schema partitioned by `edge_id` from day one.
3. An edge outbox drain that is fully decoupled from the control loop.
4. Retry with exponential backoff and full jitter; replay after an outage
   creating no duplicates.
5. Demonstrated proof that a cloud outage changes nothing locally.

## Non-goals

- Cloud-issued commands or configuration. The cloud has **no** write path toward
  devices ([ADR-003](../adr/003-edge-first-ownership-model.md)). This is the
  architecture, not a missing feature.
- Authentication. V1 runs the cloud locally in Docker; auth is M14.
- Multi-region, sharding, or any horizontal scaling concern.
- A cloud UI. The cloud exposes read APIs; visualising them is out of V1 scope.

## User/system flows

```text
any state change → outbox row written in the SAME transaction
        ↓
outbox_drain (independent task)
        ↓
POST /api/v1/edges/{edge_id}/events  (batch ≤ 500)
        ↓
cloud: one transaction, ON CONFLICT DO NOTHING per event
        ↓
per-event results → edge marks synced / quarantined / retries
```

Outage and recovery:

```text
cloud stops → POSTs fail → attempts++, next_attempt_at = now + full_jitter(backoff)
           → LOCAL OPERATION COMPLETELY UNAFFECTED
           → pending_cloud_events grows
cloud returns → batches drain → every event lands exactly once
```

## Functional requirements

### Cloud API

| ID | Requirement |
|---|---|
| F-070-01 | `POST /api/v1/edges/{edge_id}/events` accepts a batch of ≤ 500 events, ≤ 5 MiB |
| F-070-02 | Returns **HTTP 200 with per-event results** even on partial failure |
| F-070-03 | `duplicate` is a **success** result; the edge marks it synced |
| F-070-04 | A rejected event does not abort the batch or block the queue |
| F-070-05 | 4xx only for a malformed request envelope; 5xx for genuine server faults |
| F-070-06 | Idempotency enforced by a unique index on `(edge_id, event_id)` |
| F-070-07 | Each batch processed in one transaction with `ON CONFLICT DO NOTHING` |
| F-070-08 | An unknown event kind is **stored in the ledger** and reported `rejected` for projection only — history is never lost |
| F-070-09 | Projections are order-insensitive; a late status cannot overwrite a newer one |
| F-070-10 | Read APIs per [http-api-boundaries.md](../protocol/http-api-boundaries.md) §3.2 |
| F-070-11 | **No command endpoints, no config write endpoints, no endpoint the edge polls for instructions** |
| F-070-12 | `/health/live`, `/health/ready`, `/metrics` |
| F-070-13 | A `reproject` command rebuilds projections from `synced_events` |

### Edge outbox

| ID | Requirement |
|---|---|
| F-070-20 | Outbox rows written in the same transaction as the change they describe |
| F-070-21 | `event_id` generated once, at write time; **stable across every retry** |
| F-070-22 | The drain task never blocks the control loop; no shared lock, no awaited call |
| F-070-23 | Backoff: base 1 s, cap 300 s, full jitter, unlimited attempts |
| F-070-24 | 429 honoured with `Retry-After` where present |
| F-070-25 | Batch size halves on timeout, floor 10 |
| F-070-26 | `value_tier` assigned at the single write site, defaulting to `high` |
| F-070-27 | At `outbox_max_rows` (500 000), prune `low` tier oldest-first; **never prune `high`** |
| F-070-28 | Pruning emits an alert-level log and increments a counter |
| F-070-29 | Synced rows pruned after 24 h |
| F-070-30 | `cloud.enabled` defaults to **false**; with it false, no outbox rows are written and no task runs |

F-070-21 is the requirement the whole idempotency scheme rests on: an `event_id`
regenerated on retry would defeat the unique index entirely.

## Interfaces

**Cloud HTTP:** [http-api-boundaries.md](../protocol/http-api-boundaries.md) §3.

```rust
// rhizo-cloud-client
pub struct CloudClient { base: Url, http: reqwest::Client, edge_id: String }

impl CloudClient {
    pub async fn send_batch(&self, events: &[OutboxEvent])
        -> Result<Vec<EventResult>, CloudError>;
}

pub enum EventResult { Accepted, Duplicate, Rejected { error: String } }

pub enum CloudError {
    Transport(reqwest::Error),   // Transient
    Server { status: u16 },      // Transient
    BadRequest { status: u16 },  // Permanent — the whole batch envelope
    RateLimited { retry_after: Option<Duration> },
}
```

`classify(&CloudError) -> FailureKind` per
[ADR-014](../adr/014-failure-and-retry-policy.md), matching exhaustively.

## Data model

**Edge:** `pending_cloud_events` (created in M3, drained here).

**Cloud:** the two-layer schema from
[ADR-005](../adr/005-cloud-event-model-and-idempotency.md) — `synced_events` as
the append-only ledger and idempotency boundary, plus projections
(`edge_instances`, `devices`, `plants`, `measurements`, `watering_events`,
`device_events`), every one carrying `edge_id`.

Storing measurement data twice (JSONB in the ledger, columns in the projection)
is deliberate: it means a projection bug can be fixed and the tables rebuilt
without asking the edge to resend anything.

Migrations from `migrations/cloud/` via `sqlx`.

## State model

Outbox row lifecycle:

```text
      ┌─────────┐  2xx accepted/duplicate   ┌────────┐
      │ pending │──────────────────────────►│ synced │──24h──► pruned
      └────┬────┘                           └────────┘
           │ 5xx / timeout / DNS
           │ attempts++, next_attempt_at = now + full_jitter(backoff)
           └──────────────► pending (loop)
           │
           │ 4xx per-event rejection
           ▼
     ┌─────────────┐
     │ quarantined │  operator inspects; never retried automatically
     └─────────────┘
```

The cloud itself is stateless per request; all state is in PostgreSQL.

## Failure modes

Per [failure-model.md](../architecture/failure-model.md) §4:

| Failure | Behaviour |
|---|---|
| Cloud unreachable / DNS / timeout | Transient; backoff; **local operation untouched** |
| Cloud 5xx | Transient; same |
| Cloud 429 | Transient; honour `Retry-After` |
| Cloud per-event 4xx | that event quarantined; batch proceeds |
| Malformed batch envelope (4xx) | Permanent; logged loudly — this is an edge bug |
| Duplicate replay | cloud returns `duplicate`; edge marks synced |
| Prolonged outage | outbox grows to the cap, then value-tiered pruning |
| PostgreSQL down | cloud returns 5xx; edge treats as Transient |
| Projection bug | ledger unaffected; `reproject` rebuilds |
| Batch too large for a slow link | size halves on timeout, floor 10 |

## Safety implications

M7 enforces the two invariants that define the project's thesis:

**SAFETY-008 — cloud outage does not disable monitoring.** Enforced by the
outbox pattern: the control loop writes a row and moves on. There is no code
path in which a control decision awaits a network call. Also enforced by
F-070-30 (`cloud.enabled` defaults false) and by `/health/ready` excluding cloud
reachability — an edge with the cloud down is *ready*, and saying otherwise
would invite a pointless restart loop.

**SAFETY-009 — cloud outage does not bypass safety.** Enforced *structurally*:
`rhizo-domain` cannot depend on `rhizo-cloud-client`
([ADR-001](../adr/001-rust-workspace-and-crate-boundaries.md)), and
`IrrigationInputs` has no cloud-derived field. A decision function that wanted
cloud state would not compile.

The test is differential: `safety_009_decisions_identical_with_cloud_down` runs
the same seeded scenario twice, cloud up and cloud down, and asserts the issued
command sequences are identical modulo ids and timestamps.

Also safety-relevant: F-070-27's refusal to prune high-tier events. History is
nice; the ledger of what the machine did to a living plant is not optional.

## Observability

Metrics:

```text
pending_cloud_events                 gauge
cloud_sync_attempts_total{outcome}
cloud_sync_duration_seconds
cloud_events_quarantined_total
cloud_events_dropped_total
cloud_last_success_timestamp_seconds gauge
```

`pending_cloud_events` and `cloud_last_success_timestamp_seconds` are two of the
three most operationally valuable series in the system
([ADR-010](../adr/010-observability-strategy.md)).

Logging: the **first** failure of an outage at ERROR, subsequent retries at WARN
(so a week-long outage does not produce a week of ERROR), recovery at INFO with
the drained count.

## Testing strategy

- Unit: backoff bounds and jitter; `classify` per `CloudError` variant; batch
  halving; value-tier assignment; outbox state transitions.
- Integration (real PostgreSQL): idempotent insert; replay of an identical batch
  creating no rows; partial-failure result mapping; order-insensitive
  projections; `reproject` reproducing identical tables.
- Integration: SCEN-063 (one rejected event does not block), SCEN-064 (5xx then
  recovery), SCEN-065 (cap and value-tiered pruning).
- **SCEN-060 / SCEN-061 / SCEN-062** — the milestone-defining tests.
- Round-trip: MQTT integer-ms ↔ RFC 3339 ↔ `TIMESTAMPTZ` preserves the instant
  (the two-representation seam from
  [ADR-013](../adr/013-clock-and-time-semantics.md)).

## Acceptance criteria

- [ ] With the cloud stopped for an entire scenario, ingestion, storage,
      recommendations, automatic watering, the REST API, and metrics all work.
- [ ] `/health/ready` returns **200** with the cloud stopped.
- [ ] `pending_cloud_events` grows during the outage and returns to 0 after
      recovery.
- [ ] Every event reaches PostgreSQL exactly once, verified by
      `SELECT COUNT(*) FROM synced_events` against the edge's emitted count.
- [ ] Re-POSTing an identical batch returns all `duplicate` and creates no rows.
- [ ] `safety_009_decisions_identical_with_cloud_down` passes.
- [ ] A deliberately rejected event is quarantined while the other 499 sync.
- [ ] Filling the outbox past the cap prunes measurements and preserves **every**
      watering event, command, and lockout.
- [ ] `cloud-api reproject --edge-id home-01` reproduces byte-identical
      projection tables.
- [ ] `rhizo-domain`'s `Cargo.toml` contains no dependency on
      `rhizo-cloud-client`.

## Dependencies

- M6 (there must be watering events worth syncing).
- M0 (backoff utility, telemetry, compose skeleton).

## Open questions

1. **`outbox_max_rows = 500 000`** is roughly weeks of a small deployment. It is
   configurable; the value matters only in a pathological outage.
2. **Whether the cloud should expose an aggregate/downsample endpoint** for long
   ranges. Deferred until there is a consumer; the read APIs return raw with a
   `resolution` parameter reserved.
3. **Ledger retention.** None in V1. If storage ever matters, the ledger can be
   pruned behind the projections — but only once `reproject` is no longer needed
   for that range.

## Future work

- Cloud authentication and per-edge tokens (M14).
- Cloud-pushed desired state (M14 — deliberately deferred, see
  [ADR-003](../adr/003-edge-first-ownership-model.md)).
- Multi-edge dashboards (post-V1).
- TLS between edge and cloud (M13).
