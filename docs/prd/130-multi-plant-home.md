# PRD 130 — Multi-Plant Home System

**Milestone:** M13 · **Status:** PLANNED · **Depends on:** M12

> **Revised 2026-08-26.** Three operational deliverables were added:
> **release binary CI** (M13-013), the **MSRV + current-stable CI matrix**
> (M13-014), and the **optional Prometheus + Grafana profile** (M13-015).
>
> - **Release artefacts** so using Rhizo Edge does not require installing Rust
>   and building a workspace: a tag produces checksummed archives for the
>   components that exist by then, for targets that are actually tested. Nothing
>   in M1–M8 depends on this.
> - **MSRV matrix** so an accidental bump past 1.98.0 fails CI rather than
>   reaching a user on an older toolchain
>   ([ADR-001](../adr/001-rust-workspace-and-crate-boundaries.md)).
> - **Observability profile**, strictly opt-in. `docker compose up` without
>   `--profile observability` behaves exactly as before, and the M8 acceptance
>   suite never references it. Operational metrics go to Prometheus; **plant
>   history does not** — it stays in SQLite/PostgreSQL and is read through a SQL
>   datasource ([ADR-010](../adr/010-observability-strategy.md)).

## Summary

Scale from one plant to a household: several ESP32 nodes, per-device
credentials issued by tooling, multiple plants and profiles, notifications, a
supportable home deployment, and the operational tooling that a system running
unattended for months actually needs.

## Problem

V1 proves the architecture with one plant. A household has ten, some sharing a
device, some sharing a reservoir. Three things break at that scale:

1. **Provisioning by hand.** Ten devices means ten credential generations, ten
   serial sessions, and ten opportunities to reuse a password.
2. **Nobody watches a dashboard.** A lockout on plant seven at 3 a.m. is
   invisible until someone opens the app days later.
3. **Docker Compose on a laptop is not a deployment.** The system needs to
   survive reboots, rotate logs, and be backed up.

## Goals

1. Multiple devices and plants operating independently.
2. Provisioning tooling that generates credentials and configures the broker.
3. Notifications for lockouts and faults.
4. A supportable home deployment: systemd units, backup, log rotation.
5. Shared-reservoir awareness.
6. Multi-device failure testing.

## Non-goals

- Multi-tenancy or user accounts. One household, one operator.
- Cloud-based notification services. Local delivery mechanisms only.
- Zones, valves, or agricultural features ([PRD 140](140-field-readiness.md)).
- Authentication on the Edge API — still deferred, and still a stated limitation.

## User/system flows

**Adding a device:**

```text
rhizo-provision new --name "bedroom-node"
   → generates device_id and a random password
   → appends to the Mosquitto password file and reloads the broker
   → prints the serial provisioning commands to paste
   → device boots, connects, auto-registers (no plant)
   → operator attaches a plant in the UI
```

**A lockout at 3 a.m.:**

```text
plant 7 leak detected → Lock(Leak) → notification dispatched
   → operator sees it on their phone (ntfy) or in email
   → opens the app, sees the plant, investigates
```

## Functional requirements

### Scale

| ID | Requirement |
|---|---|
| F-130-01 | ≥ 10 devices and ≥ 20 plants without architectural change |
| F-130-02 | One device may serve several plants and one plant may bind sensor capabilities from several devices (ADR-016) |
| F-130-03 | Per-plant irrigation state is fully independent |
| F-130-04 | The control loop evaluates all plants within one tick period |
| F-130-05 | One device's failure never affects another's plants |
| F-130-06 | Commands to different devices may be in flight simultaneously |

### Provisioning

| ID | Requirement |
|---|---|
| F-130-10 | `rhizo-provision` generates a `device_id` and a 32-byte random password |
| F-130-11 | Updates the Mosquitto password file and triggers a reload |
| F-130-12 | Emits ready-to-paste serial provisioning commands |
| F-130-13 | **Never reuses a password**; refuses to overwrite an existing device without `--force` |
| F-130-14 | `rhizo-provision revoke` removes credentials and marks the device retired |
| F-130-15 | Documented password rotation procedure |

### Shared reservoir

| ID | Requirement |
|---|---|
| F-130-20 | A reservoir entity that several devices may reference |
| F-130-21 | Tank level from any device on that reservoir applies to all of them |
| F-130-22 | A low reservoir locks out **every** plant drawing from it |
| F-130-23 | Conflicting readings from two sensors on one reservoir resolve to the **lowest** value |

F-130-23 is the conservative choice: with two disagreeing sensors, the safe
belief is the one that prevents pumping.

### Notifications

