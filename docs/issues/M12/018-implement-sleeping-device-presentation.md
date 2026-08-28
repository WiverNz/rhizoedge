# Issue M12-018 — Present sleeping devices and delayed commands

**Milestone:** M12 · **PRD:** [PRD 120](../../prd/120-rust-ui.md) · **Depends on:** M12-015, M12-006, M12-005

## Context

M12-015 built the connectivity views that distinguish cloud offline, site
offline, and device isolated. A battery device adds a fourth condition that is
not a degradation at all: it is asleep, on purpose, and will be back shortly.

Getting this wrong has a specific cost.
[ADR-018](../../adr/018-battery-and-deep-sleep-device-mode.md) rejects "show it
as offline" precisely because a device that shows a red offline badge a hundred
times a day trains its owner to ignore the badge — and that badge is the only
indication that a device has actually died.

## Goal

Three conditions the operator can tell apart at a glance, and a pending dose that
looks pending rather than ignored.

## Scope

- Three visually distinct device states:

  ```text
  Sleeping — next wake expected around 14:45
  Offline unexpectedly
  Syncing offline history…
  ```

- `Sleeping` presented as a normal operating state — not an error colour, not a
  warning icon, not the offline treatment
- `Offline unexpectedly` for an overdue sleeper, carrying the missed-wake count
  and how long it has been overdue
- Last-known readings shown for a sleeping device with their age, in the same
  greyed-with-age treatment M12-010 already uses, since data from four minutes
  ago is normal here rather than suspect
- Watering actions on a battery device: the button explains up front that the
  dose will be delivered at the next wake, and the returned state renders as
  `Pending until device wakes` with the expected delivery time — never as a
  spinner, never as "sent"
- A pending intent visible on the plant view as well as in the command list
- The 409 on a second request rendered as "a dose is already waiting for this
  plant", not as a generic failure
- `expired_before_wake` and `refused` shown in history with their reason
- A monitoring-only battery device rendered as a first-class monitoring plant,
  with no watering control at all — the M12-006 rule, unchanged and re-verified
  for this device class
- Battery voltage charted alongside other measurements, and a `battery_low`
  alert surfaced as a maintenance condition

## Non-goals

- Any control that wakes a device, expedites a dose, or cancels an intent. The
  first two do not exist; the third is an open question in PRD 060.
- Predicting remaining battery life. M13-016 owns trending; this view renders
  what the API gives it.
- Distinguishing battery from mains devices anywhere the distinction does not
  change what the operator should do.

## Dependencies

- M12-015
- M12-006
- M12-005

## Implementation notes

The visual hierarchy carries the safety meaning here, so it is worth stating
rather than leaving to whoever picks the colours. `Sleeping` and
`Offline unexpectedly` must not be variations of one treatment with different
words; they are different situations and one of them needs somebody to go and
look at the plant. Reuse whatever `Reconciling` already does for the "temporary,
expected, no action needed" register, and keep the offline treatment for the
device that is genuinely missing.

"Next wake expected around 14:45" is deliberately approximate in its wording.
The edge's window has a grace period and the device's own timer drifts, so
minute-precision phrasing would be a false claim, and an operator who sees 14:45
pass by a few seconds should not conclude anything is wrong.

The pending-dose affordance is the one place a user could reasonably feel the
interface is broken. Say the latency before they press, not after — "will run at
the next wake, around 14:45" on the button, not a spinner that resolves fifteen
minutes later. The absence of `command_id` in the API response (M6-023) is the
signal to render this path.

Nothing in this view may be a control. It is the same structural claim M12 makes
everywhere: the UI has no MQTT dependency, no `rhizo-domain` dependency, and no
override control, and adding a "wake now" button would need a mechanism that does
not exist.

## Acceptance criteria

- [ ] A sleeping device is visually distinct from an unexpectedly offline one and
      is not presented as a fault.
- [ ] The expected wake time is shown and updates across cycles.
- [ ] An overdue sleeper renders as offline with its missed-wake count.
- [ ] Last-known readings are shown with their age rather than hidden.
- [ ] A dose on a battery device announces the delay **before** it is requested
      and renders as `Pending until device wakes` afterwards.
- [ ] The pending dose is visible from the plant view.
- [ ] A second request renders the 409 as an explanation, not an error.
- [ ] A monitoring-only battery plant shows no watering control at all.
- [ ] No control wakes, expedites, or overrides anything.
- [ ] Battery voltage charts; `battery_low` appears as a maintenance alert.
- [ ] The UI workspace still has no `package.json` and no MQTT dependency.

## Verification

```bash
cd ui/rhizo-ui
cargo test -p rhizo-ui device_state::
cargo test -p rhizo-ui pending_intent::
trunk build --release
grep -rniE 'wake now|force|override|expedite' src/   # expect no matches
```

## Tests required

- Rendering for each of the four device conditions.
- Pending, refused, and expired intent rendering.
- Monitoring-only battery plant shows no watering control.
- Absence of any wake or override affordance.
- SCEN-110, SCEN-113.

## Documentation impact

- [PRD 120](../../prd/120-rust-ui.md) — safety presentation section.
- [connectivity-modes.md](../../architecture/connectivity-modes.md) §7.

## Files likely affected

```text
ui/rhizo-ui/src/views/device.rs
ui/rhizo-ui/src/views/plant.rs
ui/rhizo-ui/src/components/device_state.rs
ui/rhizo-ui/src/components/watering_action.rs
```
