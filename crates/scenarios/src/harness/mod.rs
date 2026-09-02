//! Typed clients, lifecycle control, isolation, and failure diagnostics.

use crate::scenarios::Scenario;
use anyhow::{Context, Result, bail, ensure};
use rumqttc::{AsyncClient, Event, MqttOptions, Packet, QoS};
use serde_json::Value;
use sqlx::{PgPool, SqlitePool};
use std::{path::PathBuf, process::Command, sync::Arc, time::Duration};
use tokio::sync::Mutex;

/// Captured MQTT publication observable at the broker boundary.
#[derive(Clone, Debug)]
pub struct CapturedMqtt {
    /// Topic as received by the spy subscriber.
    pub topic: String,
    /// Exact payload bytes.
    pub payload: Vec<u8>,
    /// Broker retain bit.
    pub retain: bool,
}

/// Complete assembled-system harness.
pub struct Harness {
    /// Deterministic scenario seed.
    pub seed: u64,
    /// Edge API base URL.
    pub edge_url: String,
    /// Simulator control API base URL.
    pub simulator_url: String,
    /// Battery simulator control API base URL.
    pub battery_url: String,
    /// Cloud API base URL.
    pub cloud_url: String,
    /// Shared typed HTTP client.
    pub http: reqwest::Client,
    /// Direct SQLite reader.
    pub sqlite: SqlitePool,
    /// Direct PostgreSQL reader.
    pub postgres: PgPool,
    mqtt: Arc<Mutex<Vec<CapturedMqtt>>>,
    mqtt_client: AsyncClient,
    artifacts: PathBuf,
    expected_time_scale: f64,
}

impl Harness {
    /// Loads endpoints, opens both databases, and starts the MQTT spy.
    pub async fn from_env(seed: u64) -> Result<Self> {
        let edge_url = env("RHIZO_SCENARIO__EDGE_URL", "http://127.0.0.1:8080");
        let simulator_url = env("RHIZO_SCENARIO__SIMULATOR_URL", "http://127.0.0.1:9090");
        let battery_url = env(
            "RHIZO_SCENARIO__BATTERY_URL",
            "http://battery-simulator:9090",
        );
        let cloud_url = env("RHIZO_SCENARIO__CLOUD_URL", "http://127.0.0.1:8081");
        let edge_db = env("RHIZO_SCENARIO__EDGE_DB", "data/edge.sqlite");
        let postgres_url = env(
            "RHIZO_SCENARIO__POSTGRES_URL",
            "postgres://rhizo:rhizo@127.0.0.1:5432/rhizo",
        );
        let expected_time_scale = env("RHIZO_SCENARIO__TIME_SCALE", "600")
            .parse::<f64>()
            .context("RHIZO_SCENARIO__TIME_SCALE must be numeric")?;
        let sqlite = SqlitePool::connect(&format!("sqlite://{edge_db}?mode=rw"))
            .await
            .context("open edge SQLite for observable-state assertions")?;
        let postgres = PgPool::connect(&postgres_url)
            .await
            .context("connect to PostgreSQL for observable-state assertions")?;
        let mqtt = Arc::new(Mutex::new(Vec::new()));
        let mqtt_client = start_mqtt_spy(Arc::clone(&mqtt))?;
        Ok(Self {
            seed,
            edge_url,
            simulator_url,
            battery_url,
            cloud_url,
            http: reqwest::Client::new(),
            sqlite,
            postgres,
            mqtt,
            mqtt_client,
            artifacts: PathBuf::from(env("RHIZO_SCENARIO__ARTIFACTS", "test/artifacts")),
            expected_time_scale,
        })
    }

    /// Proves Edge, simulator, and runner agree before any scenario executes.
    ///
    /// M8-004: asserted *before* the first scenario, because a mismatch found
    /// halfway through a suite wastes the whole run and produces failures that
    /// read like logic bugs. The message names both values and the variable,
    /// since the fix is always a Compose change.
    pub async fn assert_time_scale_agreement(&self) -> Result<()> {
        let edge = self
            .get_json(&format!("{}/api/v1/overview", self.edge_url))
            .await?;
        let simulator = self
            .get_json(&format!("{}/sim/scale", self.simulator_url))
            .await?;
        let edge_scale = number(&edge, "time_scale")?;
        let simulator_scale = number(&simulator, "time_scale")?;
        if edge_scale != simulator_scale || edge_scale != self.expected_time_scale {
            bail!(
                "RHIZO_TIME_SCALE mismatch: edge={edge_scale}, simulator={simulator_scale}, runner={}; all services must use the one Compose variable",
                self.expected_time_scale
            );
        }
        Ok(())
    }

