# Local Development

How to run, test, and debug Rhizo Edge on a developer machine. No hardware
required for anything in this document.

---

## 1. Prerequisites

| Tool | Version | Needed for |
|---|---|---|
| Rust | **1.98.0**, pinned by `rust-toolchain.toml` | everything |
| Docker + Compose v2 | current | the full topology |
| `sqlx-cli` | `cargo install sqlx-cli --no-default-features --features sqlite,postgres` | migrations, offline query cache |
| `mosquitto-clients` | any | manual MQTT inspection |
| `jq` | any | reading JSON logs |

Firmware and UI toolchains are only needed from M9 and M12 respectively; see
[ADR-007](../adr/007-esp32-rust-framework-and-toolchain.md) and
[ADR-009](../adr/009-ui-architecture-and-rust-web-stack.md).

---

## 2. First run

Three steps, and the middle one is the easy one to forget: the broker refuses
to start without a password file, and that file is generated rather than
committed.

```bash
cp .env.example .env               # replace every change-me-* placeholder
./scripts/gen-mosquitto-passwd.sh  # generates deploy/mosquitto/passwd
docker compose -f deploy/docker-compose.yml up -d mosquitto
```

`gen-mosquitto-passwd.sh` creates one broker account per entry in `DEVICE_IDS`
plus the edge's own, and refuses to run while any password is still a
placeholder. It runs `mosquitto_passwd` inside the same `eclipse-mosquitto:2`
image the broker uses, so nothing needs to be installed locally. Re-running it
is safe — the file is rebuilt from `.env` each time.

Check the broker is up, authenticating, and enforcing its ACLs:

```bash
docker compose -f deploy/docker-compose.yml ps        # expect (healthy)
./scripts/verify-mosquitto-acls.sh                    # expect 8 passed, 0 failed
```

`verify-mosquitto-acls.sh` asserts what
[ADR-012](../adr/012-device-identity-and-provisioning.md) promises: anonymous
and wrong-password connections are refused, a device can use its own
`rhizo/v1/devices/{device_id}/#` subtree, and it is denied every other
device's. Configuring an ACL and enforcing one are different things, and a
typo in the pattern leaves a broker that starts cleanly and protects nothing.

Then run the edge against it:

```bash
RHIZO_EDGE__MQTT__BROKER_URL=mqtt://localhost:1883 \
RHIZO_EDGE__LOG__FORMAT=compact \
cargo run -p edge-controller
```

**As of M0 that is the whole topology.** The simulator, the edge's ingestion
and API, the cloud API, and PostgreSQL arrive in M2, M3–M4, and M7; their
Compose services are written out but commented in
`deploy/docker-compose.yml`, each naming the issue that turns it on. Once M8
completes the topology, `up --build` starts everything and:

```bash
curl -s localhost:8080/api/v1/overview | jq
curl -s localhost:8080/api/v1/devices  | jq
curl -s localhost:8080/metrics | grep -E 'devices_online|pending_cloud_events'
```

---

## 3. VS Code launch and simulation presets

The normal interactive workflow needs no platform-specific HTTP commands:

1. Run `Mosquitto: up` from **Tasks: Run Task**.
2. Select `Edge + one plant node` (or a battery/two-node compound) in **Run and
   Debug**, then press F5.
3. Run any `Rhizo:` task from **Tasks: Run Task**.

The launch compounds remain in `.vscode/launch.json`; the development controls
are Cargo process tasks in `.vscode/tasks.json`, so VS Code starts the same Rust
binary with the same argument boundaries on Windows, Linux, and WSL2. The most
useful tasks are:

- `Rhizo: Edge readiness`
- `Rhizo: show Edge device state...`
- `Rhizo: show plant recommendation...` — after the next simulator sample and
  Edge control tick, inspect the latest decision for a configured plant
- `Rhizo: simulator state`
- `Rhizo: set soil moisture...`
- `Rhizo: simulate event...` (leak, tank, restart, missed wake, disconnect, and
  recovery)
