//! Reconciling a reconnecting device's buffered history (M6-020, SAFETY-016).
//!
//! # The hold is the safety-critical half
//!
//! A device that autonomously watered ninety seconds before reconnecting has
//! that dose **in its buffer**, not yet in the edge's budget. Issuing on top of
//! it is exactly the failure this rule prevents, and the only defence is
//! refusing to act until the buffer has been read. So a reconnecting device's
//! plants are held: [`is_reconciling`] answers `true` while the edge has not
//! committed a contiguous prefix through the device's final replay batch, the
//! gate sees `reconciling: true`, and it returns `Uncertain` — which already
//! means "inputs are not trustworthy enough to act", precisely the situation.
//!
//! `Uncertain` is reused rather than a new state invented, and it auto-clears
//! the moment the replay is committed.
//!
//! The hold is **derived from `replay_progress`**, not stored in
//! `devices.connectivity_mode`. `device.status` rewrites that column on every
//! heartbeat, and a device replays *while* it is heartbeating — so a hold kept
//! there could be cleared by an unrelated status message, which is not a hold.
//!
//! # Exactly once
//!
//! Replay is idempotent on `event_id` through M3's `persist_replay`, so a device
//! that reconnects, disconnects, and reconnects mid-replay creates no
//! duplicates. Nothing here re-implements that; this module owns the *hold* and
//! the *release*.
//!
//! # Attribution: the device names its own subject
//!
//! A `watering.offline_autonomous` event carries `detail.plant_id` — the plant
//! whose `OfflinePolicy` the device evaluated at the moment the water went into
//! the pot. `persist_replay` writes that name onto the `watering_events` row in
//! the same transaction as the event, so the charge is fixed when the history
//! is committed and nothing here can re-decide it.
//!
//! This is not a convenience. The alternative — resolving the plant from
//! `actuator_bindings` **at replay time** — asks a question about the present
//! and applies the answer to the past. Bindings are editable, and an isolated
//! device is precisely the case where an operator has time to edit them: move
//! the pump from plant A to plant B while A is alone, and A's autonomous dose
//! lands in B's budget. That is wrong in both directions at once. A, which
//! really was watered, keeps a clean budget and may be watered again
//! immediately — SAFETY-006 defeated in the over-watering direction — while B is
//! charged for water it never received and may be refused a dose it needs.
//!
//! # The fallback, and when it is ambiguous
//!
//! `plant_id` is optional on the wire, because a v1 device built before the
//! field exists is still a conformant v1 device. Its doses arrive with a `NULL`
//! plant, and only then does [`attribute_autonomous_doses`] resolve them from
//! the actuator bindings. One actuator-bound plant is the ordinary case. If a
//! device carries the actuator for **several** plants, the dose is charged to
//! every one of them: over-counting reduces future doses, under-counting would
//! permit an extra one, and the conservative direction is the safe one — the
//! same choice PRD 060 §Open questions 2 makes for an interrupted dose. The
//! ambiguity is recorded as a warning event rather than hidden.
//!
//! A named plant this edge has never provisioned takes the same fallback:
//! `watering_events.plant_id` is a foreign key, and letting an unknown name
//! abort the replay transaction would wedge reconciliation for ever.

use chrono::{DateTime, Utc};
use rhizo_storage::EdgeDb;
use rhizo_storage::repo::plant as plant_repo;

use crate::error::EdgeError;

/// Records that a device has begun replaying buffered history.
///
/// The **hold itself is derived**, not stored: see [`is_reconciling`]. This
/// records the observation for the operator and does not write the connectivity
/// column, because `device.status` overwrites that column on every heartbeat and
/// a device replays *while* it is heartbeating. A hold that a routine status
/// message could clear would not be a hold at all.
pub async fn begin(db: &EdgeDb, device_id: &str, now: DateTime<Utc>) -> Result<bool, EdgeError> {
    let recorded = record_device_event(
        db,
        device_id,
        "reconciling",
        "info",
        None,
        now.timestamp_millis(),
    )
    .await?;
    if recorded {
        tracing::info!(
            device_id = %device_id,
            "device reconnected with buffered history; its plants are held until the replay is committed"
        );
    }
    Ok(recorded)
}