    /// Proves the edge under test can actually be crashed at an exact instant.
    ///
    /// SCEN-051 and SCEN-102 arm a marker and expect the process to die. The
    /// hooks are a compile-time feature the production image does not carry, so
    /// an overlay run against a production build would leave those two
    /// scenarios asserting nothing in particular. Refusing to start is the only
    /// honest answer: PRD 080's failure table says an environment that cannot
    /// exercise its subject must fail loudly rather than report a pass.
    pub async fn assert_fault_injection_available(&self) -> Result<()> {
        let edge = self
            .get_json(&format!("{}/api/v1/overview", self.edge_url))
            .await?;
        ensure!(
            edge.get("fault_injection").and_then(Value::as_bool) == Some(true),
            "the edge under test was not built with the `e2e-faults` feature, so SCEN-051 and \
             SCEN-102 could not crash it; rebuild the overlay image so its CARGO_FEATURES build \
             argument applies"
        );
        Ok(())
    }

    /// Proves each device authenticates as itself, not as the edge principal.
    ///
    /// ADR-012 makes the broker username the `device_id`, and the ACL's
    /// `pattern readwrite rhizo/v1/devices/%u/#` is what turns that into a real
    /// boundary. A topology that logged its devices in as `rhizo-edge` would
    /// hand each one `readwrite rhizo/v1/#`, and every scenario that believes
    /// it is exercising a confined device would be exercising a client with
    /// fleet-wide rights instead.
    ///
    /// # Why this is a delivery test and not a subscribe test
    ///
    /// Mosquitto answers an unauthorised SUBSCRIBE with a perfectly ordinary
    /// SUBACK and filters the traffic afterwards, message by message — measured
    /// against this broker, not assumed. That is the same shape as the
    /// documented PUBLISH case, where a denied publication is acknowledged and
    /// discarded, so neither acknowledgement can serve as evidence. What *is*
    /// observable is what arrives: subscribe as the device to a neighbour's
    /// subtree and to its own, publish a marker to each as the edge principal,
    /// and require the second to arrive while the first never does. The
    /// ordering is the proof — the own-marker arriving is what makes the
    /// negative result a refusal rather than a race.
    pub async fn assert_device_identity_is_enforced(&self) -> Result<()> {
        let device_id = self
            .get_json(&format!("{}/sim/state", self.simulator_url))
            .await?
            .get("device_id")
            .and_then(Value::as_str)
            .context("simulator device_id")?
            .to_owned();
        ensure!(
            device_id != env("RHIZO_SCENARIO__EDGE_PRINCIPAL", "rhizo-edge"),
            "the simulator is authenticated as the edge principal, not as a device (ADR-012)"
        );
        let variable = format!(
            "RHIZO_DEVICE_{}_PASSWORD",
            device_id.to_uppercase().replace('-', "_")
        );
        let password = std::env::var(&variable)
            .with_context(|| format!("{variable} is required to prove the ACL boundary"))?;

        let neighbour = "acl-probe-neighbour";
        let own_topic = format!("rhizo/v1/devices/{device_id}/acl-probe");
        let neighbour_topic = format!("rhizo/v1/devices/{neighbour}/acl-probe");
        let received = Arc::new(Mutex::new(Vec::<String>::new()));
        let listener = listen_as(
            &device_id,
            &password,
            &[
                format!("rhizo/v1/devices/{device_id}/#"),
                format!("rhizo/v1/devices/{neighbour}/#"),
            ],
            Arc::clone(&received),
        )
        .await
        .context("connect as the device to probe the ACL boundary")?;

        self.publish(&neighbour_topic, b"probe".to_vec()).await?;
        self.publish(&own_topic, b"probe".to_vec()).await?;
        let mut delivered_own = false;
        for _ in 0..100 {
            if received.lock().await.iter().any(|t| t == &own_topic) {
                delivered_own = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let seen = received.lock().await.clone();
        let _ = listener.disconnect().await;
        ensure!(
            delivered_own,
            "{device_id} received nothing on its own subtree, so this probe proves nothing about \
             the boundary"
        );
        ensure!(
            !seen.iter().any(|t| t == &neighbour_topic),
            "{device_id} received {neighbour_topic}; devices are authenticating with an account \
             broader than ADR-012 allows, and every scenario relying on device confinement is \
             vacuous"
        );
        Ok(())
    }

    /// Runs one isolated scenario.
    pub async fn run(&self, scenario: &Scenario) -> Result<()> {
        self.reset_scenario().await?;
        (scenario.run)(self).await
    }

    /// Performs a typed GET and requires a successful JSON response.
    pub async fn get_json(&self, url: &str) -> Result<Value> {
        self.http
            .get(url)
            .send()
            .await
            .with_context(|| format!("GET {url}"))?
            .error_for_status()
            .with_context(|| format!("GET {url} returned failure"))?
            .json()
            .await
            .with_context(|| format!("decode JSON from {url}"))
    }

    /// Sends JSON to an assembled-system API and returns both status and body.
    pub async fn json(
        &self,
        method: reqwest::Method,
        url: &str,
        body: Value,
    ) -> Result<(reqwest::StatusCode, Value)> {
        let response = self
            .http
            .request(method, url)
            .json(&body)
            .send()
            .await
            .with_context(|| format!("request {url}"))?;
        let status = response.status();
        let bytes = response.bytes().await?;
        let value = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes)?
        };
        Ok((status, value))
    }