- `Rhizo scenario: dry plant`
- `Rhizo scenario: leak while dry`
- `Rhizo scenario: battery missed wake`
- `Rhizo scenario: recover normal`
- `Rhizo: reset local state (DELETES DEV DATA)` — after stopping the debug
  session, removes the configured local SQLite database and the simulator state
  files named by `DEVICE_IDS`; use this when intentionally starting a clean
  disposable topology

VS Code launches use `compact` logging: one event per line, with the component
and the most useful correlation identifier before the message. Levels are
coloured only when stdout is an interactive terminal, so CI logs and redirected
files contain no escape sequences. Set the format to `pretty` when expanded span
and source context is more useful than scanability; production remains `json`.

These tasks call `rhizo-devctl`, a development-only Rust binary. It reads
`.env`, with the process environment taking precedence. Edge remains governed
by `RHIZO_EDGE__API__BIND`; the simulator and the tool share
`RHIZO_SIMULATOR__CONTROL_BIND`. Neither address is embedded in a task. An
unspecified Edge bind such as `0.0.0.0:8080` is converted to the corresponding
loopback address only for the client's connection.

Mutation and scenario tasks confirm success from a fresh `/sim/state` read-back,
for example `✓ dry-plant applied: moisture_vwc=20.0`. A successful POST alone is
not reported as success. Multi-step failures name the step that failed and exit
non-zero, so a green task terminal means the displayed state was actually read
from the running simulator.

The simulator bind must stay loopback because its control API is a local test
affordance, not a device or production API. A second simulator can still use
the existing `--control-port` launch override; the standard presets intentionally
control the primary simulator named by `.env`.

The CLI is also available directly when an editor task is inconvenient:

```text
cargo run -p rhizo-devctl -- simulator state
cargo run -p rhizo-devctl -- simulator set-soil 20
cargo run -p rhizo-devctl -- simulator leak on
cargo run -p rhizo-devctl -- simulator tank empty
cargo run -p rhizo-devctl -- simulator missed-wake 1
cargo run -p rhizo-devctl -- simulator disconnect 900
cargo run -p rhizo-devctl -- simulator reconnect
cargo run -p rhizo-devctl -- edge readiness
cargo run -p rhizo-devctl -- edge device-state plant-node-01
cargo run -p rhizo-devctl -- edge plant-recommendation monstera-01
cargo run -p rhizo-devctl -- reset-local-state --confirm
```

## 4. Running pieces individually (manual fallback)

Often faster than rebuilding a container:

```bash
# broker only
docker compose -f deploy/docker-compose.yml up mosquitto

# edge on the host, against the containerised broker
RHIZO_EDGE__MQTT__BROKER_URL=mqtt://localhost:1883 \
RHIZO_EDGE__STORAGE__PATH=./data/edge.sqlite \
RHIZO_EDGE__LOG__FORMAT=compact \
RHIZO_EDGE__CLOUD__ENABLED=false \
cargo run -p edge-controller

# simulator on the host, fast virtual time
cargo run -p device-simulator -- \
  --device-id plant-node-01 \
  --broker mqtt://localhost:1883 \
  --initial-moisture 42 \
  --time-scale 600
```

`RHIZO_EDGE__CLOUD__ENABLED=false` is the default and is the normal way to work
on M0–M6: the cloud is genuinely optional
([ADR-003](../adr/003-edge-first-ownership-model.md)).

---

## 5. Watching MQTT directly

The fastest way to understand what is happening:

```bash
# everything
mosquitto_sub -h localhost -u rhizo-edge -P "$MQTT_PASSWORD" -t 'rhizo/v1/#' -v

# commands only — the interesting ones
mosquitto_sub -h localhost -u rhizo-edge -P "$MQTT_PASSWORD" \
  -t 'rhizo/v1/devices/+/commands/#' -v

# check retained state (should show status and config, and NOTHING on commands)
mosquitto_sub -h localhost -u rhizo-edge -P "$MQTT_PASSWORD" \
  -t 'rhizo/v1/#' -v --retained-only
```