| ID | Requirement |
|---|---|
| F-130-30 | Dispatch on: lockout set, device offline > threshold, pump fault, no-delivery, cloud sync broken > threshold |
| F-130-31 | Channels: ntfy, generic webhook, SMTP |
| F-130-32 | Rate-limited and deduplicated — one leak produces one notification, not one per tick |
| F-130-33 | Configurable per severity |
| F-130-34 | **Notification failure never affects control.** Fire-and-forget from a separate task, exactly like the cloud outbox. |
| F-130-35 | Optional daily digest |
| F-130-70 | Battery voltage trended per device using M5-005's existing least-squares trend, returning `None` on sparse data for the same reason |
| F-130-71 | A projected depletion date **only where the trend supports one**, absent otherwise — a LiFePO4 discharge curve is flat across most of its range, and a linear projection over the flat region confidently predicts a date that is wrong by weeks |
| F-130-72 | `battery_low` and `battery_critical` dispatched through the existing dispatcher and **coalesced per device**; a threshold crossed once and stayed across becomes a daily message that gets muted, which defeats the alert |
| F-130-73 | `device_wake_missed` dispatched as a **distinct** notification from the low-battery one — different causes, different remedies |
| F-130-74 | Battery state affects **no** watering decision, checked structurally ([ADR-018](../adr/018-battery-and-deep-sleep-device-mode.md) §7) |

F-130-34 restates the SAFETY-008 principle for a new outbound dependency: adding
a notification channel must not add a way for the control loop to block.

### Deployment

| ID | Requirement |
|---|---|
| F-130-40 | systemd units for `mosquitto` and `edge-controller` with restart policies |
| F-130-41 | Documented Raspberry Pi installation |
| F-130-42 | Automated SQLite backup with a verified restore procedure |
| F-130-43 | Log rotation via journald limits |
| F-130-44 | Measurement downsampling to hourly beyond the raw retention window |
| F-130-45 | An upgrade procedure that preserves data |
| F-130-46 | Backup and restore round-trip `power_mode`, `wake_interval_seconds`, and pending command intents |
| F-130-47 | Documented battery-node deployment: cell type, replacement interval derived from M10-012's **measured** budget, what to check when a node stops waking, and how to change a wake interval safely |

### UI

| ID | Requirement |
|---|---|
| F-130-50 | Multi-plant overview that stays legible at 20 plants |
| F-130-51 | Grouping by room or reservoir |
| F-130-52 | Bulk automation enable/disable with a confirmation listing affected plants |
| F-130-53 | Notification configuration |
| F-130-54 | Fleet views filter and group by power mode |
| F-130-55 | **Sleeping devices are not counted as offline** in any UI count or metric — a household with six battery nodes has roughly six sleeping devices at any instant, and that is the healthy state |

### Release and operations

| ID | Requirement |
|---|---|
| F-130-60 | A `v*` tag runs release CI and publishes checksummed archives whose binaries report the tagged version |
| F-130-61 | CI builds on the MSRV 1.98.0 and current stable so an accidental MSRV increase fails visibly |
| F-130-62 | An opt-in `observability` Compose profile adds Prometheus/Grafana; no normal service or test depends on it and plant history remains in SQL storage |

## Interfaces

```text
GET/POST  /api/v1/reservoirs
GET/PATCH /api/v1/reservoirs/{id}
GET/PUT   /api/v1/notifications/config
POST      /api/v1/notifications/test
GET       /api/v1/plants?group=&state=&locked=      # filtering
```

```bash
rhizo-provision new --name "bedroom-node" [--device-id …]
rhizo-provision list
rhizo-provision revoke <device_id>
rhizo-provision rotate <device_id>
rhizo-backup create|restore|verify
```

## Data model

New tables:

```sql
CREATE TABLE reservoirs (
    reservoir_id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    capacity_ml REAL,
    min_percent REAL NOT NULL DEFAULT 15.0,
    created_at INTEGER NOT NULL
);

ALTER TABLE devices ADD COLUMN reservoir_id TEXT REFERENCES reservoirs(reservoir_id);
ALTER TABLE devices ADD COLUMN retired_at INTEGER;
ALTER TABLE plants  ADD COLUMN group_name TEXT;

CREATE TABLE notification_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    kind TEXT NOT NULL, severity TEXT NOT NULL,
    subject_id TEXT, channel TEXT NOT NULL,
    status TEXT NOT NULL, sent_at INTEGER NOT NULL
);

CREATE TABLE measurements_hourly (
    device_id TEXT NOT NULL, sensor_id TEXT, point TEXT NOT NULL,
    kind TEXT NOT NULL, unit TEXT NOT NULL,
    hour_start INTEGER NOT NULL,
    value_avg REAL, value_min REAL, value_max REAL,
    true_count INTEGER, false_count INTEGER,
    sample_count INTEGER NOT NULL,
    PRIMARY KEY (device_id, sensor_id, point, kind, hour_start)
);
```

The aggregate remains keyed by measurement identity and kind, so adding a new
`MeasurementKind` does not require adding columns. Aggregation fields apply by
measurement class; incompatible values remain absent rather than coerced.

`notification_log` exists so a missed alert can be distinguished from an alert
that was never generated — the first is a delivery problem, the second is a
detection problem.

