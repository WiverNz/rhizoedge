//! Axum assembly with bounded requests and explicit-origin CORS.
#![allow(missing_docs)]
use super::{
    ApiState, bindings, device_config, devices, health, intents, measurement_policies,
    offline_policy, plants, presets, profiles, recommendation, watering,
};
use axum::{
    Router,
    extract::DefaultBodyLimit,
    http::{HeaderValue, header},
    middleware,
    response::{IntoResponse, Response},
    routing::{delete, get, post, put},
};
use std::time::Duration;

pub fn router(state: ApiState, origins: Vec<String>) -> Router {
    let cors_origins = std::sync::Arc::new(origins);
    let request_metrics = state.metrics.clone();
    Router::new()
        .route("/health/live", get(health::live))
        .route("/health/ready", get(health::ready))
        .route(
            "/metrics",
            get(|| async { rhizo_telemetry::render_prometheus() }),
        )
        .route("/api/v1/devices", get(devices::list))
        .route(
            "/api/v1/devices/{id}",
            get(devices::get).patch(devices::patch),
        )
        .route("/api/v1/devices/{id}/events", get(devices::events))
        .route("/api/v1/devices/{id}/config", put(device_config::put))
        .route(
            "/api/v1/devices/{id}/commands/tare",
            post(watering::tare),
        )
        .route(
            "/api/v1/devices/{id}/commands/calibrate",
            post(watering::calibrate),
        )
        .route(
            "/api/v1/devices/{id}/measurements/latest",
            get(devices::latest_measurements),
        )
        .route("/api/v1/quarantined-messages", get(devices::quarantined))
        // ------------------------------------------------------------ plants
        .route("/api/v1/plants", get(plants::list).post(plants::create))
        .route(
            "/api/v1/plants/{id}",
            get(plants::get)
                .patch(plants::patch)
                .delete(plants::delete),
        )
        .route(
            "/api/v1/plants/{id}/measurements",
            get(plants::measurements),
        )
        .route(
            "/api/v1/plants/{id}/watering-events",
            get(plants::watering_events),
        )
        .route("/api/v1/plants/{id}/events", get(plants::events))
        .route(
            "/api/v1/plants/{id}/recommendation",
            get(recommendation::get),
        )
        // The safety-critical endpoints. Every one of them reaches the pump
        // through `rhizo_domain::irrigation::evaluate`, and none of them accepts
        // an override, force, bypass, expedite, or wake parameter.
        .route("/api/v1/plants/{id}/water", post(watering::water))
        .route(
            "/api/v1/plants/{id}/auto-watering/enable",
            post(watering::enable_auto),
        )
        .route(
            "/api/v1/plants/{id}/auto-watering/disable",
            post(watering::disable_auto),
        )
        .route(
            "/api/v1/plants/{id}/lockout/clear",
            post(watering::clear_lockout),
        )
        .route("/api/v1/commands/{id}", get(watering::get_command))
        .route("/api/v1/intents/{id}", get(intents::get))
        .route(
            "/api/v1/plants/{id}/apply-preset",
            post(plants::apply_preset),
        )
        // ---------------------------------------------------------- bindings
        .route(
            "/api/v1/plants/{id}/bindings/sensors",
            get(bindings::list_sensors).put(bindings::put_sensor),
        )
        .route(
            "/api/v1/plants/{id}/bindings/sensors/{binding_id}",
            delete(bindings::delete_sensor),
        )
        .route(
            "/api/v1/plants/{id}/bindings/actuator",
            get(bindings::get_actuator)
                .put(bindings::put_actuator)
                .delete(bindings::delete_actuator),
        )
        // ------------------------------------------------ measurement policies
        .route(
            "/api/v1/plants/{id}/measurement-policies",
            get(measurement_policies::list),
        )
        .route(
            "/api/v1/plants/{id}/measurement-policies/{kind}",
            put(measurement_policies::put).delete(measurement_policies::delete),
        )
        // ---------------------------------------------------- offline policy
        .route(
            "/api/v1/plants/{id}/offline-policy",
            get(offline_policy::get).put(offline_policy::put),
        )
        .route(
            "/api/v1/plants/{id}/offline-policy/enable",
            post(offline_policy::enable),
        )
        .route(
            "/api/v1/plants/{id}/offline-policy/disable",
            post(offline_policy::disable),
        )
        // ---------------------------------------------------------- profiles
        .route(
            "/api/v1/profiles",
            get(profiles::list).post(profiles::create),
        )
        .route("/api/v1/profiles/{id}", get(profiles::get).put(profiles::put))
        // ----------------------------------------------------------- presets
        .route("/api/v1/presets", get(presets::list))
        .route("/api/v1/presets/{id}", get(presets::get))
        .layer(DefaultBodyLimit::max(64 * 1024))
        .layer(middleware::from_fn(
            move |request: axum::extract::Request, next: middleware::Next| {
                let metrics = request_metrics.clone();
                async move {
                    let route = bounded_route(request.uri().path());
                    let started = std::time::Instant::now();
                    let response = next.run(request).await;
                    metrics
                        .http_duration
                        .with_label_values(&[route, response.status().as_str()])
                        .observe(started.elapsed().as_secs_f64());
                    response
                }
            },
        ))
        .layer(middleware::from_fn(
            move |request: axum::extract::Request, next: middleware::Next| {
                let allowed = cors_origins.clone();
                async move {
                    let origin = request.headers().get(header::ORIGIN).cloned();
                    let mut response: Response = next.run(request).await;
                    if let Some(origin) = origin
                        && allowed
                            .iter()
                            .any(|v| HeaderValue::from_str(v).ok().as_ref() == Some(&origin))
                    {
                        response
                            .headers_mut()
                            .insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin);
                        response.headers_mut().insert(
                            header::ACCESS_CONTROL_ALLOW_METHODS,
                            HeaderValue::from_static("GET, POST, PUT, PATCH, DELETE"),
                        );
                    }
                    response
                }
            },
        ))
        .layer(middleware::from_fn(
            |request: axum::extract::Request, next: middleware::Next| async move {
                match tokio::time::timeout(Duration::from_secs(10), next.run(request)).await {
                    Ok(r) => r,
                    Err(_) => axum::http::StatusCode::REQUEST_TIMEOUT.into_response(),
                }
            },
        ))
        .with_state(state)
}

