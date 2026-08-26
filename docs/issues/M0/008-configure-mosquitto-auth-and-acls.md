# Issue M0-008 — Configure Mosquitto with authentication and per-device ACLs

**Milestone:** M0 · **PRD:** [PRD 000](../../prd/000-platform-foundation.md) · **Depends on:** M0-001

## Context

ADR-012 makes device identity a real boundary using the Mosquitto ACL pattern
`rhizo/v1/devices/%u/#`, which ties a device's topic subtree to its
authenticated username. Anonymous access is disabled from the first commit —
retrofitting authentication later means a period where it is absent.

## Goal

Provide a Mosquitto configuration enforcing authentication and per-device topic isolation.

## Scope

- `mosquitto.conf` with `allow_anonymous false`
- An ACL file: `pattern readwrite rhizo/v1/devices/%u/#` plus a broad `rhizo-edge` user
- `scripts/gen-mosquitto-passwd.sh` generating the password file from `.env`
- Persistence and logging configuration

## Non-goals

- TLS (deferred to M13).
- Per-device certificates (post-V1).
- The provisioning tool (M13-002).

## Dependencies

- M0-001

## Implementation notes

The `%u` substitution is what makes this more than decoration: a device
authenticated as `plant-node-01` physically cannot publish to
`rhizo/v1/devices/plant-node-02/...`. This is why ADR-002 puts `device_id`
before the message kind in the topic tree.

The generated `passwd` file must be gitignored. The script reads credentials
from `.env` and runs `mosquitto_passwd -b`.

The `rhizo-edge` account needs `topic readwrite rhizo/v1/#`.

## Acceptance criteria

- [x] An anonymous connection is refused.
- [x] A wrong password is refused.
- [x] `plant-node-01` can publish to its own telemetry topic.
- [x] `plant-node-01` is **denied** publishing to `plant-node-02`'s topic.
- [x] The `rhizo-edge` account can subscribe to `rhizo/v1/devices/+/#`.
- [x] `deploy/mosquitto/passwd` is gitignored and absent from the index.

## Verification

```bash
docker compose -f deploy/docker-compose.yml up -d mosquitto
mosquitto_sub -h localhost -t 'rhizo/v1/#' -v            # must FAIL (anonymous)
mosquitto_pub -h localhost -u plant-node-01 -P "$P1" -t 'rhizo/v1/devices/plant-node-02/telemetry/soil' -m '{}'  # must FAIL
```

## Tests required

- Manual verification here; automated as an integration test in M2-012.

## Documentation impact

- docs/testing/local-development.md already documents the mosquitto_sub commands.

## Files likely affected

```text
deploy/mosquitto/mosquitto.conf
deploy/mosquitto/aclfile
scripts/gen-mosquitto-passwd.sh
.gitignore
```
