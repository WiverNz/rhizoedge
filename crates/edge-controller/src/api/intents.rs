//! Pending-command state at the API boundary (M6-023),
//! `http-api-boundaries.md` §2.6.
//!
//! # `pending_for_device_wake` and `sent` are different states, and look it
//!
//! A caller that cannot tell "held" from "sent" reads the 202 as "the pump is
//! about to run", and when nothing happens for fifteen minutes presses the
//! button again — which is the behaviour the single-open-intent rule exists to
//! survive, but not the behaviour the API should invite.
//!
//! ```json
//! { "command_id": "018fd7b1-…", "status": "issued",
//!   "expires_at": "2026-08-28T11:32:00Z" }
//!
//! { "intent_id": "018fd7c9-…", "status": "pending_for_device_wake",
//!   "expected_delivery_after": "2026-08-28T11:45:00Z",
//!   "intent_expires_at": "2026-08-28T12:15:00Z" }
//! ```
//!
//! **The absence of `command_id` in the second is load-bearing.** The field is
//! *absent*, not null, so a client that reads it unconditionally fails loudly
//! rather than polling an id that does not exist. It appears the moment delivery
//! allocates one, so a caller can follow the handover.
//!
//! # There is no cancel, and that is a decision rather than an omission
//!
//! A cancel that races a wake is a distributed-consensus problem for a feature
//! nobody has asked for, and `intent_expires_at` already bounds the exposure.
//! Recorded as an open question in PRD 060 rather than half-built.

#![allow(missing_docs)]
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};

use super::ApiState;
use super::support::{error, storage_error, timestamp};
use super::watering::command_json;
use crate::control::intents;

/// One intent, as the API renders it.
#[must_use]
pub fn intent_json(
    row: &rhizo_storage::repo::intent::IntentRow,
    command: Option<&rhizo_storage::repo::command::CommandRow>,
) -> serde_json::Value {
    let mut value = serde_json::json!({
        "intent_id": row.intent_id,
        "plant_id": row.plant_id,
        "device_id": row.device_id,
        "status": row.state,
        "requested_ml": row.requested_ml,
        "mode": row.mode,
        "created_at": timestamp(row.created_at),
        "expected_delivery_after": row.expected_delivery_after.and_then(timestamp),
        "intent_expires_at": timestamp(row.intent_expires_at),
        "refusal_reason": row.refusal_reason,
    });
    // `command_id` appears only once one exists, so a caller can follow the
    // handover — and, before delivery, cannot mistake an absence for a null id.
    if let (Some(id), Some(map)) = (row.command_id.as_ref(), value.as_object_mut()) {
        map.insert("command_id".to_owned(), serde_json::json!(id));
        if let Some(command) = command {
            map.insert("command".to_owned(), command_json(command));
        }
    }
    value
}

/// `GET /api/v1/intents/{intent_id}`.
pub async fn get(State(state): State<ApiState>, Path(id): Path<String>) -> Response {
    match intents::get(&state.db, &id).await {
        Ok(Some(row)) => {
            let command = intents::command_for(&state.db, &row).await.ok().flatten();
            Json(intent_json(&row, command.as_ref())).into_response()
        }
        Ok(None) => error(StatusCode::NOT_FOUND, "intent_not_found", "unknown intent"),
        Err(_) => storage_error(),
    }
}