/// Collapses a request path to one of a fixed set of labels.
///
/// The label set is closed on purpose: a metric labelled with the raw path would
/// grow a new series per plant id, which is the cardinality explosion ADR-010
/// exists to prevent.
fn bounded_route(path: &str) -> &'static str {
    if path == "/api/v1/devices" {
        "/api/v1/devices"
    } else if path.ends_with("/events") && path.starts_with("/api/v1/devices/") {
        "/api/v1/devices/{id}/events"
    } else if path.ends_with("/measurements/latest") && path.starts_with("/api/v1/devices/") {
        "/api/v1/devices/{id}/measurements/latest"
    } else if path.ends_with("/config") && path.starts_with("/api/v1/devices/") {
        "/api/v1/devices/{id}/config"
    } else if path.ends_with("/commands/tare") && path.starts_with("/api/v1/devices/") {
        "/api/v1/devices/{id}/commands/tare"
    } else if path.ends_with("/commands/calibrate") && path.starts_with("/api/v1/devices/") {
        "/api/v1/devices/{id}/commands/calibrate"
    } else if path.starts_with("/api/v1/devices/") {
        "/api/v1/devices/{id}"
    } else if path == "/api/v1/plants" {
        "/api/v1/plants"
    } else if path.starts_with("/api/v1/plants/") {
        let tail = path.trim_start_matches("/api/v1/plants/");
        match tail.split_once('/').map(|(_, rest)| rest) {
            None => "/api/v1/plants/{id}",
            Some("measurements") => "/api/v1/plants/{id}/measurements",
            Some("watering-events") => "/api/v1/plants/{id}/watering-events",
            Some("events") => "/api/v1/plants/{id}/events",
            Some("recommendation") => "/api/v1/plants/{id}/recommendation",
            Some("water") => "/api/v1/plants/{id}/water",
            Some("apply-preset") => "/api/v1/plants/{id}/apply-preset",
            Some("auto-watering/enable" | "auto-watering/disable") => {
                "/api/v1/plants/{id}/auto-watering"
            }
            Some("lockout/clear") => "/api/v1/plants/{id}/lockout/clear",
            Some(rest) if rest.starts_with("bindings/actuator") => {
                "/api/v1/plants/{id}/bindings/actuator"
            }
            Some(rest) if rest.starts_with("bindings/sensors") => {
                "/api/v1/plants/{id}/bindings/sensors"
            }
            Some(rest) if rest.starts_with("measurement-policies") => {
                "/api/v1/plants/{id}/measurement-policies"
            }
            Some(rest) if rest.starts_with("offline-policy") => {
                "/api/v1/plants/{id}/offline-policy"
            }
            Some(_) => "unknown",
        }
    } else if path.starts_with("/api/v1/commands/") {
        "/api/v1/commands/{id}"
    } else if path.starts_with("/api/v1/intents/") {
        "/api/v1/intents/{id}"
    } else if path == "/api/v1/profiles" {
        "/api/v1/profiles"
    } else if path.starts_with("/api/v1/profiles/") {
        "/api/v1/profiles/{id}"
    } else if path == "/api/v1/presets" {
        "/api/v1/presets"
    } else if path.starts_with("/api/v1/presets/") {
        "/api/v1/presets/{id}"
    } else if path == "/health/live" {
        "/health/live"
    } else if path == "/health/ready" {
        "/health/ready"
    } else if path == "/metrics" {
        "/metrics"
    } else if path == "/api/v1/quarantined-messages" {
        "/api/v1/quarantined-messages"
    } else {
        "unknown"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode, header},
    };
    use tower::ServiceExt as _;

    async fn app(origins: Vec<String>) -> Router {
        let db = rhizo_storage::EdgeDb::in_memory().await.unwrap();
        db.migrate().await.unwrap();
        let e = rhizo_mqtt_contract::Envelope::from_json(include_bytes!(
            "../../../../test/fixtures/protocol/valid/status-with-capabilities.json"
        ))
        .unwrap();
        rhizo_storage::repo::ingest::persist_status(&db, &e, 1_000)
            .await
            .unwrap();
        let metrics = crate::metrics::Metrics::new().unwrap();
        let clock = std::sync::Arc::new(rhizo_testkit::TestClock::new(
            chrono::DateTime::from_timestamp_millis(1_000).unwrap(),
        ));
        router(
            ApiState {
                db: db.clone(),
                metrics: metrics.clone(),
                clock: clock.clone(),
                commander: crate::control::command::Commander::new(
                    db,
                    clock,
                    std::sync::Arc::new(crate::control::transport::RecordingTransport::new()),
                    metrics,
                ),
            },
            origins,
        )
    }
    #[tokio::test]
    async fn devices_shape_and_immutable_patch() {
        let app = app(vec![]).await;
        let response = app
            .clone()
            .oneshot(
                Request::get("/api/v1/devices/plant-node-01")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let value: serde_json::Value =
            serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(value["device_id"], "plant-node-01");
        assert!(value["capabilities"].is_array());
        let bad = app
            .oneshot(
                Request::patch("/api/v1/devices/plant-node-01")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"device_id":"other"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(bad.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }
    #[tokio::test]
    async fn cors_is_exact_and_body_is_bounded() {
        let app = app(vec!["http://allowed".to_owned()]).await;
        let denied = app
            .clone()
            .oneshot(
                Request::get("/api/v1/devices")
                    .header(header::ORIGIN, "http://evil")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(
            denied
                .headers()
                .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
                .is_none()
        );
        let allowed = app
            .clone()
            .oneshot(
                Request::get("/api/v1/devices")
                    .header(header::ORIGIN, "http://allowed")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            allowed.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN],
            "http://allowed"
        );
        let huge = app
            .oneshot(
                Request::patch("/api/v1/devices/plant-node-01")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(vec![b'x'; 70_000]))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(huge.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }
}