/// Whether a device is still replaying buffered history for its current boot.
///
/// Derived from `replay_progress` rather than from a flag, so it survives a
/// restart, cannot be cleared by an unrelated status message, and answers `true`
/// exactly while the edge has *not* committed a contiguous prefix through the
/// device's final batch.
///
/// `complete` alone is the sender's framing and is not proof of a committed
/// prefix; a batch that arrives complete but leaves a **hole** keeps the plant
/// held, which is the conservative reading and the one SAFETY-016 needs.
///
/// An *empty* complete replay is a different thing and releases the plant: a
/// device that was never isolated buffers nothing, says so, and has nothing to
/// reconcile. Conflating "the device had nothing" with "the edge is missing the
/// beginning" would hold every ordinary reconnection for ever.
pub async fn is_reconciling(db: &EdgeDb, device_id: &str) -> Result<bool, EdgeError> {
    use sqlx::Row as _;
    let Some(row) = sqlx::query(
        "SELECT p.complete AS complete, p.through_device_seq AS through,                 (SELECT count(*) FROM device_events e                   WHERE e.device_id=p.device_id AND e.boot_id=p.boot_id                     AND e.origin='offline_replay') AS committed          FROM replay_progress p JOIN devices d ON d.device_id=p.device_id          WHERE p.device_id=? AND d.boot_id IS NOT NULL AND p.boot_id=d.boot_id",
    )
    .bind(device_id)
    .fetch_optional(db.pool())
    .await
    .map_err(|e| EdgeError::Storage(rhizo_storage::StorageError::Database(e.to_string())))?
    else {
        // No replay for this boot: there is nothing to reconcile.
        return Ok(false);
    };
    // More batches are expected.
    if row.get::<i64, _>("complete") == 0 {
        return Ok(true);
    }
    // The sender says it is done. The edge is satisfied when it holds a
    // contiguous prefix — or when the device turned out to have nothing at all,
    // which is the ordinary case for a device that was never isolated. A
    // *suffix* with no prefix is the one that keeps the plant held: the edge is
    // holding history it cannot vouch for the start of.
    if row.get::<Option<i64>, _>("through").is_some() {
        return Ok(false);
    }
    Ok(row.get::<i64, _>("committed") > 0)
}

/// Releases a device once its replay has been **committed**, not merely sent.
///
/// `complete` on the wire is the sender's framing. The edge releases only when
/// it holds a contiguous committed prefix through the device's final sequence,
/// which is what `persist_replay` reports.
pub async fn complete(
    db: &EdgeDb,
    device_id: &str,
    boot_id: &str,
    now: DateTime<Utc>,
) -> Result<Summary, EdgeError> {
    let summary = summarise(db, device_id, boot_id).await?;
    attribute_autonomous_doses(db, device_id, now).await?;
    // Nothing needs clearing: the hold is derived from `replay_progress`, which
    // the commit that produced this call has already advanced.
    record_device_event(
        db,
        device_id,
        "device.reconciled",
        "info",
        Some(&serde_json::json!({
            "boot_id": boot_id,
            "events": summary.events,
            "autonomous_doses": summary.autonomous_doses,
            "delivered_ml": summary.delivered_ml,
            "gaps": summary.gaps,
        })),
        now.timestamp_millis(),
    )
    .await?;
    tracing::info!(
        device_id = %device_id,
        boot_id = %boot_id,
        events = summary.events,
        autonomous_doses = summary.autonomous_doses,
        delivered_ml = summary.delivered_ml,
        gaps = summary.gaps,
        "reconciliation complete; the plant is released"
    );
    Ok(summary)
}

/// What happened while the device was isolated.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Summary {
    /// Replayed events committed for this boot.
    pub events: i64,
    /// Autonomous doses among them.
    pub autonomous_doses: i64,
    /// Volume those doses delivered.
    pub delivered_ml: f64,
    /// Gap markers reported.
    pub gaps: i64,
}

async fn summarise(db: &EdgeDb, device_id: &str, boot_id: &str) -> Result<Summary, EdgeError> {
    use sqlx::Row as _;
    let row = sqlx::query(
        "SELECT count(*) AS events, sum(CASE WHEN kind='watering.offline_autonomous' THEN 1 ELSE 0 END) AS doses \
         FROM device_events WHERE device_id=? AND boot_id=? AND origin='offline_replay'",
    )
    .bind(device_id)
    .bind(boot_id)
    .fetch_one(db.pool())
    .await
    .map_err(|e| EdgeError::Storage(rhizo_storage::StorageError::Database(e.to_string())))?;
    let gaps: i64 =
        sqlx::query_scalar("SELECT count(*) FROM history_gaps WHERE device_id=? AND boot_id=?")
            .bind(device_id)
            .bind(boot_id)
            .fetch_one(db.pool())
            .await
            .map_err(|e| {
                EdgeError::Storage(rhizo_storage::StorageError::Database(e.to_string()))
            })?;
    let delivered: Option<f64> = sqlx::query_scalar(
        "SELECT sum(delivered_ml) FROM watering_events WHERE device_id=? AND origin='offline_autonomous'",
    )
    .bind(device_id)
    .fetch_one(db.pool())
    .await
    .map_err(|e| EdgeError::Storage(rhizo_storage::StorageError::Database(e.to_string())))?;
    Ok(Summary {
        events: row.get::<i64, _>("events"),
        autonomous_doses: row.get::<Option<i64>, _>("doses").unwrap_or(0),
        delivered_ml: delivered.unwrap_or(0.0),
        gaps,
    })
}

