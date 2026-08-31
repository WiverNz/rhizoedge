# Rhizo Edge — Home Node Hardware Guide

**Status:** practical procurement and assembly guide

**Price snapshot:** 2026-08-31, Germany/EU; approximate retail ranges, excluding shipping

**Authority:** practical guidance only; PRDs, ADRs, protocols, and safety invariants remain normative

This guide follows the milestones in which hardware becomes necessary and separates shared bench equipment from final-node parts.

The MCU remains ESP32-C3. The primary M9 board is the **official Espressif ESP32-C3-DEVKITM-1-N4X**. The older **ESP32-C3-DevKitC-02 is obsolete for this project and is not a recommended new purchase**. Generic boards with similar names are not official Espressif boards unless their manufacturer and exact revision are traceable. ESP32-C3 Super Mini and Seeed Studio XIAO ESP32-C3 boards are compact secondary/production candidates, not the M9 reference board.

All concrete GPIO assignments, polarity, and peripheral construction stay in `board.rs` (or files wholly owned by that board layer). Application, safety, MQTT, sensor, and pump logic must not depend on the board.

## Cost summary

| Build point | Approximate total |
|---|---:|
| M9 development bench, including measuring instruments | **€152–316** |
| M9 + M10 sensor bench | **€224–461** |
| Complete M11 watering prototype | **€304–641** |
| Eventual battery-powered home node, excluding shared instruments/Edge host | **€190–330** |
| Optional solar variant | **€255–460** |

Bench totals assume the DMM and current/energy profiler must be bought. Final-node totals exclude development instruments. None of these totals implies demonstrated battery autonomy.

## M9 — ESP32 firmware foundation

M9 establishes trustworthy behavior on real silicon. A real ESP32 board is required to honestly complete M9 hardware verification; compilation and simulation do not prove flashing, boot safety, radio behavior, NVS, sleep, or wake cycles.

| Purpose | Recommended development item/type | Qty | Approx. dimensions | EUR | Bench or final | Reused by |
|---|---|---:|---|---:|---|---|
| Primary MCU and radio | **Official Espressif ESP32-C3-DEVKITM-1-N4X**, headers fitted | 1 | about 60 × 25 mm | €10–18 | Bench | M10–M12 |
| Flashing, serial and power | Known-good USB **data** cable matching the board; not charge-only | 1 | 1–2 m | €4–10 | Bench | M10–M12 |
| Accessible temporary wiring | 830-point solderless breadboard | 1 | about 165 × 55 mm | €5–10 | Bench | M10–M11 |
| Signal wiring | Male–male and male–female jumper kit | 1 set | 10–20 cm | €5–10 | Bench | M10–M11 |
| Basic GPIO proof | LEDs, 220–1,000 Ω and 10 kΩ resistors, pushbuttons | 1 kit | small parts | €3–8 | Bench | M11 fake-load tests |
| Voltage, continuity and spot current | Fused DMM with µA/mA/A ranges | 1 | handheld | €20–60 | Shared bench | M10–M14 |
| USB input checks | USB power meter; data pass-through where needed | 1 | inline | €15–40 | Shared bench | M10–M12 |
| Sleep/wake waveform and energy/cycle | Current/energy profiler supporting ESP32 radio peaks and µA sleep; PPK2/Joulescope-class | 1 | bench instrument | €90–160 | Shared bench | M10–M14 |

**M9 bench total: €152–316.** A €12–30 logic analyzer is useful but optional and excluded.

```text
PC ── USB data cable ── official DEVKITM-1-N4X ── breadboard LED/button/test points
power source/profiler ───────── measured supply path ─────────┘
```

Keep the antenna clear of wire bundles and metal. Record evidence for initial and repeat flashing; serial logs and provisioning; Wi-Fi association; MQTT connect/LWT/reconnect; NVS retention across reboot and power removal; reset/boot GPIO fail-off; deep-sleep entry; timed wake and repeated wake cycles; and board/system active, sleep, and wake-cycle energy.

Chip deep-sleep current is not USB board current: regulators, indicators, and USB circuitry contribute to complete board/system consumption. Every unmeasured power figure is **target/TBD**.

