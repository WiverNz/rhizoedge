//! PostgreSQL-backed append-only Rhizo cloud history service.
#![forbid(unsafe_code)]
#![allow(missing_docs)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use prometheus::{Histogram, HistogramOpts, IntCounterVec, Opts};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{PgPool, Postgres, Row, Transaction, postgres::PgPoolOptions};
use std::sync::Arc;
use uuid::Uuid;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations/cloud");

/// Shared cloud application state.
#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub metrics: CloudMetrics,
}

/// Bounded cloud metrics catalogue.
#[derive(Clone)]
pub struct CloudMetrics {
    ingested: IntCounterVec,
    duration: Histogram,
}
impl CloudMetrics {
    pub fn new() -> Result<Self, prometheus::Error> {
        let ingested = IntCounterVec::new(
            Opts::new("cloud_events_ingested_total", "Cloud ingestion outcomes"),
            &["outcome"],
        )?;
        let duration = Histogram::with_opts(HistogramOpts::new(
            "cloud_ingest_duration_seconds",
            "Cloud batch duration",
        ))?;
        let registry = rhizo_telemetry::registry();
        registry.register(Box::new(ingested.clone()))?;
        registry.register(Box::new(duration.clone()))?;
        for outcome in ["accepted", "duplicate", "rejected"] {
            ingested.with_label_values(&[outcome]);
        }
        Ok(Self { ingested, duration })
    }
}

/// Connects and applies fatal-at-startup forward migrations.
pub async fn connect(database_url: &str) -> Result<PgPool, sqlx::Error> {
    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(database_url)
        .await?;
    MIGRATOR.run(&pool).await?;
    Ok(pool)
}

/// Builds the complete cloud route set. It contains one append-only write route.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health/live", get(live))
        .route("/health/ready", get(ready))
        .route("/metrics", get(metrics))
        .route("/api/v1/edges/{edge_id}/events", post(ingest))
        .route("/api/v1/edges", get(edges))
        .route("/api/v1/edges/{edge_id}/devices", get(devices))
        .route("/api/v1/edges/{edge_id}/plants", get(plants))
        .route(
            "/api/v1/edges/{edge_id}/plants/{plant_id}/measurements",
            get(measurements),
        )
        .route(
            "/api/v1/edges/{edge_id}/plants/{plant_id}/watering-events",
            get(watering_events),
        )
        .layer(DefaultBodyLimit::max(5 * 1024 * 1024))
        .with_state(Arc::new(state))
}

async fn live() -> Json<Value> {
    Json(json!({"status":"live"}))
}
async fn ready(State(s): State<Arc<AppState>>) -> Response {
    match sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(&s.pool)
        .await
    {
        Ok(_) => (
            StatusCode::OK,
            Json(json!({"status":"ready","checks":{"postgres":"ok"}})),
        )
            .into_response(),
        Err(_) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"status":"not_ready","checks":{"postgres":"unreachable"}})),
        )
            .into_response(),
    }
}
async fn metrics() -> impl IntoResponse {
    (
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; version=0.0.4",
        )],
        rhizo_telemetry::render_prometheus(),
    )
}

/// Wire event accepted from an Edge instance.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CloudEvent {
    pub event_id: Uuid,
    pub kind: String,
    pub occurred_at: DateTime<Utc>,
    #[serde(default)]
    pub device_id: Option<String>,
    #[serde(default)]
    pub plant_id: Option<String>,
    pub payload: Value,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Batch {
    events: Vec<CloudEvent>,
}
#[derive(Serialize, Debug, PartialEq)]
struct BatchResponse {
    results: Vec<EventResult>,
}
#[derive(Serialize, Debug, PartialEq)]
#[serde(tag = "status", rename_all = "snake_case")]
enum EventResult {
    Accepted { event_id: Uuid },
    Duplicate { event_id: Uuid },
    Rejected { event_id: Uuid, error: String },
}