/// Charges *unattributed* replayed doses to the plants the device actuates.
///
/// This is the **fallback**, not the primary path. A dose whose event named its
/// plant was already charged, by name, inside the transaction that committed the
/// event; those rows have a non-`NULL` `plant_id` and the query below does not
/// see them. What reaches here is a dose from a device that predates
/// `detail.plant_id`, or one naming a plant this edge does not know.
///
/// Idempotent: the per-plant row id is derived from the event id, so replaying
/// the same event any number of times produces one row per plant.
async fn attribute_autonomous_doses(
    db: &EdgeDb,
    device_id: &str,
    now: DateTime<Utc>,
) -> Result<usize, EdgeError> {
    use sqlx::Row as _;
    let plants: Vec<String> =
        sqlx::query_scalar("SELECT plant_id FROM actuator_bindings WHERE device_id=?")
            .bind(device_id)
            .fetch_all(db.pool())
            .await
            .map_err(|e| {
                EdgeError::Storage(rhizo_storage::StorageError::Database(e.to_string()))
            })?;
    if plants.is_empty() {
        // A device that autonomously watered a plant nothing is bound to is a
        // configuration the edge cannot interpret. The event is already stored;
        // charging a budget nobody owns would be worse than saying so.
        return Ok(0);
    }
    if plants.len() > 1 {
        tracing::warn!(
            device_id = %device_id,
            plants = plants.len(),
            "an autonomous dose cannot be attributed to one plant; charging every actuator-bound plant conservatively"
        );
    }
    let unattributed = sqlx::query(
        "SELECT watering_event_id,started_at,completed_at,delivered_ml FROM watering_events \
         WHERE device_id=? AND origin='offline_autonomous' AND plant_id IS NULL",
    )
    .bind(device_id)
    .fetch_all(db.pool())
    .await
    .map_err(|e| EdgeError::Storage(rhizo_storage::StorageError::Database(e.to_string())))?;

    if !unattributed.is_empty() {
        tracing::warn!(
            device_id = %device_id,
            doses = unattributed.len(),
            "replayed autonomous doses arrived without a plant; attributing them from the \
             actuator bindings that exist now, which may not be the bindings that were in \
             force while the device was isolated"
        );
    }
    let mut written = 0;
    for row in &unattributed {
        let source: String = row.get("watering_event_id");
        let started_at: i64 = row.get("started_at");
        let completed_at: Option<i64> = row.get("completed_at");
        let delivered_ml: Option<f64> = row.get("delivered_ml");
        // One transaction per replayed dose: the per-plant rows, the marker
        // that retires the unattributed row, and the cloud events that describe
        // them commit together or not at all. `persist_replay` deliberately did
        // not emit for this dose — it had no plant to name — so this is the only
        // place its `watering.offline_autonomous` can truthfully be written.
        let mut tx = db.begin().await.map_err(EdgeError::Storage)?;
        for plant_id in &plants {
            let id = format!("{source}:{plant_id}");
            let done = sqlx::query(
                "INSERT INTO watering_events(watering_event_id,plant_id,device_id,command_id,mode,origin,started_at,completed_at,requested_ml,delivered_ml,status) \
                 VALUES(?,?,?,NULL,'automatic','offline_autonomous',?,?,NULL,?,'completed') ON CONFLICT(watering_event_id) DO NOTHING",
            )
            .bind(&id)
            .bind(plant_id)
            .bind(device_id)
            .bind(started_at)
            .bind(completed_at)
            .bind(delivered_ml)
            .execute(&mut *tx)
            .await
            .map_err(|e| EdgeError::Storage(rhizo_storage::StorageError::Database(e.to_string())))?;
            if done.rows_affected() == 1 {
                written += 1;
                rhizo_storage::repo::outbox::emit(
                    &mut tx,
                    rhizo_storage::repo::outbox::EventKind::WATERING_OFFLINE_AUTONOMOUS,
                    &serde_json::json!({"device_id":device_id,"plant_id":plant_id,"mode":"automatic","origin":"offline_autonomous","delivered_ml":delivered_ml,"status":"completed","attribution":"actuator_binding_fallback","source_watering_event_id":source,"occurred_at":completed_at.unwrap_or(started_at)}),
                    now.timestamp_millis(),
                )
                .await
                .map_err(EdgeError::Storage)?;
            }
        }
        // The unattributed row stays as the device-level record of what the
        // hardware did; the per-plant rows are what the budget sums. Marking it
        // consumed keeps the attribution idempotent without deleting history.
        sqlx::query("UPDATE watering_events SET status='attributed' WHERE watering_event_id=?")
            .bind(&source)
            .execute(&mut *tx)
            .await
            .map_err(|e| {
                EdgeError::Storage(rhizo_storage::StorageError::Database(e.to_string()))
            })?;
        tx.commit().await.map_err(|e| {
            EdgeError::Storage(rhizo_storage::StorageError::Database(e.to_string()))
        })?;
    }
    Ok(written)
}

/// Whether any device this plant depends on is still reconciling.
pub async fn plant_is_held(db: &EdgeDb, plant_id: &str, now_ms: i64) -> Result<bool, EdgeError> {
    let devices: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT device_id FROM sensor_bindings WHERE plant_id=? \
         UNION SELECT device_id FROM actuator_bindings WHERE plant_id=?",
    )
    .bind(plant_id)
    .bind(plant_id)
    .fetch_all(db.pool())
    .await
    .map_err(|e| EdgeError::Storage(rhizo_storage::StorageError::Database(e.to_string())))?;
    let _ = now_ms;
    for device in devices {
        if is_reconciling(db, &device).await? {
            return Ok(true);
        }
    }
    Ok(false)
}