## M10 — real soil sensor

M10 adds the selected physical soil path and proves calibration, stabilization, failure handling, and complete-system energy.

| Purpose | Recommended development item/type | Qty | Approx. dimensions | EUR | Bench or final | Reused by |
|---|---|---:|---|---:|---|---|
| Moisture/temperature/EC input | **DFRobot SEN0601** or selected RS485 Modbus soil probe | 1 | probe roughly 140–150 mm long; verify revision | €35–55 | Both | M11–M14 |
| 3.3 V UART/RS485 interface | MAX3485/SP3485-class module explicitly compatible with 3.3 V logic | 2, one spare | typically 20–45 mm | €4–10 | Both | M11–M14 |
| Sensor supply independent of USB | Current-limited bench supply or certified 12 V / 2 A adapter | 1 | external | €12–30 | Bench | M11 |
| Secure field wiring | Twisted pair, terminals, ferrules or probe-matching waterproof pigtail | 1 set | as needed | €8–20 | Both | M11–M14 |
| Calibration reference | Suitable digital scale, soil containers and labels | 1 set | benchtop | €15–35 | Shared bench | M11 |
| Cold-start/current fixture | Reuse M9 instruments; add shunt/fixture if needed | 1 | small | €3–10 | Shared bench | M11–M14 |

**M10 increment: €72–145; M9 + M10 sensor bench: €224–461.**

```text
DEVKITM-1-N4X UART TX/RX + DE/RE ── 3.3 V MAX3485/SP3485 ── twisted A/B ── probe
                         GND ───────── common reference ────────────────────┘
12 V sensor supply ────────────────────────────────────────────────────────┘
```

Verify the purchased probe's wire colors, polarity, supply, Modbus address/baud rate, and register map before power-up. Never connect a 5 V-only MAX485 receiver output directly to ESP32 GPIO.

Measure repeated reference points, repeatability, cold-start time until the selected stability criterion is met, active current, power-gated leakage, and full sampling-cycle energy. Stabilization time is **TBD until measured**. Report separately:

- ESP32 chip deep-sleep current at an appropriate supply boundary; and
- complete node/system use, including board overhead, transceiver, probe, radio association, MQTT exchange, and wake duration.

Only the latter supports battery sizing.

## M11 — real irrigation hardware

M11 proves calibrated physical delivery and fail-off behavior through reset, faults, and lockouts.

| Purpose | Recommended development item/type | Qty | Approx. dimensions | EUR | Bench or final | Reused by |
|---|---|---:|---|---:|---|---|
| Water delivery | 12 V-class peristaltic pump with replaceable tube and published rating | 1 | commonly 65–100 × 35–60 × 35–60 mm | €12–30 | Both | M12–M14 |
| Pump-only source | Certified DC supply sized from pump startup/run measurements | 1 | external | €15–35 | Bench | HIL |
| Inductive-load control | Logic-level MOSFET/driver verified at 3.3 V, with voltage/current headroom and flyback protection | 2, one spare | small module/PCB | €6–16 | Both | M12–M14 |
| Hardware fail-off | 10 kΩ gate pull-down and independent pump-power cutoff | 1 each | small | €3–8 | Both | M12–M14 |
| Water path | Pump-matched tubing, fittings/clips; optional anti-siphon valve | 2–3 m/set | ID/OD selected with pump | €6–15 | Both | M12–M14 |
| Bounded source | Lidded 0.5–1.0 L reservoir | 1 | about 100–150 mm per side | €4–10 | Both | M12–M14 |
| Low/empty lockout | Float switch with testable failure state | 2, one spare | typically 40–80 mm | €4–12 | Both | M12–M14 |
| Leak lockout/interrupt | Wired NO/NC or relay-output leak detector | 1 | model dependent | €15–30 | Both | M12–M14 |
| Delivery calibration | Graduated container and/or reuse M10 scale | 1 | 250–1,000 ml | €3–10 | Shared bench | Maintenance |
| Serviceable protection/wiring | Fuses/holders, terminals, stranded wire, ferrules, crimp allowance | 1 set | as needed | €12–25 | Both | M12–M14 |