async fn ingest(
    State(s): State<Arc<AppState>>,
    Path(edge_id): Path<String>,
    Json(batch): Json<Batch>,
) -> Response {
    if batch.events.len() > 500 || !valid_edge_id(&edge_id) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":{"code":"invalid_envelope"}})),
        )
            .into_response();
    }
    let timer = s.metrics.duration.start_timer();
    match ingest_batch(&s.pool, &edge_id, &batch.events, &s.metrics).await {
        Ok(results) => {
            timer.observe_duration();
            (StatusCode::OK, Json(BatchResponse { results })).into_response()
        }
        Err(error) => {
            tracing::error!(%error, "cloud ingestion transaction failed");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn ingest_batch(
    pool: &PgPool,
    edge_id: &str,
    events: &[CloudEvent],
    metrics: &CloudMetrics,
) -> Result<Vec<EventResult>, sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("INSERT INTO edge_instances(edge_id) VALUES($1) ON CONFLICT(edge_id) DO UPDATE SET last_seen_at=now()")
        .bind(edge_id).execute(&mut *tx).await?;
    let mut results = Vec::with_capacity(events.len());
    for event in events {
        let inserted = sqlx::query("INSERT INTO synced_events(edge_id,event_id,kind,occurred_at,device_id,plant_id,payload) VALUES($1,$2,$3,$4,$5,$6,$7) ON CONFLICT(edge_id,event_id) DO NOTHING")
            .bind(edge_id).bind(event.event_id).bind(&event.kind).bind(event.occurred_at)
            .bind(&event.device_id).bind(&event.plant_id).bind(&event.payload).execute(&mut *tx).await?.rows_affected() == 1;
        let result = if !inserted {
            EventResult::Duplicate {
                event_id: event.event_id,
            }
        } else if !known_kind(&event.kind) {
            EventResult::Rejected {
                event_id: event.event_id,
                error: "unknown kind; preserved in ledger".into(),
            }
        } else {
            match project(&mut tx, edge_id, event).await {
                Ok(()) => EventResult::Accepted {
                    event_id: event.event_id,
                },
                Err(error) => EventResult::Rejected {
                    event_id: event.event_id,
                    error,
                },
            }
        };
        let label = match result {
            EventResult::Accepted { .. } => "accepted",
            EventResult::Duplicate { .. } => "duplicate",
            EventResult::Rejected { .. } => "rejected",
        };
        metrics.ingested.with_label_values(&[label]).inc();
        results.push(result);
    }
    tx.commit().await?;
    Ok(results)
}

fn known_kind(kind: &str) -> bool {
    matches!(
        kind,
        "measurement.sample"
            | "device.status"
            | "device.event"
            | "device.config_applied"
            | "device.capabilities"
            | "device.policy_applied"
            | "device.isolated"
            | "device.reconciled"
            | "history.gap"
            | "plant.created"
            | "plant.updated"
            | "plant.state_changed"
            | "plant.binding_changed"
            | "plant.policy_changed"
            | "watering.started"
            | "watering.completed"
            | "watering.detected"
            | "watering.offline_autonomous"
            | "command.issued"
            | "command.settled"
            | "lockout.set"
            | "lockout.cleared"
            | "threshold.warning"
            | "threshold.critical"
            | "fertilisation.applied"
    )
}

async fn project(
    tx: &mut Transaction<'_, Postgres>,
    edge: &str,
    e: &CloudEvent,
) -> Result<(), String> {
    match e.kind.as_str() {
        "measurement.sample" => {
            let device = e.device_id.as_deref().ok_or("device_id is required")?;
            let point = text(&e.payload, "point").unwrap_or("default");
            let kind = text(&e.payload, "kind").ok_or("payload.kind is required")?;
            let unit = text(&e.payload, "unit").ok_or("payload.unit is required")?;
            let quality = text(&e.payload, "quality").ok_or("payload.quality is required")?;
            let value_num = e.payload.get("value").and_then(Value::as_f64);
            let value_bool = e.payload.get("value").and_then(Value::as_bool);
            let batch = text(&e.payload, "batch_id").and_then(|v| Uuid::parse_str(v).ok());
            sqlx::query("INSERT INTO measurements(edge_id,device_id,point,kind,occurred_at,value_num,value_bool,unit,quality,sensor_id,calibration_ref,batch_id,origin,plant_id) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14) ON CONFLICT(edge_id,device_id,point,kind,occurred_at) DO UPDATE SET value_num=excluded.value_num,value_bool=excluded.value_bool,unit=excluded.unit,quality=excluded.quality,sensor_id=excluded.sensor_id,calibration_ref=excluded.calibration_ref,batch_id=excluded.batch_id,origin=excluded.origin,plant_id=excluded.plant_id")
                .bind(edge).bind(device).bind(point).bind(kind).bind(e.occurred_at).bind(value_num).bind(value_bool).bind(unit).bind(quality)
                .bind(text(&e.payload,"sensor_id")).bind(text(&e.payload,"calibration_ref")).bind(batch).bind(text(&e.payload,"origin").unwrap_or("live")).bind(&e.plant_id)
                .execute(&mut **tx).await.map_err(|x|x.to_string())?;
        }
        "device.status" => {
            let device = e.device_id.as_deref().ok_or("device_id is required")?;
            sqlx::query("INSERT INTO devices(edge_id,device_id,name,firmware_version,status,last_seen_at,payload) VALUES($1,$2,$3,$4,$5,$6,$7) ON CONFLICT(edge_id,device_id) DO UPDATE SET name=excluded.name,firmware_version=excluded.firmware_version,status=excluded.status,last_seen_at=excluded.last_seen_at,payload=excluded.payload WHERE excluded.last_seen_at > devices.last_seen_at OR devices.last_seen_at IS NULL")
                .bind(edge).bind(device).bind(text(&e.payload,"name")).bind(text(&e.payload,"firmware_version")).bind(text(&e.payload,"status")).bind(e.occurred_at).bind(&e.payload).execute(&mut **tx).await.map_err(|x|x.to_string())?;
        }
        "plant.created"
        | "plant.updated"
        | "plant.state_changed"
        | "plant.binding_changed"
        | "plant.policy_changed" => {
            let plant = e.plant_id.as_deref().ok_or("plant_id is required")?;
            sqlx::query("INSERT INTO plants(edge_id,plant_id,name,species,bindings_json,policies_json,payload,updated_at) VALUES($1,$2,$3,$4,$5,$6,$7,$8) ON CONFLICT(edge_id,plant_id) DO UPDATE SET name=COALESCE(excluded.name,plants.name),species=COALESCE(excluded.species,plants.species),bindings_json=COALESCE(excluded.bindings_json,plants.bindings_json),policies_json=COALESCE(excluded.policies_json,plants.policies_json),payload=excluded.payload,updated_at=excluded.updated_at WHERE excluded.updated_at >= plants.updated_at OR plants.updated_at IS NULL")
                .bind(edge).bind(plant).bind(text(&e.payload,"name")).bind(text(&e.payload,"species")).bind(e.payload.get("bindings")).bind(e.payload.get("policies")).bind(&e.payload).bind(e.occurred_at).execute(&mut **tx).await.map_err(|x|x.to_string())?;
        }
        "watering.started"
        | "watering.completed"
        | "watering.detected"
        | "watering.offline_autonomous" => project_watering(tx, edge, e).await?,
        _ => {
            let device = e.device_id.as_deref().unwrap_or("edge");
            sqlx::query("INSERT INTO device_events(edge_id,event_id,device_id,kind,severity,detail,occurred_at) VALUES($1,$2,$3,$4,$5,$6,$7) ON CONFLICT(edge_id,event_id) DO UPDATE SET detail=excluded.detail")
                .bind(edge).bind(e.event_id).bind(device).bind(&e.kind).bind(text(&e.payload,"severity").unwrap_or("info")).bind(&e.payload).bind(e.occurred_at).execute(&mut **tx).await.map_err(|x|x.to_string())?;
        }
    }
    Ok(())
}

async fn project_watering(
    tx: &mut Transaction<'_, Postgres>,
    edge: &str,
    e: &CloudEvent,
) -> Result<(), String> {
    let id = text(&e.payload, "watering_event_id")
        .and_then(|v| Uuid::parse_str(v).ok())
        .unwrap_or(e.event_id);
    let plant = e
        .plant_id
        .as_deref()
        .or_else(|| text(&e.payload, "plant_id"))
        .ok_or("plant_id is required")?;
    let completed = if e.kind == "watering.completed" {
        Some(e.occurred_at)
    } else {
        None
    };
    sqlx::query("INSERT INTO watering_events(edge_id,watering_event_id,plant_id,mode,origin,started_at,completed_at,requested_ml,delivered_ml,status,payload) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11) ON CONFLICT(edge_id,watering_event_id) DO UPDATE SET completed_at=COALESCE(watering_events.completed_at,excluded.completed_at),delivered_ml=COALESCE(excluded.delivered_ml,watering_events.delivered_ml),status=CASE WHEN excluded.completed_at IS NOT NULL THEN excluded.status ELSE watering_events.status END,payload=watering_events.payload||excluded.payload")
        .bind(edge).bind(id).bind(plant).bind(text(&e.payload,"mode").unwrap_or("automatic")).bind(text(&e.payload,"origin").unwrap_or("edge_command"))
        .bind(e.payload.get("started_at").and_then(Value::as_str).and_then(|v|DateTime::parse_from_rfc3339(v).ok()).map(|v|v.with_timezone(&Utc)).unwrap_or(e.occurred_at))
        .bind(completed).bind(e.payload.get("requested_ml").and_then(Value::as_f64)).bind(e.payload.get("delivered_ml").and_then(Value::as_f64)).bind(text(&e.payload,"status").unwrap_or(if completed.is_some(){"completed"}else{"started"})).bind(&e.payload)
        .execute(&mut **tx).await.map_err(|x|x.to_string())?;
    Ok(())
}
fn text<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}
fn valid_edge_id(v: &str) -> bool {
    (3..=32).contains(&v.len())
        && v.bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
}

