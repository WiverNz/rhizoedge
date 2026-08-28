# Issue M9-020 — Implement peripheral power gating and sensor warm-up

**Milestone:** M9 · **PRD:** [PRD 090](../../prd/090-esp32-rust-firmware.md) · **Depends on:** M9-019, M9-005

## Context

A device that sleeps for fourteen minutes out of every fifteen has achieved
nothing if its RS485 transceiver and soil probe stay powered the whole time. On
the reference hardware those two parts dominate the idle draw, and they are
needed for a few hundred milliseconds per cycle.

Powering a sensor also means it is cold when it is first asked for a reading, and
a reading taken before the sensor has settled is not a bad reading — it is a
plausible one, which is worse.

## Goal

Peripherals powered only while sampling, and never sampled before they have
settled.

## Scope

- A `PowerRail` trait with `enable()`, `disable()`, and `is_enabled()`, and fake
  and GPIO implementations, in the style of M9-005's other traits
- Rail pins, polarity, and whether a given rail exists at all supplied by the
  **board profile** (M9-003), never by this module: a load switch that is
  active-low on one board and active-high on another is a board fact, and
  getting it backwards powers a rail through a whole sleep
- Separate rails for the RS485 transceiver and the sensor supply, so a device
  with an analogue probe does not power a transceiver it has no use for
- Rail control in the wake cycle: enable, wait `sensor_warmup_ms`, sample,
  disable — with `disable()` in a guard so an error path cannot leave a rail on
- `sensor_warmup_ms` from configuration, **no compiled-in default for a specific
  sensor part**
- A `sensor_warmup_incomplete` device event when a read is attempted before the
  delay has elapsed, so a misconfiguration is visible rather than silent
- Rails driven off as part of M9-007's boot-safe sequence, alongside the pump
- Rails left **on** in `AlwaysOn` mode, so gating is a battery-mode behaviour and
  not a new failure mode for a mains device

## Non-goals

- Choosing a warm-up value for the SEN0601 or any other part. **It is not
  hardcoded and not guessed**; M10-011 measures it, and until then the
  configuration carries a conservative value marked as unmeasured.
- Any PCB work. Load switches and their part selection are M14-009 planning
  only.
- Measuring what gating saves. M10-012.

## Dependencies

- M9-019
- M9-005

## Implementation notes

A board that has no RS485 rail says so in its own profile, and the sampling code
asks the board rather than assuming. This is the first place the board seam earns
its keep: the DevKitC-02's spare header pins and a XIAO's much smaller pin budget
will not agree on which rails exist, and that disagreement must stay inside
`src/board/`.

The pump is deliberately **not** a `PowerRail`. It is an actuator with a hard
run limit, a boot-safe requirement, and an independent run guard, and giving it
the same interface as a sensor supply invites a refactor that treats them alike.
M9-007's rule — pump GPIO inactive as the first statement in `main` — is
unchanged and stays where it is.

Rails do join the boot-safe sequence, though: an unexpected reset must not leave
a transceiver powered from a battery for two weeks.

Order the sequence so warm-up overlaps with work that has to happen anyway.
Enabling the rail before bringing up Wi-Fi lets association and DHCP run during
the warm-up window instead of after it — the same wall time, less of it awake.
Note the trade-off honestly in the code: it draws the sensor rail and the radio
concurrently, and which ordering wins is an energy question M10-012 settles with
a meter, not a preference to be argued about now.

`sensor_warmup_ms` as configuration rather than a constant is the point of this
issue. A hardcoded stabilisation time is a per-part guess baked into a binary,
and the whole reason M10-011 exists is that nobody knows the real number yet.

## Acceptance criteria

- [ ] Both rails are off during deep sleep, verified by GPIO state before the
      sleep call.
- [ ] A read attempted before `sensor_warmup_ms` has elapsed raises
      `sensor_warmup_incomplete` and does not publish a sample.
- [ ] An error during sampling still disables both rails.
- [ ] Rails are driven off in the boot-safe sequence.
- [ ] `AlwaysOn` mode leaves rails enabled and its behaviour unchanged.
- [ ] No stabilisation constant for any specific sensor part appears in the
      firmware source.
- [ ] A device with no RS485 capability never enables that rail.
- [ ] Rail pins and polarity come from the board profile; `src/app/` and the
      sampling code name no GPIO and no active level.

## Verification

```bash
cd firmware/esp32-node
cargo test --target x86_64-unknown-linux-gnu -p app rails::
cargo build --release
grep -rniE 'sen0601|warmup_ms *[:=] *[0-9]' src/   # expect no hardcoded value
```

## Tests required

- Rail state across a full wake cycle, including every error path.
- Warm-up enforcement and its event.
- Boot-safe rail state.
- Always-on regression.

## Documentation impact

- [PRD 090](../../prd/090-esp32-rust-firmware.md) — the `PowerRail` trait.
- [component-model.md](../../architecture/component-model.md) §10.

## Files likely affected

```text
firmware/esp32-node/src/hal/rail.rs
firmware/esp32-node/src/app/sampling.rs
firmware/esp32-node/src/board/mod.rs
firmware/esp32-node/src/board/devkitc02.rs
```