**M11 increment: €80–180; complete M11 prototype: €304–641 including M9/M10 instruments.**

```text
ESP32 GPIO ─ driver input ─ 10 kΩ pull-down to GND
pump PSU + ─ fuse ─ cutoff ─ pump ─ MOSFET ─ PSU −
                              └──── flyback protection
reservoir/float ─ inlet ─ pump ─ outlet ─ measuring vessel
                                  leak detector below wet path
```

The pump supply is separate from board power. Select it after checking pump voltage, startup current, run current, and wiring. Keep the pump and tube joints below/away from electronics.

Calibrate several timed deliveries into the measuring vessel/scale; advertised flow is not calibration. Flow, variance, pump current, and dose energy are **TBD until measured**. HIL evidence covers boot/reset/watchdog fail-off, command limits, independent run guard, empty-tank refusal, leak-before-dose refusal, leak-during-dose cutoff, power interruption, and repeated full cycles.

## M12 — desktop UI

M12 requires **no new plant-node hardware**. Reuse the M9–M11 prototype to demonstrate real state, measurements, configuration, sleeping/offline presentation, lockouts, calibration state, and watering through Tauri/Leptos. The UI remains an Edge REST client and never talks directly to MQTT or hardware. Incremental plant-node cost: **€0**.

## M13 — multi-plant/home deployment

| Purpose | Recommended item/type | Qty | Approx. dimensions | EUR | Bench or final | Reused by |
|---|---|---:|---|---:|---|---|
| Additional nodes | Verified ESP32-C3 board profile; official DEVKITM-1-N4X for development, verified XIAO/Super Mini/custom board for compact installation | 1 per plant/zone | board dependent | €5–18 each | Final | M14 |
| Always-on Edge | Raspberry Pi 4/5-class or reliable small x86 host with supported storage | 1 | Pi about 90 × 60 mm | €70–180 | Home infrastructure | M14 |
| Reliable Edge power | Reputable correctly sized PSU; UPS optional | 1 | external | €12–35 | Home infrastructure | M14 |
| Network | Existing router/AP; Ethernet, switch or extra AP where survey requires | as needed | site dependent | €0–100 | Home infrastructure | M14 |
| Serviceable housing | Location-appropriate enclosure, mounting plate, terminals, strain relief, labels | 1/node | about 300 × 200 × 150 mm if battery housed | €25–55 each | Final | M14 |
| Node power | Certified adapter or measured LiFePO4 system | 1/node | source dependent | €15–80 each | Final | M14 |

Replace Dupont wires and breadboards with crimped/soldered assemblies, pluggable terminals, strain relief, keyed connectors, branch fuses, labels, mounting hardware, and accessible pump cutoffs. Route RS485 as twisted pair, separate wet tubing from electronics, add drip loops, secure reservoirs, document pinouts, survey Wi-Fi at every position, and back up the Edge host.

## M14 / future battery, solar, and field readiness

Battery and solar are future field-readiness work, not an M9 promise. Size them from measured M9/M10/M11 consumption and real wake cycles.

| Purpose | Recommended item/type | Qty | Approx. dimensions | EUR | Bench or final | Reused by |
|---|---|---:|---|---:|---|---|
| Compact controller | Verified XIAO ESP32-C3, specific Super Mini, or custom ESP32-C3 PCB | 1 | XIAO about 21 × 18 mm; Super Mini varies | €5–12 | Final | Field use |
| Stored energy | Traceable 12.8 V LiFePO4 battery with BMS and temperature strategy | 1 | 6 Ah class about 151 × 65 × 94 mm | €35–55 | Final | Solar |
| Wall charging | Certified 4S LiFePO4 charger matched to battery limits | 1 | external | €15–30 | Service bench | Solar backup |
| Always-on conversion | Wide-input, low-quiescent-current regulator sized for radio peaks | 1 | module/PCB | €6–15 | Final | Solar |
| Switched sensor rail | Default-off high-side switch rated above maximum charging voltage | 1 | module/PCB | €3–8 | Final | Solar |
| Pump rail | Regulated/switched supply if pump cannot tolerate full battery voltage | 1 | module/PCB | €5–15 | Final | Solar |
| Protection | IP66/IP67 enclosure with mounting plate | 1 | target about 300 × 200 × 150 mm | €25–55 | Final | Solar |
| Cable entries | Sized IP68 glands/plugs and hydrophobic membrane vent | 1 set | M12/M16 typical; verify cables | €10–22 | Final | Solar |
| Power/service hardware | Main/branch fuses, cutoff, charge connector, terminals, strap, wire/ferrules | 1 set | site dependent | €20–40 | Final | Solar |
| M10/M11 hardware | Probe/RS485, pump/driver, sensors, tubing and reservoir | 1 set | as above | €66–133 | Final | Solar |