#[derive(Deserialize)]
struct Page {
    limit: Option<i64>,
    cursor: Option<String>,
    from: Option<DateTime<Utc>>,
    to: Option<DateTime<Utc>>,
    resolution: Option<String>,
}
fn limit(q: &Page) -> i64 {
    q.limit.unwrap_or(500).clamp(1, 5000)
}
async fn edges(
    State(s): State<Arc<AppState>>,
    Query(q): Query<Page>,
) -> Result<Json<Value>, StatusCode> {
    let max = limit(&q);
    let cursor = q.cursor.unwrap_or_default();
    let rows=sqlx::query("SELECT edge_id,display_name,first_seen_at,last_seen_at FROM edge_instances WHERE edge_id>$1 ORDER BY edge_id LIMIT $2").bind(cursor).bind(max).fetch_all(&s.pool).await.map_err(|_|StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(
        json!({"items":rows.iter().map(|r|json!({"edge_id":r.get::<String,_>("edge_id"),"display_name":r.get::<Option<String>,_>("display_name"),"first_seen_at":r.get::<DateTime<Utc>,_>("first_seen_at"),"last_seen_at":r.get::<DateTime<Utc>,_>("last_seen_at")})).collect::<Vec<_>>(),"next_cursor":rows.last().map(|r|r.get::<String,_>("edge_id"))}),
    ))
}
async fn devices(
    State(s): State<Arc<AppState>>,
    Path(edge): Path<String>,
    Query(q): Query<Page>,
) -> Result<Json<Value>, StatusCode> {
    require_edge(&s.pool, &edge).await?;
    list_json(&s.pool,"SELECT row_to_json(t) AS item FROM (SELECT * FROM devices WHERE edge_id=$1 AND device_id>$2 ORDER BY device_id LIMIT $3)t",&edge,q).await
}
async fn plants(
    State(s): State<Arc<AppState>>,
    Path(edge): Path<String>,
    Query(q): Query<Page>,
) -> Result<Json<Value>, StatusCode> {
    require_edge(&s.pool, &edge).await?;
    list_json(&s.pool,"SELECT row_to_json(t) AS item FROM (SELECT * FROM plants WHERE edge_id=$1 AND plant_id>$2 ORDER BY plant_id LIMIT $3)t",&edge,q).await
}
async fn list_json(
    pool: &PgPool,
    sql: &'static str,
    edge: &str,
    q: Page,
) -> Result<Json<Value>, StatusCode> {
    let max = limit(&q);
    let cursor = q.cursor.unwrap_or_default();
    let rows = sqlx::query(sql)
        .bind(edge)
        .bind(cursor)
        .bind(max)
        .fetch_all(pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let items = rows
        .iter()
        .map(|r| r.get::<Value, _>("item"))
        .collect::<Vec<_>>();
    let next_cursor = items
        .last()
        .and_then(|v| v.get("device_id").or_else(|| v.get("plant_id")))
        .cloned()
        .unwrap_or(Value::Null);
    Ok(Json(json!({"items":items,"next_cursor":next_cursor})))
}
async fn measurements(
    State(s): State<Arc<AppState>>,
    Path((edge, plant)): Path<(String, String)>,
    Query(q): Query<Page>,
) -> Result<Json<Value>, StatusCode> {
    require_plant(&s.pool, &edge, &plant).await?;
    if q.resolution
        .as_deref()
        .is_some_and(|v| !matches!(v, "raw" | "minute" | "hour" | "day"))
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    let cursor = q
        .cursor
        .as_deref()
        .and_then(|v| DateTime::parse_from_rfc3339(v).ok())
        .map(|v| v.with_timezone(&Utc));
    let from = cursor.or(q.from).unwrap_or(DateTime::<Utc>::MIN_UTC);
    let to = q.to.unwrap_or(DateTime::<Utc>::MAX_UTC);
    if to < from {
        return Err(StatusCode::BAD_REQUEST);
    }
    let statement = match q.resolution.as_deref().unwrap_or("raw") {
        "raw" => {
            "SELECT row_to_json(t) item FROM (SELECT * FROM measurements WHERE edge_id=$1 AND plant_id=$2 AND occurred_at>$3 AND occurred_at<=$4 ORDER BY occurred_at LIMIT $5)t"
        }
        "minute" => {
            "SELECT row_to_json(t) item FROM (SELECT date_trunc('minute',occurred_at) occurred_at,kind,point,avg(value_num) value_num,bool_or(value_bool) value_bool,min(unit) unit,min(quality) quality,count(*) samples FROM measurements WHERE edge_id=$1 AND plant_id=$2 AND occurred_at>$3 AND occurred_at<=$4 GROUP BY 1,kind,point ORDER BY 1 LIMIT $5)t"
        }
        "hour" => {
            "SELECT row_to_json(t) item FROM (SELECT date_trunc('hour',occurred_at) occurred_at,kind,point,avg(value_num) value_num,bool_or(value_bool) value_bool,min(unit) unit,min(quality) quality,count(*) samples FROM measurements WHERE edge_id=$1 AND plant_id=$2 AND occurred_at>$3 AND occurred_at<=$4 GROUP BY 1,kind,point ORDER BY 1 LIMIT $5)t"
        }
        "day" => {
            "SELECT row_to_json(t) item FROM (SELECT date_trunc('day',occurred_at) occurred_at,kind,point,avg(value_num) value_num,bool_or(value_bool) value_bool,min(unit) unit,min(quality) quality,count(*) samples FROM measurements WHERE edge_id=$1 AND plant_id=$2 AND occurred_at>$3 AND occurred_at<=$4 GROUP BY 1,kind,point ORDER BY 1 LIMIT $5)t"
        }
        _ => return Err(StatusCode::BAD_REQUEST),
    };
    let max = limit(&q);
    let rows = sqlx::query(statement)
        .bind(edge)
        .bind(plant)
        .bind(from)
        .bind(to)
        .bind(max)
        .fetch_all(&s.pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let items = rows
        .iter()
        .map(|r| r.get::<Value, _>("item"))
        .collect::<Vec<_>>();
    let next_cursor = items
        .last()
        .and_then(|v| v.get("occurred_at"))
        .cloned()
        .unwrap_or(Value::Null);
    Ok(Json(json!({"items":items,"next_cursor":next_cursor})))
}
async fn watering_events(
    State(s): State<Arc<AppState>>,
    Path((edge, plant)): Path<(String, String)>,
    Query(q): Query<Page>,
) -> Result<Json<Value>, StatusCode> {
    require_plant(&s.pool, &edge, &plant).await?;
    let cursor = q
        .cursor
        .as_deref()
        .and_then(|v| DateTime::parse_from_rfc3339(v).ok())
        .map(|v| v.with_timezone(&Utc))
        .unwrap_or(DateTime::<Utc>::MIN_UTC);
    let max = limit(&q);
    let rows=sqlx::query("SELECT row_to_json(t) item FROM (SELECT * FROM watering_events WHERE edge_id=$1 AND plant_id=$2 AND started_at>$3 ORDER BY started_at LIMIT $4)t").bind(edge).bind(plant).bind(cursor).bind(max).fetch_all(&s.pool).await.map_err(|_|StatusCode::INTERNAL_SERVER_ERROR)?;
    let items = rows
        .iter()
        .map(|r| r.get::<Value, _>("item"))
        .collect::<Vec<_>>();
    let next_cursor = items
        .last()
        .and_then(|v| v.get("started_at"))
        .cloned()
        .unwrap_or(Value::Null);
    Ok(Json(json!({"items":items,"next_cursor":next_cursor})))
}
async fn require_edge(pool: &PgPool, edge: &str) -> Result<(), StatusCode> {
    let found = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM edge_instances WHERE edge_id=$1)",
    )
    .bind(edge)
    .fetch_one(pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if found {
        Ok(())
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}
async fn require_plant(pool: &PgPool, edge: &str, plant: &str) -> Result<(), StatusCode> {
    require_edge(pool, edge).await?;
    let found = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM plants WHERE edge_id=$1 AND plant_id=$2)",
    )
    .bind(edge)
    .bind(plant)
    .fetch_one(pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if found {
        Ok(())
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

/// Atomically rebuilds all projections for one edge from its immutable ledger.
pub async fn reproject(pool: &PgPool, edge: &str) -> Result<u64, String> {
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    for statement in [
        "DELETE FROM measurements WHERE edge_id=$1",
        "DELETE FROM watering_events WHERE edge_id=$1",
        "DELETE FROM device_events WHERE edge_id=$1",
        "DELETE FROM devices WHERE edge_id=$1",
        "DELETE FROM plants WHERE edge_id=$1",
    ] {
        sqlx::query(statement)
            .bind(edge)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
    }
    let rows=sqlx::query("SELECT event_id,kind,occurred_at,device_id,plant_id,payload FROM synced_events WHERE edge_id=$1 ORDER BY occurred_at,id").bind(edge).fetch_all(&mut *tx).await.map_err(|e|e.to_string())?;
    let count = rows.len() as u64;
    let mut processed = 0u64;
    for row in rows {
        let e = CloudEvent {
            event_id: row.get("event_id"),
            kind: row.get("kind"),
            occurred_at: row.get("occurred_at"),
            device_id: row.get("device_id"),
            plant_id: row.get("plant_id"),
            payload: row.get("payload"),
        };
        if known_kind(&e.kind) {
            project(&mut tx, edge, &e).await?;
        }
        processed = processed.saturating_add(1);
        if processed.is_multiple_of(1_000) {
            tracing::info!(
                edge_id = edge,
                processed,
                total = count,
                "reprojection progress"
            );
        }
    }
    tx.commit().await.map_err(|e| e.to_string())?;
    Ok(count)
}

/// Checks cardinalities whose ledger facts map onto one natural projection key.
pub async fn projections_consistent(pool: &PgPool, edge: &str) -> Result<bool, sqlx::Error> {
    let (expected_devices,actual_devices):(i64,i64)=sqlx::query_as("SELECT (SELECT count(DISTINCT device_id) FROM synced_events WHERE edge_id=$1 AND kind='device.status' AND device_id IS NOT NULL),(SELECT count(*) FROM devices WHERE edge_id=$1)").bind(edge).fetch_one(pool).await?;
    let (expected_measurements,actual_measurements):(i64,i64)=sqlx::query_as("SELECT (SELECT count(DISTINCT (device_id,payload->>'point',payload->>'kind',occurred_at)) FROM synced_events WHERE edge_id=$1 AND kind='measurement.sample' AND device_id IS NOT NULL),(SELECT count(*) FROM measurements WHERE edge_id=$1)").bind(edge).fetch_one(pool).await?;
    let (expected_watering,actual_watering):(i64,i64)=sqlx::query_as("SELECT (SELECT count(DISTINCT coalesce(payload->>'watering_event_id',event_id::text)) FROM synced_events WHERE edge_id=$1 AND kind IN('watering.started','watering.completed','watering.detected','watering.offline_autonomous')),(SELECT count(*) FROM watering_events WHERE edge_id=$1)").bind(edge).fetch_one(pool).await?;
    Ok(expected_devices == actual_devices
        && expected_measurements == actual_measurements
        && expected_watering == actual_watering)
}

/// Static route inventory used to pin the cloud's negative controls.
pub const ROUTES: &[(&str, &str)] = &[
    ("POST", "/api/v1/edges/{edge_id}/events"),
    ("GET", "/api/v1/edges"),
    ("GET", "/api/v1/edges/{edge_id}/devices"),
    ("GET", "/api/v1/edges/{edge_id}/plants"),
    (
        "GET",
        "/api/v1/edges/{edge_id}/plants/{plant_id}/measurements",
    ),
    (
        "GET",
        "/api/v1/edges/{edge_id}/plants/{plant_id}/watering-events",
    ),
];

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn no_cloud_command_config_or_poll_route() {
        for (method, path) in ROUTES {
            let lower = path.to_ascii_lowercase();
            assert!(!lower.contains("command"));
            assert!(!(method == &"POST" && (lower.contains("config") || lower.contains("policy"))));
            assert!(!lower.contains("instruction"));
        }
    }
    #[test]
    fn all_v1_kinds_are_known() {
        assert!(known_kind("measurement.sample"));
        assert!(known_kind("watering.offline_autonomous"));
        assert!(!known_kind("future.event"));
    }
    #[tokio::test]
    async fn postgres_ingest_idempotency_ordering_unknown_and_reprojection() {
        let url = match std::env::var("RHIZO_TEST_POSTGRES_URL") {
            Ok(v) => v,
            Err(_) => {
                assert!(
                    std::env::var_os("RHIZO_REQUIRE_POSTGRES").is_none(),
                    "RHIZO_REQUIRE_POSTGRES=1 but RHIZO_TEST_POSTGRES_URL is absent"
                );
                eprintln!(
                    "SKIPPING real PostgreSQL test; set RHIZO_REQUIRE_POSTGRES=1 to make this fatal"
                );
                return;
            }
        };
        let pool = connect(&url).await.unwrap();
        for table in [
            "synced_events",
            "measurements",
            "watering_events",
            "device_events",
            "devices",
            "plants",
            "edge_instances",
        ] {
            sqlx::query(sqlx::AssertSqlSafe(format!("DELETE FROM {table}")))
                .execute(&pool)
                .await
                .unwrap();
        }
        let metrics = CloudMetrics::new().unwrap();
        let old = CloudEvent {
            event_id: Uuid::new_v4(),
            kind: "device.status".into(),
            occurred_at: DateTime::parse_from_rfc3339("2026-08-31T10:00:00.123Z")
                .unwrap()
                .with_timezone(&Utc),
            device_id: Some("node-01".into()),
            plant_id: None,
            payload: json!({"status":"offline"}),
        };
        let newer = CloudEvent {
            event_id: Uuid::new_v4(),
            occurred_at: DateTime::parse_from_rfc3339("2026-08-31T11:00:00.987Z")
                .unwrap()
                .with_timezone(&Utc),
            payload: json!({"status":"online"}),
            ..old.clone()
        };
        let unknown = CloudEvent {
            event_id: Uuid::new_v4(),
            kind: "future.kind".into(),
            occurred_at: newer.occurred_at,
            device_id: None,
            plant_id: None,
            payload: json!({"kept":true}),
        };
        let first = ingest_batch(
            &pool,
            "home-01",
            &[newer.clone(), old.clone(), unknown.clone()],
            &metrics,
        )
        .await
        .unwrap();
        assert!(matches!(first[2], EventResult::Rejected { .. }));
        let replay = ingest_batch(
            &pool,
            "home-01",
            &[newer.clone(), old.clone(), unknown],
            &metrics,
        )
        .await
        .unwrap();
        assert!(
            replay
                .iter()
                .all(|v| matches!(v, EventResult::Duplicate { .. }))
        );
        let ledger: i64 =
            sqlx::query_scalar("SELECT count(*) FROM synced_events WHERE edge_id='home-01'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(ledger, 3);
        let(status,at):(Option<String>,Option<DateTime<Utc>>)=sqlx::query_as("SELECT status,last_seen_at FROM devices WHERE edge_id='home-01' AND device_id='node-01'").fetch_one(&pool).await.unwrap();
        assert_eq!(status.as_deref(), Some("online"));
        assert_eq!(at, Some(newer.occurred_at));
        assert!(projections_consistent(&pool, "home-01").await.unwrap());
        sqlx::query("DELETE FROM devices WHERE edge_id='home-01' AND device_id='node-01'")
            .execute(&pool)
            .await
            .unwrap();
        assert!(
            !projections_consistent(&pool, "home-01").await.unwrap(),
            "deliberate projection corruption is detected"
        );
        assert_eq!(reproject(&pool, "home-01").await.unwrap(), 3);
        assert!(projections_consistent(&pool, "home-01").await.unwrap());
        let before:Value=sqlx::query_scalar("SELECT coalesce(jsonb_agg(row_to_json(t)::jsonb ORDER BY device_id),'[]'::jsonb) FROM (SELECT * FROM devices WHERE edge_id='home-01')t").fetch_one(&pool).await.unwrap();
        assert_eq!(reproject(&pool, "home-01").await.unwrap(), 3);
        let after:Value=sqlx::query_scalar("SELECT coalesce(jsonb_agg(row_to_json(t)::jsonb ORDER BY device_id),'[]'::jsonb) FROM (SELECT * FROM devices WHERE edge_id='home-01')t").fetch_one(&pool).await.unwrap();
        assert_eq!(before, after);
        assert_eq!(reproject(&pool, "home-01").await.unwrap(), 3);
        let mut large = Vec::with_capacity(500);
        for index in 0..499 {
            large.push(CloudEvent {
                event_id: Uuid::new_v4(),
                kind: "device.event".into(),
                occurred_at: newer.occurred_at,
                device_id: Some("node-01".into()),
                plant_id: None,
                payload: json!({"severity":"info","index":index}),
            });
        }
        large.push(CloudEvent {
            event_id: Uuid::new_v4(), kind: "measurement.sample".into(), occurred_at: newer.occurred_at,
            device_id: None, plant_id: None,
            payload: json!({"point":"default","kind":"soil_moisture","unit":"vwc_percent","quality":"ok","value":31.5}),
        });
        let partial = ingest_batch(&pool, "partial-01", &large, &metrics)
            .await
            .unwrap();
        assert_eq!(
            partial
                .iter()
                .filter(|v| matches!(v, EventResult::Accepted { .. }))
                .count(),
            499
        );
        assert_eq!(
            partial
                .iter()
                .filter(|v| matches!(v, EventResult::Rejected { .. }))
                .count(),
            1
        );
        let partial_ledger: i64 =
            sqlx::query_scalar("SELECT count(*) FROM synced_events WHERE edge_id='partial-01'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            partial_ledger, 500,
            "projection rejection preserves the ledger fact"
        );
        pool.close().await;
    }
}