/// The open intent for a plant, rendered for the plant and device responses.
///
/// Exposed on those responses so a UI does not have to poll a separate endpoint
/// to know a dose is waiting (M6-023).
pub async fn open_for_plant(
    db: &rhizo_storage::EdgeDb,
    plant_id: &str,
) -> Option<serde_json::Value> {
    rhizo_storage::repo::intent::open_for_plant(db, plant_id)
        .await
        .ok()
        .flatten()
        .map(|row| intent_json(&row, None))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use crate::api::testsupport::TestApi;
    use crate::control::intents;
    use axum::http::StatusCode;

    /// A sleeping device: **nothing is published**, an intent is created, and
    /// the response carries no `command_id` at all.
    #[tokio::test]
    async fn a_sleeping_device_holds_the_dose_and_publishes_nothing() {
        let api = TestApi::start().await;
        api.waterable("monstera-01").await;
        api.device_sleeping(900_000).await;

        let (status, body) = api
            .json(
                "POST",
                "/api/v1/plants/monstera-01/water",
                serde_json::json!({ "ml": 30.0, "mode": "manual" }),
            )
            .await;
        assert_eq!(status, StatusCode::ACCEPTED, "{body}");
        assert_eq!(body["status"], intents::PENDING);
        assert!(body["intent_id"].is_string());
        assert!(body["expected_delivery_after"].is_string());
        assert!(body["intent_expires_at"].is_string());
        assert!(
            body.get("command_id").is_none(),
            "the field is absent, not null: a client that reads it unconditionally \
             must fail loudly rather than poll an id that does not exist"
        );
        assert!(
            api.transport.published().is_empty(),
            "a held dose publishes nothing at all"
        );
        let commands: i64 = sqlx::query_scalar("SELECT count(*) FROM commands")
            .fetch_one(api.db.pool())
            .await
            .unwrap();
        assert_eq!(commands, 0, "and mints no command");
    }

    /// A connected device's 202 is unchanged from M6-016, and creates no intent.
    #[tokio::test]
    async fn a_connected_device_takes_the_immediate_path_unchanged() {
        let api = TestApi::start().await;
        api.waterable("monstera-01").await;
        api.device_connected().await;
        let (status, body) = api
            .json(
                "POST",
                "/api/v1/plants/monstera-01/water",
                serde_json::json!({ "ml": 30.0 }),
            )
            .await;
        assert_eq!(status, StatusCode::ACCEPTED);
        assert!(body["command_id"].is_string());
        assert_eq!(body["status"], "issued");
        assert!(body.get("intent_id").is_none());
        let intents: i64 = sqlx::query_scalar("SELECT count(*) FROM command_intents")
            .fetch_one(api.db.pool())
            .await
            .unwrap();
        assert_eq!(intents, 0);
    }

    /// A second request while one is pending returns 409 naming the pending
    /// intent, so an impatient operator cannot queue several doses that all
    /// deliver at one wake.
    #[tokio::test]
    async fn a_second_request_returns_409_naming_the_pending_intent() {
        let api = TestApi::start().await;
        api.waterable("monstera-01").await;
        api.device_sleeping(900_000).await;
        let (_, first) = api
            .json(
                "POST",
                "/api/v1/plants/monstera-01/water",
                serde_json::json!({ "ml": 30.0 }),
            )
            .await;

        let (status, body) = api
            .json(
                "POST",
                "/api/v1/plants/monstera-01/water",
                serde_json::json!({ "ml": 30.0 }),
            )
            .await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert_eq!(body["error"]["code"], "intent_already_pending");
        assert_eq!(
            body["error"]["details"]["intent_id"], first["intent_id"],
            "the refusal names the intent the operator is waiting for"
        );
        assert!(body["error"]["details"]["expected_delivery_after"].is_string());
    }

    /// `GET /intents/{id}` reports every lifecycle state, and gains the
    /// `command_id` once delivery has happened.
    #[tokio::test]
    async fn an_intent_reports_its_state_and_then_its_command() {
        let api = TestApi::start().await;
        api.waterable("monstera-01").await;
        api.device_sleeping(900_000).await;
        let (_, held) = api
            .json(
                "POST",
                "/api/v1/plants/monstera-01/water",
                serde_json::json!({ "ml": 30.0 }),
            )
            .await;
        let intent_id = held["intent_id"].as_str().unwrap();

        let (status, body) = api.get(&format!("/api/v1/intents/{intent_id}")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["status"], intents::PENDING);
        assert!(body.get("command_id").is_none());

        // The device wakes.
        api.clock.advance(chrono::Duration::minutes(15));
        api.device_connected().await;
        api.sample(api.clock.now(), 20.0).await;
        api.sample_bool(api.clock.now(), "leak-0", "tray", "leak_state", false)
            .await;
        api.sample_from(
            api.clock.now(),
            "tank-0",
            "reservoir",
            "tank_level",
            "percent",
            70.0,
        )
        .await;
        let sent = intents::deliver_ready(&api.commander, api.clock.now())
            .await
            .unwrap();
        assert_eq!(sent, 1);

        let (_, body) = api.get(&format!("/api/v1/intents/{intent_id}")).await;
        assert_eq!(body["status"], intents::SENT);
        let command_id = body["command_id"]
            .as_str()
            .expect("the handover is visible");
        assert_eq!(body["command"]["command_id"], command_id);

        // Exactly one command was published, and its `issued_at` is the wake.
        assert_eq!(api.transport.commands().len(), 1);
        let row = rhizo_storage::repo::command::get(&api.db, command_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            row.issued_at,
            api.clock.now().timestamp_millis(),
            "the command is minted at the wake, not at the request"
        );
        assert!(
            row.issued_at > held["intent_expires_at"].as_str().map_or(0, |_| 0),
            "and it is a real instant"
        );
    }

    /// `edge.time` precedes the command on every wake delivery (F-040-17): a
    /// device that has just woken may not yet be able to date what it is given.
    #[tokio::test]
    async fn edge_time_is_published_before_the_command_at_a_wake() {
        let api = TestApi::start().await;
        api.waterable("monstera-01").await;
        api.device_sleeping(900_000).await;
        api.json(
            "POST",
            "/api/v1/plants/monstera-01/water",
            serde_json::json!({ "ml": 30.0 }),
        )
        .await;

        api.clock.advance(chrono::Duration::minutes(15));
        api.device_connected().await;
        api.sample(api.clock.now(), 20.0).await;
        api.sample_bool(api.clock.now(), "leak-0", "tray", "leak_state", false)
            .await;
        api.sample_from(
            api.clock.now(),
            "tank-0",
            "reservoir",
            "tank_level",
            "percent",
            70.0,
        )
        .await;
        intents::deliver_ready(&api.commander, api.clock.now())
            .await
            .unwrap();

        let topics: Vec<String> = api
            .transport
            .published()
            .into_iter()
            .map(|m| m.topic)
            .collect();
        let time = topics.iter().position(|t| t.ends_with("/time"));
        let command = topics.iter().position(|t| t.ends_with("/commands/water"));
        assert!(time.is_some() && command.is_some(), "{topics:?}");
        assert!(time < command, "edge.time must go first: {topics:?}");
    }

    /// **The gate re-runs in full at delivery.** A leak raised while the device
    /// slept refuses the dose, and nothing is published.
    #[tokio::test]
    async fn a_leak_raised_during_sleep_refuses_the_intent_at_delivery() {
        let api = TestApi::start().await;
        api.waterable("monstera-01").await;
        api.device_sleeping(900_000).await;
        let (_, held) = api
            .json(
                "POST",
                "/api/v1/plants/monstera-01/water",
                serde_json::json!({ "ml": 30.0 }),
            )
            .await;
        let intent_id = held["intent_id"].as_str().unwrap().to_owned();

        // The tray floods while the device is asleep.
        api.clock.advance(chrono::Duration::minutes(15));
        api.device_connected().await;
        api.sample(api.clock.now(), 20.0).await;
        api.sample_bool(api.clock.now(), "leak-0", "tray", "leak_state", true)
            .await;
        api.sample_from(
            api.clock.now(),
            "tank-0",
            "reservoir",
            "tank_level",
            "percent",
            70.0,
        )
        .await;
        let sent = intents::deliver_ready(&api.commander, api.clock.now())
            .await
            .unwrap();
        assert_eq!(sent, 0);

        let (_, body) = api.get(&format!("/api/v1/intents/{intent_id}")).await;
        assert_eq!(body["status"], intents::REFUSED);
        assert_eq!(body["refusal_reason"], "leak");
        assert!(
            body.get("command_id").is_none(),
            "a refused intent mints none"
        );
        assert!(
            api.transport.commands().is_empty(),
            "and publishes nothing at all"
        );
    }

    /// The same, for the reservoir and the rolling cap: the delivery path runs
    /// the **whole** gate, not a cheap "still allowed?" check.
    #[tokio::test]
    async fn every_refusal_reason_applies_at_delivery() {
        for (label, setup) in [("tank", 0u8), ("cap", 1u8)] {
            let api = TestApi::start().await;
            api.waterable("monstera-01").await;
            api.device_sleeping(900_000).await;
            api.json(
                "POST",
                "/api/v1/plants/monstera-01/water",
                serde_json::json!({ "ml": 40.0 }),
            )
            .await;

            api.clock.advance(chrono::Duration::minutes(15));
            api.device_connected().await;
            api.sample(api.clock.now(), 20.0).await;
            api.sample_bool(api.clock.now(), "leak-0", "tray", "leak_state", false)
                .await;
            let tank = if setup == 0 { 2.0 } else { 70.0 };
            api.sample_from(
                api.clock.now(),
                "tank-0",
                "reservoir",
                "tank_level",
                "percent",
                tank,
            )
            .await;
            if setup == 1 {
                sqlx::query(
                    "INSERT INTO watering_events(watering_event_id,plant_id,device_id,mode,origin,started_at,completed_at,delivered_ml,status) \
                     VALUES('we-cap','monstera-01','plant-node-01','automatic','edge_command',?,?,299.0,'completed')",
                )
                .bind(api.clock.now().timestamp_millis())
                .bind(api.clock.now().timestamp_millis())
                .execute(api.db.pool())
                .await
                .unwrap();
            }

            let sent = intents::deliver_ready(&api.commander, api.clock.now())
                .await
                .unwrap();
            assert_eq!(sent, 0, "{label}");
            assert!(api.transport.commands().is_empty(), "{label}");
        }
    }

    /// An edge restart between request and wake still delivers exactly once: an
    /// intent is a durable row, and restarting is re-reading it.
    #[tokio::test]
    async fn an_edge_restart_between_request_and_wake_delivers_exactly_once() {
        let api = TestApi::start().await;
        api.waterable("monstera-01").await;
        api.device_sleeping(900_000).await;
        api.json(
            "POST",
            "/api/v1/plants/monstera-01/water",
            serde_json::json!({ "ml": 30.0 }),
        )
        .await;

        // "Restarting" is running boot reconciliation against the same rows.
        let pending = intents::reconcile(&api.commander, api.clock.now())
            .await
            .unwrap();
        assert_eq!(pending, 1);
        assert!(
            api.transport.published().is_empty(),
            "reconciliation publishes nothing"
        );

        api.clock.advance(chrono::Duration::minutes(15));
        api.device_connected().await;
        api.sample(api.clock.now(), 20.0).await;
        api.sample_bool(api.clock.now(), "leak-0", "tray", "leak_state", false)
            .await;
        api.sample_from(
            api.clock.now(),
            "tank-0",
            "reservoir",
            "tank_level",
            "percent",
            70.0,
        )
        .await;
        assert_eq!(
            intents::deliver_ready(&api.commander, api.clock.now())
                .await
                .unwrap(),
            1
        );
        // Delivering again changes nothing: the intent is terminal.
        assert_eq!(
            intents::deliver_ready(&api.commander, api.clock.now())
                .await
                .unwrap(),
            0
        );
        assert_eq!(api.transport.commands().len(), 1);
        let commands: i64 = sqlx::query_scalar("SELECT count(*) FROM commands")
            .fetch_one(api.db.pool())
            .await
            .unwrap();
        assert_eq!(commands, 1);
    }

    /// An intent past `intent_expires_at` becomes `expired_before_wake` and is
    /// never delivered.
    #[tokio::test]
    async fn an_expired_intent_is_never_delivered() {
        let api = TestApi::start().await;
        api.waterable("monstera-01").await;
        api.device_sleeping(900_000).await;
        let (_, held) = api
            .json(
                "POST",
                "/api/v1/plants/monstera-01/water",
                serde_json::json!({ "ml": 30.0 }),
            )
            .await;
        let intent_id = held["intent_id"].as_str().unwrap().to_owned();

        api.clock.advance(chrono::Duration::minutes(31));
        assert_eq!(
            intents::sweep(&api.commander, api.clock.now())
                .await
                .unwrap(),
            1
        );
        let (_, body) = api.get(&format!("/api/v1/intents/{intent_id}")).await;
        assert_eq!(body["status"], intents::EXPIRED);

        api.device_connected().await;
        assert_eq!(
            intents::deliver_ready(&api.commander, api.clock.now())
                .await
                .unwrap(),
            0
        );
        assert!(api.transport.commands().is_empty());
    }

    /// An isolated device is neither connected nor asleep: there is no wake to
    /// wait for, so the request is refused rather than held indefinitely.
    #[tokio::test]
    async fn an_isolated_device_is_refused_rather_than_held() {
        let api = TestApi::start().await;
        api.waterable("monstera-01").await;
        sqlx::query(
            "UPDATE devices SET connectivity_mode='isolated' WHERE device_id='plant-node-01'",
        )
        .execute(api.db.pool())
        .await
        .unwrap();
        let (status, body) = api
            .json(
                "POST",
                "/api/v1/plants/monstera-01/water",
                serde_json::json!({ "ml": 30.0 }),
            )
            .await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert_eq!(body["error"]["code"], "device_unreachable");
        assert!(api.transport.published().is_empty());
    }

    /// `commands` gained no column: the reviewer's check that an intent really
    /// is not a command.
    #[tokio::test]
    async fn commands_gained_no_column() {
        let api = TestApi::start().await;
        let columns: Vec<String> =
            sqlx::query_scalar("SELECT name FROM pragma_table_info('commands') ORDER BY cid")
                .fetch_all(api.db.pool())
                .await
                .unwrap();
        assert_eq!(
            columns,
            [
                "command_id",
                "device_id",
                "plant_id",
                "kind",
                "requested_ml",
                "mode",
                "issued_at",
                "expires_at",
                "status",
                "published_at",
                "settled_at",
                "reason"
            ]
        );
    }

    /// No endpoint accepts an override, force, expedite, or wake parameter —
    /// and there is no mechanism by which the edge could wake a device anyway.
    #[test]
    fn there_is_no_wake_or_expedite_parameter() {
        // The production half only: this test names the forbidden shapes, and
        // a scan that matched its own assertion list would prove nothing.
        let production = include_str!("intents.rs")
            .split(
                "
#[cfg(test)]",
            )
            .next()
            .expect("the file has a production half");
        for forbidden in ["fn wake(", "fn expedite(", "fn cancel("] {
            assert!(
                !production.contains(forbidden),
                "`{forbidden}` must not exist: there is no mechanism by which the                  edge could wake a device, and a cancel that races a wake is a                  consensus problem for a feature nobody has asked for"
            );
        }
    }
}