async fn record_device_event(
    db: &EdgeDb,
    device_id: &str,
    kind: &str,
    severity: &str,
    detail: Option<&serde_json::Value>,
    at: i64,
) -> Result<bool, EdgeError> {
    let done = sqlx::query(
        "INSERT INTO device_events(event_id,device_id,kind,severity,detail_json,occurred_at,received_at,origin) \
         VALUES(?,?,?,?,?,?,?,'edge') ON CONFLICT(event_id) DO NOTHING",
    )
    .bind(format!("edge:{device_id}:{kind}:{at}"))
    .bind(device_id)
    .bind(kind)
    .bind(severity)
    .bind(detail.map(ToString::to_string))
    .bind(at)
    .bind(at)
    .execute(db.pool())
    .await
    .map_err(|e| EdgeError::Storage(rhizo_storage::StorageError::Database(e.to_string())))?;
    Ok(done.rows_affected() == 1)
}

/// The plants an unreleased device holds, for the API and for logging.
pub async fn held_plants(db: &EdgeDb, device_id: &str) -> Result<Vec<String>, EdgeError> {
    let mut plants: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT plant_id FROM sensor_bindings WHERE device_id=? \
         UNION SELECT plant_id FROM actuator_bindings WHERE device_id=?",
    )
    .bind(device_id)
    .bind(device_id)
    .fetch_all(db.pool())
    .await
    .map_err(|e| EdgeError::Storage(rhizo_storage::StorageError::Database(e.to_string())))?;
    plants.sort();
    plants.dedup();
    Ok(plants)
}

/// Re-exports the plant repository so callers name one module for events.
pub use plant_repo::record_plant_event;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
#[allow(
    clippy::module_inception,
    reason = "the module name is the verification filter the issue quotes literally"
)]
mod reconcile {
    use super::*;
    use crate::api::testsupport::TestApi;
    use rhizo_domain::irrigation::types::IrrigationDecision;
    use rhizo_domain::state::LockoutReason;

    /// The two boot identities the tests use. `boot_id` is a UUID on the wire,
    /// so the strings a reader would reach for are not valid here.
    fn boot_a() -> uuid::Uuid {
        uuid::Uuid::from_u128(0xa)
    }

    fn boot_b() -> uuid::Uuid {
        uuid::Uuid::from_u128(0xb)
    }

    /// A replay batch as a device that predates `detail.plant_id` publishes it.
    ///
    /// Kept as the *unnamed* form on purpose: it is the fallback path, and every
    /// test that was written against binding-based attribution still exercises
    /// exactly that.
    fn replay(
        boot: uuid::Uuid,
        seqs: &[u64],
        complete: bool,
        delivered_ml: f32,
    ) -> serde_json::Value {
        replay_named(boot, seqs, complete, delivered_ml, None)
    }

    /// A replay batch as a device would publish it, with one autonomous dose.
    ///
    /// `plant` is what the device says the dose was for. `None` reproduces a v1
    /// device built before the field existed.
    fn replay_named(
        boot: uuid::Uuid,
        seqs: &[u64],
        complete: bool,
        delivered_ml: f32,
        plant: Option<&str>,
    ) -> serde_json::Value {
        serde_json::json!({
            "v": 1,
            "kind": "device.events",
            "message_id": uuid::Uuid::now_v7(),
            "device_id": "plant-node-01",
            "boot_id": boot.to_string(),
            "data": {
                "replay": true,
                "complete": complete,
                "events": seqs.iter().map(|seq| {
                    let mut detail = serde_json::json!({
                        "detail_type": "watering",
                        "policy_version": 7,
                        "delivered_ml": delivered_ml,
                        "trigger_value": 20.0,
                        "duration_ms": 4_000,
                    });
                    if let Some(plant) = plant {
                        detail["plant_id"] = serde_json::json!(plant);
                    }
                    serde_json::json!({
                        "event_id": uuid::Uuid::from_u128(u128::from(*seq) + 1),
                        "device_seq": seq,
                        "tier": "audit",
                        "kind": "watering.offline_autonomous",
                        "monotonic_ms": 1_000,
                        "detail": detail,
                    })
                }).collect::<Vec<_>>(),
            },
        })
    }

    /// Moves the pump from one plant to another, as an operator would while the
    /// device was unreachable.
    async fn rebind_actuator(api: &TestApi, from: &str, to: &str) {
        sqlx::query("DELETE FROM actuator_bindings WHERE plant_id=?")
            .bind(from)
            .execute(api.db.pool())
            .await
            .unwrap();
        api.bind_actuator(to).await;
    }

    async fn persist(api: &TestApi, batch: &serde_json::Value) {
        let envelope: rhizo_mqtt_contract::Envelope<
            rhizo_mqtt_contract::payload::DeviceEventBatch,
        > = rhizo_mqtt_contract::Envelope::from_json(&serde_json::to_vec(batch).unwrap()).unwrap();
        let now = api.clock.now().timestamp_millis();
        begin(&api.db, "plant-node-01", api.clock.now())
            .await
            .unwrap();
        let commit = rhizo_storage::repo::ingest::persist_replay(&api.db, &envelope, now)
            .await
            .unwrap();
        if commit.sender_reports_complete && commit.through_device_seq.is_some() {
            complete(
                &api.db,
                "plant-node-01",
                &boot_a().to_string(),
                api.clock.now(),
            )
            .await
            .unwrap();
        }
    }