Devices are **retired, never deleted**, so their history stays attributable.

## State model

No new state machine. Each plant runs the M6 machine independently; each device
runs the M4 lifecycle independently.

One new cross-cutting rule: a reservoir's level is the **minimum** of its
sensors' reported levels, and unknown from any sensor makes the reservoir
unknown — which is a lockout.

## Failure modes

| Failure | Behaviour |
|---|---|
| One device offline | only its plants lock out |
| Two devices on one reservoir disagree | lowest value wins |
| Reservoir sensor unknown | every plant on it locks out |
| Control tick exceeds its period with many plants | logged; tick duration is a monitored metric; plants are evaluated in a stable order so none starves |
| Notification channel down | logged in `notification_log`, retried with backoff, **never blocks control** |
| Notification storm (many plants lock at once) | rate-limited and coalesced into a digest |
| Provisioning collision | refused unless `--force` |
| Backup fails | alert; the failure is itself notification-worthy |
| SD card degrading | documented as the most likely hardware failure; backup is the mitigation |

## Safety implications

No new invariants, but every existing one must hold **per plant and per device**
rather than globally. Three specific risks that scale introduces:

- **Cross-plant interference.** A defect that lets one plant's state affect
  another's would be a new class of bug. Mitigated by per-plant rows and
  independent evaluation, and tested by SCEN-080 (below).
- **Shared reservoir accounting.** Two devices drawing from one tank can each
  believe they have budget. Mitigated by F-130-22: a low reservoir locks out all
  its plants, and the device-side `FIRMWARE_MAX_DAILY_ML` bounds each device
  regardless.
- **Notification as a new blocking dependency.** Explicitly prevented by
  F-130-34.

`FIRMWARE_MAX_DAILY_ML` is per device. With several plants on one device, the
device cap now bounds their *combined* delivery — which is correct and
conservative, but must be documented so an operator does not configure per-plant
caps summing above it and then wonder why the device refuses.

## Observability

```text
devices_total / plants_total                    gauge
plants_locked_out{reason}                       gauge
control_tick_duration_seconds                   histogram — the scale canary
notifications_sent_total{channel,status}
reservoir_level_percent{reservoir_id}           gauge
backup_last_success_timestamp_seconds           gauge
```

`control_tick_duration_seconds` is the metric that tells you when the
single-loop design needs revisiting.

## Testing strategy

- Integration with 5+ simulators: independent operation; one device's failure
  isolated; simultaneous commands to different devices.
- **SCEN-080** — cross-plant isolation: force every failure mode on plant A
  and assert plant B's state is byte-identical to a control run.
- Shared reservoir: two devices, one tank; assert the lowest reading governs and
  that a low tank locks out both.
- Notification: dedup and rate limiting; channel failure does not delay a tick.
- Deployment: install on a real Pi, reboot, verify recovery; backup and restore
  verified by comparing row counts and a watering-history checksum.
- Load: 20 plants, 10 devices; assert tick duration stays within the period.

## Acceptance criteria

- [ ] 5 simulated devices and 10 plants operate independently.
- [ ] Killing one device locks out only its plants; SCEN-080 shows byte-identical
      state for unaffected plants.
- [ ] `rhizo-provision new` produces working credentials in one command.
- [ ] Provisioning refuses to reuse a `device_id` without `--force`.
- [ ] A leak on one plant produces exactly one notification.
- [ ] A notification channel being down does not delay the control loop
      (asserted by tick duration).
- [ ] The system survives a Pi reboot and resumes automatically.
- [ ] Backup and restore reproduce identical row counts and watering history.
- [ ] Two devices on one reservoir: the lowest reading governs both.
- [ ] 20 plants evaluate within one tick period.
- [ ] The UI remains legible at 20 plants.
- [ ] A `v*` tag produces checksummed downloadable archives with matching versions.
- [ ] The MSRV/current-stable matrix catches an intentional post-MSRV language feature.
- [ ] The complete system and M8 suite work with the observability profile disabled.

## Dependencies

- M12 (a UI that must scale).
- M11 (real hardware worth replicating).
- Hardware: 3+ ESP32 nodes, pumps, sensors, a Raspberry Pi.

## Open questions

1. **Whether per-plant caps should be validated against the device cap** at
   configuration time. Leaning yes, consistent with
   [ADR-011](../adr/011-configuration-and-secrets-model.md)'s reject-don't-clamp
   principle. Decided in M13-006.
2. **Whether the control loop should parallelise** across plants. Not at 20
   plants; `control_tick_duration_seconds` is the signal that would change this.
3. **Notification channel priority.** ntfy is simplest and self-hostable; SMTP is
   most universal; a webhook is most flexible. All three implemented, none
   privileged.

## Future work

- Authentication on the Edge API (needed before any non-trusted network).
- TLS on MQTT with per-device certificates.
- Zones and valves ([PRD 140](140-field-readiness.md)).
- Mobile access via the deferred web frontend.
