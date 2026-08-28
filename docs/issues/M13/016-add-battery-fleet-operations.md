# Issue M13-016 — Support battery devices in a multi-device home deployment

**Milestone:** M13 · **PRD:** [PRD 130](../../prd/130-multi-plant-home.md) · **Depends on:** M13-007, M13-012, M12-019

## Context

A household with twenty plants will not run twenty mains cables. Some nodes will
be on batteries — on a balcony, a bookshelf, a bathroom windowsill — and those
nodes fail differently from mains ones: they do not stop reporting because
something broke, they stop reporting because nobody replaced a cell.

M12-018 shows one battery device's state. This issue is about a fleet of them,
where the operator needs to be told a battery is going flat **before** the plant
stops being monitored, and needs the notification path and the deployment
documentation to account for devices that are absent by design.

## Goal

Battery devices are supportable at household scale, and a flat battery is
predicted rather than discovered.

## Scope

- Battery-voltage trending per device, reusing M5-005's least-squares trend
  rather than a second implementation, and returning `None` on sparse data for
  the same reason
- A projected depletion date where the trend supports one, explicitly absent
  where it does not
- `battery_low` and `battery_critical` notifications through M13-007's
  dispatcher, coalesced per device so a slow decline does not notify daily
- A `device_wake_missed` notification, distinct from the low-battery one — the
  two have different causes and different remedies
- Fleet views filtered and grouped by power mode, so "which of my nodes need
  attention" is one screen
- Sleeping devices excluded from any "devices offline" count that would otherwise
  make a healthy household look broken
- Backup and restore (M13-008) verified to round-trip power mode, wake interval,
  and pending intents
- Deployment documentation for battery nodes: expected cell type, replacement
  interval derived from M10-012's measured budget, what to check when a node
  stops waking, and how to change a wake interval safely

## Non-goals

- Solar. M14-009 owns outdoor power planning, and a balcony node on a battery is
  supportable without it.
- Any watering behaviour that depends on battery state. Unchanged and forbidden
  ([ADR-018](../../adr/018-battery-and-deep-sleep-device-mode.md) §7): a low
  battery notifies and refuses nothing.
- Remote firmware or power-mode reconfiguration beyond the existing retained
  config path.
- Hardware selection or a recommended battery product.

## Dependencies

- M13-007
- M13-012
- M12-019

## Implementation notes

Prediction has to be honest about its own uncertainty, and a voltage trend on
LiFePO4 is the least honest input available: the discharge curve is nearly flat
across most of its range and then falls quickly. A linear projection over the
flat region will confidently predict a date years away and then be wrong by
weeks. Report a projection only where the trend's confidence supports it, absent
it otherwise, and prefer "voltage falling" to a fabricated date — the same
judgement M5-005 already makes about sparse moisture data, and the same rule
M10-006 applies to an uncalibrated probe.

Coalescing matters more here than for other notifications. A battery crosses a
threshold once and stays across it, so an uncoalesced alert becomes a daily
message that gets muted, which defeats the alert. Notify on the crossing, then on
a material worsening, and not otherwise.

The "devices offline" count is worth getting right because it is the number an
operator glances at. A household with six battery nodes has, at any instant,
roughly six sleeping devices and that is the healthy state. Count sleeping
devices separately or not at all; counting them as offline reproduces at fleet
scale the exact mistake M12-018 avoids per device.

A wake-interval change reaches a device only at its next wake, which makes it a
change with up to one interval of latency and no immediate confirmation. Document
that, and make the UI show desired-versus-applied rather than assuming the change
took — the existing config-drift mechanism (M4-006) already does this and needs
no new machinery.

## Acceptance criteria

- [ ] Battery voltage trends per device, using the existing trend implementation.
- [ ] A projection appears only where the trend supports it and is absent
      otherwise.
- [ ] `battery_low`, `battery_critical`, and `device_wake_missed` dispatch as
      distinct notifications, coalesced per device.
- [ ] A dead notification channel does not delay the control loop (M13-007's
      property, re-verified).
- [ ] Fleet views filter and group by power mode.
- [ ] Sleeping devices are not counted as offline anywhere in the UI or metrics.
- [ ] Backup and restore round-trip power mode, wake interval, and pending
      intents.
- [ ] A wake-interval change surfaces as config drift until the device applies it.
- [ ] Twenty plants including six battery nodes still evaluate within one tick.
- [ ] Battery state affects no watering decision, checked structurally.

## Verification

```bash
cargo test -p edge-controller battery_trend::
cargo test -p edge-controller notifications::battery
cargo test --test integration battery_fleet_at_scale
cargo test --test integration backup_restore_roundtrip
```

## Tests required

- Trend and projection, including the sparse and flat-curve cases.
- Notification coalescing and channel independence.
- Offline-count exclusion of sleeping devices.
- Backup/restore round-trip of the new fields.
- Scale test with a mixed mains and battery fleet.

## Documentation impact

- [PRD 130](../../prd/130-multi-plant-home.md) — battery fleet operations.
- [deployment-model.md](../../architecture/deployment-model.md) §2b.
- Deployment notes for battery nodes.

## Files likely affected

```text
crates/edge-controller/src/device/battery_trend.rs
crates/edge-controller/src/notify/battery.rs
crates/edge-controller/src/api/devices.rs
docs/architecture/deployment-model.md
```