    /// Sets the device's current boot so the hold is scoped to it.
    async fn boot(api: &TestApi, boot_id: uuid::Uuid) {
        sqlx::query("UPDATE devices SET boot_id=?,connectivity_mode='connected' WHERE device_id='plant-node-01'")
            .bind(boot_id.to_string())
            .execute(api.db.pool())
            .await
            .unwrap();
    }

    /// Replayed offline history has to *leave* the edge, not merely survive on
    /// it. Until the post-M7 correction none of it did: `persist_replay` wrote
    /// device events, gap markers, and autonomous doses without ever touching
    /// the outbox, so an isolated device's history was invisible in the cloud.
    async fn enable_cloud(api: &TestApi) {
        rhizo_storage::repo::outbox::configure(&api.db, true, 500_000)
            .await
            .unwrap();
    }

    async fn outbox_kinds(api: &TestApi) -> Vec<String> {
        sqlx::query_scalar("SELECT kind FROM pending_cloud_events ORDER BY created_at,event_id")
            .fetch_all(api.db.pool())
            .await
            .unwrap()
    }

    async fn outbox_payloads(api: &TestApi, kind: &str) -> Vec<serde_json::Value> {
        let rows: Vec<String> = sqlx::query_scalar(
            "SELECT payload_json FROM pending_cloud_events WHERE kind=? ORDER BY created_at,event_id",
        )
        .bind(kind)
        .fetch_all(api.db.pool())
        .await
        .unwrap();
        rows.iter()
            .map(|r| serde_json::from_str(r).unwrap())
            .collect()
    }

    /// A dose that named its own plant is emitted from the transaction that
    /// committed it, and a reconnect that replays the same `event_id` emits
    /// nothing further — the edge deduplicates on the device's id, and a second
    /// outbox row would carry a second `event_id` past the cloud's idempotency.
    #[tokio::test]
    async fn a_named_replayed_dose_reaches_the_cloud_outbox_exactly_once() {
        let api = TestApi::start().await;
        api.waterable("monstera-01").await;
        api.device_connected().await;
        boot(&api, boot_a()).await;
        enable_cloud(&api).await;

        let batch = replay_named(boot_a(), &[0], true, 30.0, Some("monstera-01"));
        persist(&api, &batch).await;
        let doses = outbox_payloads(&api, "watering.offline_autonomous").await;
        assert_eq!(doses.len(), 1);
        assert_eq!(doses[0]["plant_id"], "monstera-01");
        assert_eq!(doses[0]["origin"], "offline_autonomous");
        assert_eq!(doses[0]["status"], "completed");
        assert_eq!(doses[0]["delivered_ml"], 30.0);

        // The same events again, under a fresh transport message id.
        persist(&api, &batch).await;
        assert_eq!(
            outbox_payloads(&api, "watering.offline_autonomous")
                .await
                .len(),
            1
        );
    }

    /// A dose the device could not name is emitted where its plant is actually
    /// decided — the fallback attribution — and not before. Emitting it at
    /// replay time would have meant an event with no plant, which the cloud
    /// cannot project and would quarantine.
    #[tokio::test]
    async fn a_fallback_attributed_dose_is_emitted_by_reconciliation() {
        let api = TestApi::start().await;
        api.waterable("monstera-01").await;
        api.device_connected().await;
        boot(&api, boot_a()).await;
        enable_cloud(&api).await;

        persist(&api, &replay(boot_a(), &[0], true, 25.0)).await;
        let doses = outbox_payloads(&api, "watering.offline_autonomous").await;
        assert_eq!(doses.len(), 1, "the fallback path emits exactly once");
        assert_eq!(doses[0]["plant_id"], "monstera-01");
        assert_eq!(doses[0]["attribution"], "actuator_binding_fallback");

        // Reconciling again attributes nothing new, so it announces nothing new.
        complete(
            &api.db,
            "plant-node-01",
            &boot_a().to_string(),
            api.clock.now(),
        )
        .await
        .unwrap();
        assert_eq!(
            outbox_payloads(&api, "watering.offline_autonomous")
                .await
                .len(),
            1
        );
    }

