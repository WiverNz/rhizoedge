//! Axum assembly with bounded requests and explicit-origin CORS.
#![allow(missing_docs)]
use super::{ApiState, devices, health};
use axum::{
    Router,
    extract::DefaultBodyLimit,
    http::{HeaderValue, header},
    middleware,
    response::{IntoResponse, Response},
    routing::get,
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
        .route("/api/v1/quarantined-messages", get(devices::quarantined))
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
                            HeaderValue::from_static("GET, PATCH"),
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

fn bounded_route(path: &str) -> &'static str {
    if path == "/api/v1/devices" {
        "/api/v1/devices"
    } else if path.ends_with("/events") && path.starts_with("/api/v1/devices/") {
        "/api/v1/devices/{id}/events"
    } else if path.starts_with("/api/v1/devices/") {
        "/api/v1/devices/{id}"
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
        router(
            ApiState {
                db,
                metrics: crate::metrics::Metrics::new().unwrap(),
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