    /// Mutates only the simulator's modelled environment or fault set.
    pub async fn simulator_post(&self, path: &str, body: Value) -> Result<Value> {
        let url = format!("{}{}", self.simulator_url, path);
        let (status, value) = self.json(reqwest::Method::POST, &url, body).await?;
        if !status.is_success() {
            bail!("POST {url} returned {status}: {value}");
        }
        Ok(value)
    }

    /// Mutates only the battery simulator's modelled environment or faults.
    pub async fn battery_post(&self, path: &str, body: Value) -> Result<Value> {
        let url = format!("{}{}", self.battery_url, path);
        let (status, value) = self.json(reqwest::Method::POST, &url, body).await?;
        if !status.is_success() {
            bail!("POST {url} returned {status}: {value}");
        }
        Ok(value)
    }

    /// Recreates the fixed battery node with no persisted state.
    pub async fn reset_battery_simulator(&self) -> Result<()> {
        self.stop_service("battery-simulator")?;
        let state = std::path::Path::new("/var/lib/rhizo-simulator/battery-node-01.state.json");
        if state.exists() {
            std::fs::remove_file(state)?;
        }
        for suffix in ["status", "policy", "config"] {
            self.clear_retained(&format!("rhizo/v1/devices/battery-node-01/{suffix}"))
                .await?;
        }
        self.start_service("battery-simulator")?;
        wait_http(&self.http, &format!("{}/sim/state", self.battery_url)).await
    }

    /// Waits until an HTTP endpoint answers.
    ///
    /// Tolerates the connection refusals a container emits between `docker
    /// start` returning and its listener binding. A scenario that polled with
    /// [`Self::get_json`] instead would abort on the first refusal, which is
    /// not a failure of the thing under test — it is the scenario asking too
    /// early.
    pub async fn wait_ready(&self, url: &str) -> Result<()> {
        wait_http(&self.http, url).await
    }

    /// Waits until the device simulator's control API answers again.
    pub async fn wait_simulator_ready(&self) -> Result<()> {
        self.wait_ready(&format!("{}/sim/state", self.simulator_url))
            .await
    }

    /// Returns MQTT captured since the last isolation boundary.
    pub async fn mqtt(&self) -> Vec<CapturedMqtt> {
        self.mqtt.lock().await.clone()
    }

    /// Begins a new negative-assertion window without restarting the spy.
    pub async fn clear_mqtt(&self) {
        self.mqtt.lock().await.clear();
    }

    /// Publishes through the broker boundary as the edge test principal.
    pub async fn publish(&self, topic: &str, payload: Vec<u8>) -> Result<()> {
        self.mqtt_client
            .publish(topic, QoS::AtLeastOnce, false, payload)
            .await
            .with_context(|| format!("publish test message to {topic}"))
    }