That last command is worth running after any change to the publish paths. A
retained message on a `commands/*` topic is a protocol violation and would cause
repeated watering ([ADR-002](../adr/002-mqtt-topic-versioning-and-qos.md)).

Injecting a command by hand, to test the device's independent veto:

```bash
mosquitto_pub -h localhost -u rhizo-edge -P "$MQTT_PASSWORD" \
  -t 'rhizo/v1/devices/plant-node-01/commands/water' -q 1 -m '{
    "v":1,"kind":"command.water","message_id":"018fd7b1-4c2e-7f10-a3b8-9d1e2f304050",
    "device_id":"plant-node-01",
    "data":{"command_id":"018fd7b1-4c2e-7f10-a3b8-9d1e2f304051",
            "requested_ml":10000,"issued_at_ms":1756121500000,
            "expires_at_ms":1756121620000}}'
```

The simulator must clamp or reject this (SAFETY-007). If it delivers 10 000 ml,
that is the most serious class of bug this project has.

---

## 6. Inspecting the database

```bash
sqlite3 ./data/edge.sqlite

.headers on
.mode column

-- latest readings
SELECT device_id, datetime(received_at/1000,'unixepoch') AS t,
       moisture_vwc, tank_level_percent, leak_detected
FROM measurements ORDER BY received_at DESC LIMIT 10;

-- what the machine did
SELECT plant_id, mode, delivered_ml, status,
       datetime(completed_at/1000,'unixepoch') AS done
FROM watering_events ORDER BY completed_at DESC LIMIT 20;

-- rolling 24h total (the SAFETY-006 query)
SELECT plant_id, SUM(delivered_ml)
FROM watering_events
WHERE completed_at > (strftime('%s','now')*1000 - 86400000)
  AND mode IN ('automatic','recommended')
GROUP BY plant_id;

-- current control state
SELECT * FROM irrigation_state;

-- outbox health
SELECT status, COUNT(*) FROM pending_cloud_events GROUP BY status;
```

---

## 7. Accelerated scenarios

Real time makes a watering cycle take an hour. Don't wait:

```bash
docker compose -f deploy/docker-compose.yml -f deploy/docker-compose.test.yml \
  up --abort-on-container-exit --exit-code-from scenario-runner
```

A single scenario:

```bash
docker compose -f deploy/docker-compose.yml -f deploy/docker-compose.test.yml \
  run --rm scenario-runner --scenario scenario_cloud_outage_recovery
```

At `--time-scale 600`, a full multi-dose cycle with two 15-minute absorption
waits finishes in about six seconds.

---

## 8. Injecting faults (manual fallback)

Prefer `Rhizo: simulate event...` or one of the named scenario tasks in VS
Code. The raw HTTP examples below are useful when inspecting the control API
itself.

```bash
# at startup
cargo run -p device-simulator -- --device-id plant-node-01 --fault leak

# at runtime
curl -X POST localhost:9090/sim/fault -d '{"fault":"leak","enabled":true}'
curl -X POST localhost:9090/sim/fault -d '{"fault":"tank-empty","enabled":true}'
curl -X POST localhost:9090/sim/fault -d '{"fault":"clock-unsync","enabled":true}'
```

The full catalogue is in
[simulator-strategy.md](simulator-strategy.md) §6.

---

## 9. Tests

```bash
cargo test --workspace --all-features           # everything
cargo test -p rhizo-domain                      # fast, pure, the common loop
cargo test safety_                              # the entire safety suite
cargo test --test integration                   # needs a broker

cargo test -p rhizo-domain -- --nocapture prop_  # property tests with output
PROPTEST_CASES=10000 cargo test -p rhizo-domain safety_006   # hammer one invariant
```