    /// A gap marker and a policy activation are catalogue kinds of their own.
    /// Every other replayed device event is a `device.event` — a device-side
    /// lockout is not the plant lockout ADR-005's `lockout.set` describes.
    #[tokio::test]
    async fn replayed_gaps_and_policy_activations_carry_their_own_catalogue_kinds() {
        let api = TestApi::start().await;
        api.waterable("monstera-01").await;
        api.device_connected().await;
        boot(&api, boot_a()).await;
        enable_cloud(&api).await;

        let batch = serde_json::json!({
            "v": 1,
            "kind": "device.events",
            "message_id": uuid::Uuid::now_v7(),
            "device_id": "plant-node-01",
            "boot_id": boot_a().to_string(),
            "data": {
                "replay": true,
                "complete": true,
                "events": [
                    {
                        "event_id": uuid::Uuid::from_u128(0x51),
                        "device_seq": 0,
                        "tier": "audit",
                        "kind": "history.gap",
                        "monotonic_ms": 1_000,
                        "detail": {
                            "detail_type": "gap",
                            "from_seq": 1,
                            "to_seq": 4,
                            "lost_count": 4,
                            "lost_tier": "routine",
                        },
                    },
                    {
                        "event_id": uuid::Uuid::from_u128(0x52),
                        "device_seq": 1,
                        "tier": "audit",
                        "kind": "policy.activated",
                        "monotonic_ms": 2_000,
                        "detail": { "detail_type": "policy_activated", "policy_version": 7 },
                    },
                    {
                        "event_id": uuid::Uuid::from_u128(0x53),
                        "device_seq": 2,
                        "tier": "audit",
                        "kind": "lockout.set",
                        "monotonic_ms": 3_000,
                        "detail": { "detail_type": "lockout", "reason": "leak_detected" },
                    },
                ],
            },
        });
        persist(&api, &batch).await;

        let kinds = outbox_kinds(&api).await;
        assert_eq!(
            kinds.iter().filter(|k| *k == "history.gap").count(),
            1,
            "kinds were {kinds:?}"
        );
        assert_eq!(
            kinds
                .iter()
                .filter(|k| *k == "device.policy_applied")
                .count(),
            1,
            "kinds were {kinds:?}"
        );
        assert_eq!(
            kinds.iter().filter(|k| *k == "device.event").count(),
            1,
            "a device-side lockout stays a device.event; kinds were {kinds:?}"
        );
        assert!(
            !kinds.iter().any(|k| k == "lockout.set"),
            "a device lockout must not masquerade as a plant lockout"
        );
    }

    /// **SAFETY-016's hold.** While a replay is incomplete the plant is held in
    /// `Uncertain`, and the loop publishes **no command** — asserted against the
    /// transport, not against a status code.
    #[tokio::test]
    async fn safety_016_no_dose_is_issued_while_a_plant_is_reconciling() {
        let api = TestApi::start().await;
        api.waterable("monstera-01").await;
        api.device_connected().await;
        boot(&api, boot_a()).await;
        api.json(
            "POST",
            "/api/v1/plants/monstera-01/auto-watering/enable",
            serde_json::json!({}),
        )
        .await;

        // A partial replay: sequences 0 and 1, not marked complete.
        persist(&api, &replay(boot_a(), &[0, 1], false, 35.0)).await;
        assert!(is_reconciling(&api.db, "plant-node-01").await.unwrap());

        let pass = api.irrigate("monstera-01").await;
        assert_eq!(
            pass.decision,
            IrrigationDecision::Lock {
                reason: LockoutReason::Uncertain
            }
        );
        assert!(
            api.transport.commands().is_empty(),
            "no command may be published while a plant is reconciling"
        );

        // A manual request is refused for the same reason.
        let (status, body) = api
            .json(
                "POST",
                "/api/v1/plants/monstera-01/water",
                serde_json::json!({ "ml": 30.0 }),
            )
            .await;
        assert_eq!(status, axum::http::StatusCode::CONFLICT, "{body}");
        assert!(api.transport.commands().is_empty());
    }

    /// The plant is released only after the replay is **committed**, and the
    /// autonomous doses then appear in the rolling budget.
    #[tokio::test]
    async fn the_plant_is_released_once_the_replay_is_committed() {
        let api = TestApi::start().await;
        api.waterable("monstera-01").await;
        api.device_connected().await;
        boot(&api, boot_a()).await;

        persist(&api, &replay(boot_a(), &[0, 1], false, 35.0)).await;
        assert!(is_reconciling(&api.db, "plant-node-01").await.unwrap());

        persist(&api, &replay(boot_a(), &[2], true, 35.0)).await;
        assert!(!is_reconciling(&api.db, "plant-node-01").await.unwrap());

        // SAFETY-014: the autonomous doses land in the same rolling window as
        // commanded ones. There is one budget per plant, not one per path.
        let delivered =
            rhizo_storage::repo::command::delivered_in_window(&api.db, "monstera-01", 0)
                .await
                .unwrap();
        assert!(
            (delivered - 105.0).abs() < 1e-6,
            "three autonomous doses of 35 ml, got {delivered}"
        );
    }

    /// **SAFETY-016.** Replaying the same batch any number of times, in any
    /// order, produces one `watering_event` per `event_id` and one budget charge.
    #[tokio::test]
    async fn safety_016_replay_is_idempotent() {
        let api = TestApi::start().await;
        api.waterable("monstera-01").await;
        api.device_connected().await;
        boot(&api, boot_a()).await;

        let first = replay(boot_a(), &[0, 1], false, 35.0);
        let last = replay(boot_a(), &[2], true, 35.0);
        // Out of order, and three times over.
        for _ in 0..3 {
            persist(&api, &last).await;
            persist(&api, &first).await;
            persist(&api, &last).await;
        }

        let events: i64 =
            sqlx::query_scalar("SELECT count(*) FROM watering_events WHERE plant_id='monstera-01'")
                .fetch_one(api.db.pool())
                .await
                .unwrap();
        assert_eq!(events, 3, "one per distinct event_id, however many replays");
        let delivered =
            rhizo_storage::repo::command::delivered_in_window(&api.db, "monstera-01", 0)
                .await
                .unwrap();
        assert!((delivered - 105.0).abs() < 1e-6, "{delivered}");
    }

