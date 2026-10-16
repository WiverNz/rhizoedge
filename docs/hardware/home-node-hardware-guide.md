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

### Optional SH1106 OLED: compatibility investigation

**Reviewed 2026-10-15; source/documentation inspection only, not hardware validation.** An optional 1.3-inch SH1106 128×64 I2C module specified for 3.3 V can provisionally use **SDA = GPIO0, SCL = GPIO1, VCC = 3V3, GND = GND** on the official ESP32-C3-DEVKITM-1-N4X. This is a bench wiring option, not an implemented feature or a reserved production pin map. Confirm the actual module's supply requirements, pin order, and pull-ups before connecting with power removed.

Verified against the current repository:

- The [board profile](../../firmware/esp32-node/src/board/devkitm1.rs) constructs GPIO5 as active-high pump control, GPIO6 as sensor supply enable, and GPIO7 as RS485 supply enable. GPIO0/1/3/4/10 are unused by the application. GPIO7 is a supply enable, not RS485 DE/RE.
- The [console adapter](../../firmware/esp32-node/src/hal/console.rs) uses ESP-IDF's configured console. On this board, the CP210x bridge connects to UART0 GPIO20/21; native USB Serial/JTAG uses the separate GPIO18/19 pair. Preserve both interfaces when allocating optional pins.
- The [dependency manifest](../../firmware/esp32-node/Cargo.toml) pins `esp-idf-hal` 0.46.2, which provides `I2cDriver` and embedded-hal I2C implementations; the [SDK configuration](../../firmware/esp32-node/.cargo/config.toml) pins ESP-IDF v5.5.1. No I2C bus or SH1106 display initialization is implemented. Connecting the display alone will not produce useful output. The pinned HAL uses the legacy ESP-IDF I2C API; do not mix it with the new I2C driver on a later implementation.
- The [sleep adapter](../../firmware/esp32-node/src/hal/sleep.rs) enables timer wake, not GPIO wake, and has no OLED shutdown or power-control handling. [Battery ADC support](../../firmware/esp32-node/src/hal/battery.rs) is an uninstantiated adapter on this board.

Espressif's [board pinout](https://docs.espressif.com/projects/esp-idf/en/v4.4/esp32c3/hw-reference/esp32c3/user-guide-devkitm-1.html#header-block) exposes IO0 at J1 pin 9 and IO1 at J1 pin 10. Neither is a strapping pin: ESP32-C3 strapping pins are GPIO2/8/9. The [board schematic, sheet 2](https://dl.espressif.com/dl/schematics/SCH_ESP32-C3-DEVKITM-1_V1_20200915A.pdf) shows automatic programming through EN/GPIO9. Thus GPIO0/1 I2C pull-ups do not select boot/download mode or overlap UART0/flash. GPIO0/1 also serve ADC1 channels 0/1 and the external 32.768 kHz crystal pair; these functions cannot be used simultaneously with the OLED bus. The [GPIO reference](https://docs.espressif.com/projects/esp-idf/en/v5.5.1/esp32c3/api-reference/peripherals/gpio.html) confirms peripheral routing through the GPIO matrix and deep-sleep wake capability on GPIO0–5.

**Future allocation remains open.** [M10 Modbus](../issues/M10/002-implement-modbus-rtu-client.md) needs UART and direction control; [PRD-100](../prd/100-real-soil-sensor.md) also defines an analog soil alternative; [PRD-110](../prd/110-real-pump-and-safety-hardware.md) needs tank/leak inputs. Battery sensing and the later [PRD-160 reservoir load cell](../prd/160-verified-watering.md) add possible requirements. None assigns these final GPIOs. Preserving the current outputs, UART, native USB, and avoiding strapping pins leaves five spare pins (0/1/3/4/10); an OLED leaves three. Conventional RS485 TX/RX/direction plus separate tank/leak inputs already needs five. This pin-budget inference means the complete node needs an allocation/interface review regardless of the OLED pair.

For a removable bench display, GPIO0/1 is reasonable. **GPIO3 SDA + GPIO10 SCL is a provisional alternative** that preserves the crystal pair and more ADC/deep-sleep-wake-capable pins. It is not a committed assignment and does not resolve the overall pin shortage. Future boards need their own map under [ADR-007](../adr/007-esp32-rust-framework-and-toolchain.md).

Electrical and power conditions still requiring validation:

- Pull SDA/SCL up to **3.3 V only**. Inspect existing module resistors before adding more. For short wiring, about 4.7 kΩ is a starting point, subject to capacitance and measurement; see [ESP-IDF I2C guidance](https://docs.espressif.com/projects/esp-idf/en/v5.5.1/esp32c3/api-reference/peripherals/i2c.html). Start conservatively at 100 kHz when support is implemented.
- Board 3V3 remains powered during deep sleep. A blank OLED is not evidence of negligible consumption; controller, charge pump, and module circuitry still matter. Firmware currently neither commands display-off nor switches OLED power.
- Future power gating must prevent SDA/SCL from back-powering an unpowered OLED and coordinate pull-ups, GPIO state, and supply sequencing. Do not power the module from a GPIO. A line held low through 4.7 kΩ draws approximately 0.70 mA at 3.3 V (calculated, not measured).
- Exact OLED active/standby current, pull-up arrangement, supply margin during Wi-Fi peaks, and complete-system sleep current remain unverified. Include the display in [M10-012 energy measurements](../issues/M10/012-measure-system-energy-budget.md) if fitted.

**Conclusion:** GPIO0/1 has no identified current firmware conflict, conditional on correct 3.3 V wiring. Connection alone is expected to leave application logic unchanged, but changes electrical loading and sleep consumption. Display operation requires a separately authorized firmware change and hardware verification.

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
| Cable entries | Sized IP68 glands/plugs and hydrophobic membrane vent | 1 set | M12/M16 *gland thread sizes* (not milestones); verify cables | €10–22 | Final | Solar |
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