**Battery home node: €190–330**, excluding shared instruments and Edge host.

```text
LiFePO4 + ─ main fuse ─┬─ low-Iq regulator ─ controller
                       ├─ default-off sensor rail ─ RS485 + probe
                       └─ cutoff + switched/reg. pump rail ─ driver + pump
```

### Optional solar add-on

| Purpose | Recommended item/type | Qty | Approx. dimensions | EUR | Location |
|---|---|---:|---|---:|---|
| Generation | 20 W-class monocrystalline 12 V-nominal panel as initial candidate | 1 | commonly about 450 × 350 mm; verify | €25–50 | Installation |
| LiFePO4 charging | Controller explicitly supporting 4S LiFePO4, correct limits, low self-consumption, temperature strategy | 1 | model dependent | €30–60 | Final enclosure/adjacent |
| Outdoor connection | UV-resistant cable, glands/connectors, fuse and mounting bracket | 1 set | site dependent | €10–20 | Installation |

**Solar increment: €65–130; complete solar variant: €255–460.**

Battery autonomy is **target/TBD, never guaranteed**. Calculate it from measured complete-system sleep power, wake frequency, sensor stabilization energy, Wi-Fi/MQTT energy, watering frequency and pump energy, conversion loss, usable battery capacity under intended conditions, and safety margin. Solar additionally needs site/season generation and controller self-consumption. Never describe autonomy as unlimited.

## Procurement sequence

1. Buy the official DEVKITM-1-N4X and M9 equipment; complete real-board verification.
2. Add probe, 3.3 V RS485, supply, and calibration fixtures; measure stabilization and sampling energy.
3. Add pump, separate supply, fail-off driver, reservoir/tubing and safety sensors; calibrate and run HIL tests.
4. Reuse the prototype for M12; no new plant hardware.
5. For M13, duplicate only verified assemblies and replace temporary wiring with maintainable installation hardware.
6. Select battery hardware after M9–M11 measurements; select enclosure holes after measuring actual cables.
7. Add solar last, after the battery node works independently and a site-specific energy budget exists.

## Exact selections still open

- compact production board and its measured sleep/GPIO behavior;
- exact probe revision/vendor, connector, and RS485 module/termination needs;
- current/energy profiler and high-current fixture;
- pump, tubing, driver/flyback implementation, pump supply, tank sensor and leak detector;
- regulator, sensor switch, optional pump rail, battery, charger, enclosure, glands, vent, connectors, fuses and wire gauges;
- solar panel/controller and mounting for the actual site.

Sensor warm-up, calibration, pump flow/current, chip sleep current, complete system consumption, wake energy, battery autonomy, and solar margin remain **TBD until physically measured**.

## Safety rules

- Keep mains outside the node and use certified low-voltage supplies/chargers.
- Never power the pump from the ESP32 board.
- Pump OFF is the electrical default via hardware pull-down and independent cutoff.
- Failed/unknown tank or leak input refuses watering.
- Fuse battery positive near the battery and size protection from actual load/conductor limits.
- Keep wet parts below and away from electronics; use drip loops and strain relief.
- Observe LiFePO4 charging temperature limits.
- Before unattended use, test leak, empty tank, reset, watchdog, power loss, and repeated doses into a safe vessel.