    /// A batch marked complete that leaves a hole does **not** release the
    /// plant: `complete` is the sender's framing, not proof of a committed
    /// prefix.
    #[tokio::test]
    async fn a_complete_batch_with_a_hole_does_not_release_the_plant() {
        let api = TestApi::start().await;
        api.waterable("monstera-01").await;
        api.device_connected().await;
        boot(&api, boot_a()).await;

        // Sequences 5 and 6 with nothing before them: a suffix, not a prefix.
        persist(&api, &replay(boot_a(), &[5, 6], true, 35.0)).await;
        assert!(
            is_reconciling(&api.db, "plant-node-01").await.unwrap(),
            "a hole keeps the plant held, which is the conservative reading"
        );
    }

    /// `device.reconciled` summarises what happened while the device was alone.
    #[tokio::test]
    async fn a_reconciled_device_records_a_summary() {
        let api = TestApi::start().await;
        api.waterable("monstera-01").await;
        api.device_connected().await;
        boot(&api, boot_a()).await;
        persist(&api, &replay(boot_a(), &[0], true, 35.0)).await;

        let detail: Option<String> = sqlx::query_scalar(
            "SELECT detail_json FROM device_events WHERE kind='device.reconciled' AND device_id='plant-node-01'",
        )
        .fetch_optional(api.db.pool())
        .await
        .unwrap();
        let detail: serde_json::Value = serde_json::from_str(&detail.expect("a summary")).unwrap();
        assert_eq!(detail["boot_id"], boot_a().to_string());
        assert_eq!(detail["autonomous_doses"], 1);
        assert_eq!(detail["delivered_ml"], 35.0);
    }

    /// The hold is scoped to the device's **current** boot: an unfinished replay
    /// from a previous run does not hold a plant for ever.
    #[tokio::test]
    async fn an_unfinished_replay_from_a_previous_boot_does_not_hold_the_plant() {
        let api = TestApi::start().await;
        api.waterable("monstera-01").await;
        api.device_connected().await;
        boot(&api, boot_a()).await;
        persist(&api, &replay(boot_a(), &[0, 1], false, 35.0)).await;
        assert!(is_reconciling(&api.db, "plant-node-01").await.unwrap());

        // The device reboots. Whatever it still holds, it will replay under its
        // new boot identity.
        boot(&api, boot_b()).await;
        assert!(!is_reconciling(&api.db, "plant-node-01").await.unwrap());
    }

    /// An edge restart mid-reconciliation replays safely: the hold is derived
    /// from committed rows, so re-reading them reaches the same answer.
    #[tokio::test]
    async fn an_edge_restart_mid_reconciliation_changes_nothing() {
        let api = TestApi::start().await;
        api.waterable("monstera-01").await;
        api.device_connected().await;
        boot(&api, boot_a()).await;
        persist(&api, &replay(boot_a(), &[0, 1], false, 35.0)).await;

        // "Restarting" is re-reading the same durable rows.
        assert!(is_reconciling(&api.db, "plant-node-01").await.unwrap());
        let recovery = api.commander.reconcile().await.unwrap();
        assert_eq!(recovery.republished, 0);
        assert!(is_reconciling(&api.db, "plant-node-01").await.unwrap());
        assert!(api.transport.published().is_empty());
    }

    /// **The misattribution regression.** A dose delivered to plant A while the
    /// device was isolated is charged to A even though the actuator has since
    /// been rebound to plant B.
    ///
    /// ```text
    /// plant A bound -> isolate -> A waters offline
    ///               -> binding moves to B -> replay
    ///               -> A charged exactly once, B unchanged
    /// ```
    ///
    /// Before `detail.plant_id`, the replay resolved ownership from
    /// `actuator_bindings` as they stood at replay time, so this charged B and
    /// left A with a clean budget — the plant that had just been watered was the
    /// one free to be watered again.
    #[tokio::test]
    async fn safety_016_a_replayed_dose_is_charged_to_the_plant_the_device_named() {
        let api = TestApi::start().await;
        api.waterable("monstera-01").await;
        api.plant("fern-01").await;
        api.device_connected().await;
        boot(&api, boot_a()).await;

        // The operator moves the pump while the device is alone. The dose that
        // is about to replay happened before this, to monstera-01.
        rebind_actuator(&api, "monstera-01", "fern-01").await;

        persist(
            &api,
            &replay_named(boot_a(), &[0], true, 35.0, Some("monstera-01")),
        )
        .await;

        let charged = rhizo_storage::repo::command::delivered_in_window(&api.db, "monstera-01", 0)
            .await
            .unwrap();
        assert!(
            (charged - 35.0).abs() < 1e-6,
            "the plant the device watered must carry the charge, got {charged}"
        );
        let untouched = rhizo_storage::repo::command::delivered_in_window(&api.db, "fern-01", 0)
            .await
            .unwrap();
        assert!(
            untouched.abs() < 1e-6,
            "a plant that was never watered must not be charged, got {untouched}"
        );

        // Exactly once: one row, not one per replay and not one per binding.
        let rows: i64 =
            sqlx::query_scalar("SELECT count(*) FROM watering_events WHERE plant_id='monstera-01'")
                .fetch_one(api.db.pool())
                .await
                .unwrap();
        assert_eq!(rows, 1, "one charge for one dose");
    }