    /// Clears one retained broker record at an explicit scenario boundary.
    pub async fn clear_retained(&self, topic: &str) -> Result<()> {
        self.mqtt_client
            .publish(topic, QoS::AtLeastOnce, true, Vec::new())
            .await
            .with_context(|| format!("clear retained MQTT record {topic}"))
    }

    /// Stops a Compose service by its label-derived container id.
    pub fn stop_service(&self, service: &str) -> Result<()> {
        docker_service("stop", service)
    }

    /// Stops a Compose service that may not have been created.
    ///
    /// Used only where the harness deliberately does not care — a scenario that
    /// asked for a service by name and found nothing should still fail loudly,
    /// which is why this is a second method rather than a softer `stop_service`.
    pub fn stop_service_if_present(&self, service: &str) -> Result<()> {
        match docker_container_id(service) {
            Ok(_) => docker_service("stop", service),
            Err(_) => Ok(()),
        }
    }

    /// Abruptly terminates a service for crash-recovery scenarios.
    pub fn kill_service(&self, service: &str) -> Result<()> {
        docker_service("kill", service)
    }

    /// Freezes a service without emitting its graceful-shutdown status.
    pub fn pause_service(&self, service: &str) -> Result<()> {
        docker_service("pause", service)
    }

    /// Resumes a service frozen by [`Self::pause_service`].
    pub fn unpause_service(&self, service: &str) -> Result<()> {
        docker_service("unpause", service)
    }

    /// Starts a Compose service by its label-derived container id.
    pub fn start_service(&self, service: &str) -> Result<()> {
        docker_service("start", service)
    }

    /// Creates a one-shot fault marker inside a Compose service.
    pub fn arm_service_fault(&self, service: &str, marker: &str) -> Result<()> {
        let id = docker_container_id(service)?;
        let status = Command::new("docker")
            .args(["exec", &id, "touch", marker])
            .status()?;
        if !status.success() {
            bail!("could not arm fault marker {marker} in {service}");
        }
        Ok(())
    }

    /// Corrupts the simulator's persisted offline-policy checksum while its
    /// container is stopped, modelling a torn/bit-flipped NVS policy blob.
    pub fn corrupt_simulator_policy(&self) -> Result<()> {
        let state_dir = std::path::Path::new("/var/lib/rhizo-simulator");
        for entry in std::fs::read_dir(state_dir)? {
            let path = entry?.path();
            if !path.is_file() || path.extension().and_then(|v| v.to_str()) != Some("json") {
                continue;
            }
            let bytes = std::fs::read(&path)?;
            let value: Value = serde_json::from_slice(&bytes)?;
            let mut state: device_simulator::state::PersistentState =
                serde_json::from_value(value["state"].clone())?;
            if let Some(policy) = state.policy_active.as_mut() {
                policy.checksum = "corrupt".to_owned();
                std::fs::write(&path, device_simulator::state::encode_state(&state)?)?;
                return Ok(());
            }
        }
        bail!("no persisted simulator policy checksum was found to corrupt")
    }

    /// Whether the Compose service's container process is currently running.
    pub fn service_running(&self, service: &str) -> Result<bool> {
        let id = docker_container_id(service)?;
        let output = Command::new("docker")
            .args(["inspect", "--format", "{{.State.Running}}", &id])
            .output()?;
        ensure!(
            output.status.success(),
            "docker inspect failed for {service}"
        );
        Ok(String::from_utf8(output.stdout)?.trim() == "true")
    }