`cargo test safety_` is the command that answers "are the invariants still
enforced?". It should be reflexive before pushing anything that touches the
domain crate.

---

## 9. Working with `sqlx` offline mode

`sqlx::query!` verifies SQL against a real schema at compile time. CI has no
database, so an offline cache is committed.

After changing a query or a migration:

```bash
export DATABASE_URL="sqlite://$PWD/data/edge.sqlite"
sqlx database create
sqlx migrate run --source migrations/edge
cargo sqlx prepare --workspace
cargo sqlx prepare --workspace --check   # CI/staleness check
git add .sqlx
```

A stale `.sqlx/` cache produces confusing compile errors. If a query "should
compile" and does not, regenerate the cache first
([ADR-004](../adr/004-sqlite-edge-persistence-model.md)).

**On Windows, use a relative URL.** `sqlite://$PWD/data/edge.sqlite` expands to
`sqlite://D:/...`, which `sqlx-cli` rejects with `(code: 14) unable to open
database file` — it reads the drive letter as an authority. `DATABASE_URL="sqlite://data/edge.sqlite"`
works, and so does running the whole procedure from WSL2.

The cache is committed on purpose and is **not** gitignored: a build with
`DATABASE_URL` unset — every CI job and every fresh clone — reads `.sqlx/`
instead of connecting, and fails with `set DATABASE_URL to use query macros
online, or run cargo sqlx prepare to update the query cache` if it is missing or
stale. One file per checked query is the expected shape.

---

## 10. Logs

```bash
RHIZO_EDGE__LOG__FORMAT=compact RUST_LOG=debug cargo run -p edge-controller

# JSON output, filtered
docker compose logs -f edge-controller | jq 'select(.fields.plant_id == "monstera-01")'
docker compose logs -f edge-controller | jq 'select(.level == "ERROR")'

# per-module verbosity
RUST_LOG=info,edge_controller::pipeline=trace,rumqttc=warn cargo run -p edge-controller
```

INFO should be quiet — a few lines per hour. If INFO is noisy, something is
logging a per-message event at the wrong level
([ADR-010](../adr/010-observability-strategy.md)).

---

## 11. Common problems

| Symptom | Cause | Fix |
|---|---|---|
| Edge starts, no telemetry | broker auth failure | check `.env`; `docker compose logs mosquitto` |
| `SQLITE_BUSY` under load | two writers | all writes go through the pipeline task ([ADR-004](../adr/004-sqlite-edge-persistence-model.md)) |
| `sqlx` compile errors after a schema change | stale offline cache | `cargo sqlx prepare` (§9) |
| Plant never leaves `Lock(StaleData)` | simulator not publishing, or scale mismatch | check `--sensors`; confirm edge and simulator use the same `--time-scale` |
| Plant never waters despite dry soil | `auto_watering_enabled` is `false` | it defaults to false by design ([ADR-011](../adr/011-configuration-and-secrets-model.md)) |
| Watering cycle never completes in a test | real time, not virtual | set `--time-scale` |
| `pending_cloud_events` grows forever | `cloud.enabled=true` with no cloud running | set it to `false`, or start the cloud |
| Device rejects every command | `clock_synced: false` | confirm the edge is publishing `edge.time` on the device's `time` topic; in the simulator, clear the `clock-unsync` fault |

---

## 12. Before opening a change

```bash
cargo fmt --all
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo run -p rhizo-docscheck
docker compose -f deploy/docker-compose.yml config >/dev/null
```

If the change touches either firmware-shared crate, also:

```bash
cargo build -p rhizo-mqtt-contract --no-default-features --target thumbv7em-none-eabi
cargo build -p rhizo-policy --no-default-features --target thumbv7em-none-eabi
```

Those commands stop a `std`-only dependency from silently breaking the firmware
build, which ordinary host tests do not exercise
([ADR-001](../adr/001-rust-workspace-and-crate-boundaries.md)).