    /// The same scenario replayed repeatedly still charges once, and rebinding
    /// *between* replays does not add a second charge.
    #[tokio::test]
    async fn safety_016_a_named_dose_survives_repeated_replay_and_further_rebinding() {
        let api = TestApi::start().await;
        api.waterable("monstera-01").await;
        api.plant("fern-01").await;
        api.device_connected().await;
        boot(&api, boot_a()).await;

        let batch = replay_named(boot_a(), &[0], true, 35.0, Some("monstera-01"));
        persist(&api, &batch).await;
        rebind_actuator(&api, "monstera-01", "fern-01").await;
        persist(&api, &batch).await;
        rebind_actuator(&api, "fern-01", "monstera-01").await;
        persist(&api, &batch).await;

        let charged = rhizo_storage::repo::command::delivered_in_window(&api.db, "monstera-01", 0)
            .await
            .unwrap();
        assert!((charged - 35.0).abs() < 1e-6, "{charged}");
        let untouched = rhizo_storage::repo::command::delivered_in_window(&api.db, "fern-01", 0)
            .await
            .unwrap();
        assert!(untouched.abs() < 1e-6, "{untouched}");
    }

    /// **The negative control.** Strip `plant_id` from exactly the same batch
    /// and the misattribution comes back: the dose follows the binding.
    ///
    /// This is what proves the test above is testing the field and not the
    /// fixture. It also pins the documented behaviour for a v1 device that
    /// predates the field — binding-based attribution, which is the best the
    /// edge can do when the device says nothing.
    #[tokio::test]
    async fn without_a_named_plant_the_dose_follows_the_binding() {
        let api = TestApi::start().await;
        api.waterable("monstera-01").await;
        api.plant("fern-01").await;
        api.device_connected().await;
        boot(&api, boot_a()).await;
        rebind_actuator(&api, "monstera-01", "fern-01").await;

        persist(&api, &replay_named(boot_a(), &[0], true, 35.0, None)).await;

        let named_plant_would_have_been_charged =
            rhizo_storage::repo::command::delivered_in_window(&api.db, "monstera-01", 0)
                .await
                .unwrap();
        assert!(
            named_plant_would_have_been_charged.abs() < 1e-6,
            "an unnamed dose cannot reach the plant that was actually watered"
        );
        let bound = rhizo_storage::repo::command::delivered_in_window(&api.db, "fern-01", 0)
            .await
            .unwrap();
        assert!((bound - 35.0).abs() < 1e-6, "{bound}");
    }

    /// A name the edge has never provisioned falls back rather than aborting the
    /// replay. A foreign-key failure here would wedge reconciliation for ever,
    /// which is a far worse outcome than an approximate charge.
    #[tokio::test]
    async fn an_unknown_plant_name_falls_back_instead_of_failing_the_replay() {
        let api = TestApi::start().await;
        api.waterable("monstera-01").await;
        api.device_connected().await;
        boot(&api, boot_a()).await;

        persist(
            &api,
            &replay_named(boot_a(), &[0], true, 35.0, Some("never-provisioned")),
        )
        .await;

        assert!(
            !is_reconciling(&api.db, "plant-node-01").await.unwrap(),
            "the replay must still commit and release the plant"
        );
        let bound = rhizo_storage::repo::command::delivered_in_window(&api.db, "monstera-01", 0)
            .await
            .unwrap();
        assert!(
            (bound - 35.0).abs() < 1e-6,
            "the dose falls back to the bound plant, got {bound}"
        );
    }

    /// A device carrying the actuator for several plants cannot attribute an
    /// autonomous dose to one of them, so it charges **every** one: over-counting
    /// reduces future doses, under-counting would permit an extra one.
    #[tokio::test]
    async fn an_ambiguous_dose_is_charged_conservatively_to_every_bound_plant() {
        let api = TestApi::start().await;
        api.waterable("monstera-01").await;
        api.plant("fern-01").await;
        api.bind_actuator("fern-01").await;
        api.device_connected().await;
        boot(&api, boot_a()).await;
        persist(&api, &replay(boot_a(), &[0], true, 35.0)).await;

        for plant in ["monstera-01", "fern-01"] {
            let delivered = rhizo_storage::repo::command::delivered_in_window(&api.db, plant, 0)
                .await
                .unwrap();
            assert!((delivered - 35.0).abs() < 1e-6, "{plant} got {delivered}");
        }
    }
}