    /// Writes DB rows, recent MQTT, and container logs for CI diagnosis.
    pub async fn dump_failure(&self, scenario: &str) -> Result<()> {
        std::fs::create_dir_all(&self.artifacts)?;
        let mut dump = String::new();
        for (table, statement) in [
            ("measurements", "SELECT * FROM measurements LIMIT 200"),
            ("commands", "SELECT * FROM commands LIMIT 200"),
            ("watering_events", "SELECT * FROM watering_events LIMIT 200"),
            (
                "irrigation_state",
                "SELECT * FROM irrigation_state LIMIT 200",
            ),
            (
                "pending_cloud_events",
                "SELECT * FROM pending_cloud_events LIMIT 200",
            ),
            ("history_gaps", "SELECT * FROM history_gaps LIMIT 200"),
            ("command_intents", "SELECT * FROM command_intents LIMIT 200"),
        ] {
            dump.push_str(&format!("\n===== SQLite {table} =====\n"));
            match sqlx::query(statement).fetch_all(&self.sqlite).await {
                Ok(rows) => dump.push_str(&format!("{} rows\n", rows.len())),
                Err(error) => dump.push_str(&format!("ERROR: {error}\n")),
            }
        }
        for (section, statement) in [
            (
                "devices",
                "SELECT json_object('device_id',device_id,'boot_id',boot_id,'status',status,'connectivity_mode',connectivity_mode,'clock_synced',clock_synced,'last_seen_at',last_seen_at) FROM devices LIMIT 200",
            ),
            (
                "replay_progress",
                "SELECT json_object('device_id',device_id,'boot_id',boot_id,'through_device_seq',through_device_seq,'complete',complete,'updated_at',updated_at) FROM replay_progress LIMIT 200",
            ),
            (
                "plants",
                "SELECT json_object('plant_id',plant_id,'auto_watering_enabled',auto_watering_enabled,'lockout_reason',lockout_reason,'lockout_since',lockout_since) FROM plants LIMIT 200",
            ),
            (
                "irrigation_state rows",
                "SELECT json_object('plant_id',plant_id,'state',state,'state_since',state_since,'doses_this_cycle',doses_this_cycle,'wait_until',wait_until,'active_command_id',active_command_id) FROM irrigation_state LIMIT 200",
            ),
            (
                "commands rows",
                "SELECT json_object('command_id',command_id,'device_id',device_id,'plant_id',plant_id,'status',status,'requested_ml',requested_ml,'issued_at',issued_at,'settled_at',settled_at,'reason',reason) FROM commands LIMIT 200",
            ),
            (
                "watering rows",
                "SELECT json_object('watering_event_id',watering_event_id,'plant_id',plant_id,'command_id',command_id,'status',status,'requested_ml',requested_ml,'delivered_ml',delivered_ml) FROM watering_events LIMIT 200",
            ),
            (
                "measurement tail",
                "SELECT json_object('id',id,'device_id',device_id,'sensor_id',sensor_id,'kind',kind,'value_num',value_num,'value_bool',value_bool,'quality',quality,'received_at',received_at,'batch_id',batch_id) FROM measurements ORDER BY id DESC LIMIT 50",
            ),
        ] {
            dump.push_str(&format!("\n===== SQLite {section} =====\n"));
            match sqlx::query_scalar::<_, String>(statement)
                .fetch_all(&self.sqlite)
                .await
            {
                Ok(rows) => {
                    for row in rows {
                        dump.push_str(&row);
                        dump.push('\n');
                    }
                }
                Err(error) => dump.push_str(&format!("ERROR: {error}\n")),
            }
        }
        dump.push_str("\n===== MQTT =====\n");
        for message in self.mqtt().await.iter().rev().take(500).rev() {
            dump.push_str(&format!(
                "{} retain={} {}\n",
                message.topic,
                message.retain,
                String::from_utf8_lossy(&message.payload)
            ));
        }
        dump.push_str("\n===== simulator state =====\n");
        match self
            .get_json(&format!("{}/sim/state", self.simulator_url))
            .await
        {
            Ok(state) => dump.push_str(&format!("{state}\n")),
            Err(error) => dump.push_str(&format!("ERROR: {error}\n")),
        }
        for service in [
            "mosquitto",
            "device-simulator",
            "edge-controller",
            "cloud-api",
            "postgres",
        ] {
            dump.push_str(&format!("\n===== logs {service} =====\n"));
            if let Ok(output) = docker_output(&["logs", "--tail", "200"], service) {
                dump.push_str(&output);
            }
        }
        let path = self.artifacts.join(format!("{scenario}-failure.txt"));
        std::fs::write(path, dump)?;
        Ok(())
    }

    /// Restores the independently runnable clean boundary between scenarios.
    pub async fn reset_scenario(&self) -> Result<()> {
        self.start_service("cloud-api")?;
        self.start_service("edge-controller")?;
        self.start_service("device-simulator")?;
        wait_http(&self.http, &format!("{}/health/ready", self.edge_url)).await?;
        wait_http(&self.http, &format!("{}/sim/state", self.simulator_url)).await?;
        let device_id = self
            .get_json(&format!("{}/sim/state", self.simulator_url))
            .await?
            .get("device_id")
            .and_then(Value::as_str)
            .context("simulator reset device_id")?
            .to_owned();
        // The battery profile may be active while the aggregate suite is
        // running.  Stop it before deleting device rows and durable simulator
        // state; otherwise it can re-register between those operations and a
        // subsequent clean restart legitimately looks stale to the edge.
        // Tolerated as absent: a `run --rm scenario-runner` against a partially
        // started topology should still be able to reset itself.
        self.stop_service_if_present("battery-simulator")?;
        self.stop_service("device-simulator")?;
        self.stop_service("edge-controller")?;
        for statement in [
            "DELETE FROM plant_events",
            "DELETE FROM plant_recommendations",
            "DELETE FROM plant_threshold_state",
            "DELETE FROM plant_dry_state",
            "DELETE FROM sensor_stuck_state",
            "DELETE FROM device_isolation_periods",
            "DELETE FROM pending_cloud_events",
            "DELETE FROM command_intents",
            "DELETE FROM irrigation_state",
            "DELETE FROM offline_policies",
            "DELETE FROM measurement_policies",
            "DELETE FROM actuator_bindings",
            "DELETE FROM sensor_bindings",
            "DELETE FROM device_capabilities",
            "DELETE FROM watering_events",
            "DELETE FROM command_results",
            "DELETE FROM commands",
            "DELETE FROM quarantined_messages",
            "DELETE FROM replay_progress",
            "DELETE FROM history_gaps",
            "DELETE FROM device_events",
            "DELETE FROM actuator_states",
            "DELETE FROM measurements",
            "DELETE FROM processed_messages",
            "DELETE FROM plants",
            "DELETE FROM plant_profiles WHERE profile_id != 'default'",
            "DELETE FROM devices",
        ] {
            sqlx::query(statement)
                .execute(&self.sqlite)
                .await
                .with_context(|| format!("run scenario reset statement {statement}"))?;
        }
        sqlx::query(
            "TRUNCATE watering_events, measurements, device_events, devices, plants, synced_events, edge_instances RESTART IDENTITY CASCADE",
        )
        .execute(&self.postgres)
        .await
        .context("clear PostgreSQL observable state")?;
        let state_dir = std::path::Path::new("/var/lib/rhizo-simulator");
        if state_dir.exists() {
            for entry in std::fs::read_dir(state_dir)? {
                let path = entry?.path();
                if path.is_file() {
                    std::fs::remove_file(path)?;
                }
            }
        }
        for suffix in ["status", "policy", "config"] {
            for reset_device in [&device_id, "battery-node-01"] {
                self.mqtt_client
                    .publish(
                        format!("rhizo/v1/devices/{reset_device}/{suffix}"),
                        QoS::AtLeastOnce,
                        true,
                        Vec::new(),
                    )
                    .await
                    .with_context(|| format!("clear prior retained {reset_device} {suffix}"))?;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        self.mqtt.lock().await.clear();
        self.start_service("edge-controller")?;
        wait_http(&self.http, &format!("{}/health/ready", self.edge_url)).await?;
        self.start_service("device-simulator")?;
        wait_http(&self.http, &format!("{}/sim/state", self.simulator_url)).await?;
        self.mqtt.lock().await.clear();
        Ok(())
    }
}

async fn wait_http(client: &reqwest::Client, url: &str) -> Result<()> {
    for _ in 0..100 {
        if client
            .get(url)
            .send()
            .await
            .is_ok_and(|response| response.status().is_success())
        {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    bail!("service did not become ready at {url}")
}

fn env(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_owned())
}

fn number(value: &Value, field: &str) -> Result<f64> {
    value
        .get(field)
        .and_then(Value::as_f64)
        .with_context(|| format!("response field {field} is missing or non-numeric"))
}

/// Connects as `user`, subscribes to `filters`, and records arriving topics.
///
/// Used only by the ACL probe. The returned client keeps the session alive; the
/// caller disconnects it.
async fn listen_as(
    user: &str,
    password: &str,
    filters: &[String],
    received: Arc<Mutex<Vec<String>>>,
) -> Result<AsyncClient> {
    let broker = env("RHIZO_SCENARIO__MQTT_HOST", "mosquitto");
    let mut options = MqttOptions::new(format!("rhizo-acl-probe-{user}"), broker, 1883);
    options.set_credentials(user, password);
    options.set_clean_session(true);
    let (client, mut events) = AsyncClient::new(options, 32);
    let subscriber = client.clone();
    let filters = filters.to_vec();
    let expected = filters.len();
    // Returning before the broker has acknowledged every SUBSCRIBE would make
    // the probe a race: the caller publishes immediately, and a marker sent
    // before the subscription exists is simply never delivered — which reads
    // exactly like the refusal the probe is looking for.
    let (ready, subscribed) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let mut ready = Some(ready);
        let mut acknowledged = 0usize;
        loop {
            match events.poll().await {
                Ok(Event::Incoming(Packet::ConnAck(_))) => {
                    for filter in &filters {
                        let _ = subscriber.subscribe(filter.clone(), QoS::AtLeastOnce).await;
                    }
                }
                Ok(Event::Incoming(Packet::SubAck(_))) => {
                    acknowledged += 1;
                    if acknowledged == expected
                        && let Some(signal) = ready.take()
                    {
                        let _ = signal.send(());
                    }
                }
                Ok(Event::Incoming(Packet::Publish(publication))) => {
                    received.lock().await.push(publication.topic);
                }
                Ok(_) => {}
                Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
            }
        }
    });
    tokio::time::timeout(Duration::from_secs(15), subscribed)
        .await
        .map_err(|_| anyhow::anyhow!("the broker did not acknowledge {user}'s subscriptions"))?
        .map_err(|_| anyhow::anyhow!("the {user} probe listener stopped before subscribing"))?;
    Ok(client)
}

fn start_mqtt_spy(messages: Arc<Mutex<Vec<CapturedMqtt>>>) -> Result<AsyncClient> {
    let broker = env("RHIZO_SCENARIO__MQTT_HOST", "mosquitto");
    let password = std::env::var("RHIZO_EDGE__MQTT__PASSWORD")
        .context("RHIZO_EDGE__MQTT__PASSWORD is required by the MQTT spy")?;
    let mut options = MqttOptions::new("rhizo-scenario-spy", broker, 1883);
    options.set_credentials("rhizo-edge", password);
    options.set_clean_session(true);
    let (client, mut events) = AsyncClient::new(options, 256);
    let control = client.clone();
    tokio::spawn(async move {
        if client
            .subscribe("rhizo/v1/#", QoS::AtLeastOnce)
            .await
            .is_err()
        {
            return;
        }
        loop {
            match events.poll().await {
                Ok(Event::Incoming(Packet::ConnAck(_))) => {
                    let _ = client.subscribe("rhizo/v1/#", QoS::AtLeastOnce).await;
                }
                Ok(Event::Incoming(Packet::Publish(publication))) => {
                    messages.lock().await.push(CapturedMqtt {
                        topic: publication.topic,
                        payload: publication.payload.to_vec(),
                        retain: publication.retain,
                    });
                }
                Ok(_) => {}
                Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
            }
        }
    });
    Ok(control)
}

fn docker_service(action: &str, service: &str) -> Result<()> {
    let id = docker_container_id(service)?;
    // `docker start`/`stop` echo the container id. Captured rather than
    // inherited, so the suite's own output stays a readable list of scenarios
    // and their verdicts (F-080-06) instead of a column of hex.
    let output = Command::new("docker").args([action, &id]).output()?;
    if !output.status.success() {
        bail!(
            "docker {action} failed for Compose service {service}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

fn docker_container_id(service: &str) -> Result<String> {
    let output = Command::new("docker")
        .args([
            "ps",
            "-aq",
            "--filter",
            &format!("label=com.docker.compose.service={service}"),
        ])
        .output()?;
    if !output.status.success() {
        bail!("docker ps failed while locating Compose service {service}");
    }
    let id = String::from_utf8(output.stdout)?.trim().to_owned();
    if id.is_empty() || id.lines().count() != 1 {
        bail!("expected exactly one container for Compose service {service}, got `{id}`");
    }
    Ok(id)
}

fn docker_output(args: &[&str], service: &str) -> Result<String> {
    let id = docker_container_id(service)?;
    let output = Command::new("docker").args(args).arg(id).output()?;
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}
