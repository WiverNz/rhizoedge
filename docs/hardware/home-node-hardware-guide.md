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

### DEVKITM-1 pin allocation (2026-10-23)

**Status:** permanent allocation for the `devkitm1` board profile. Only GPIO5/6/7 are constructed by firmware today; every other row is a reservation that the M10/M11 adapters must use. Concrete numbers live in [`devkitm1.rs`](../../firmware/esp32-node/src/board/devkitm1.rs), which mirrors this table; ADR-007 keeps them there.

Sources, read rather than assumed: ESP32-C3 datasheet v2.4 (Table 2-1 pin settings at/after reset, Table 2-2 power-up glitches, §3 strapping), the [DEVKITM-1 V1 schematic](https://dl.espressif.com/dl/schematics/SCH_ESP32-C3-DEVKITM-1_V1_20200915A.pdf), ESP-IDF v5.5.1 `hal/esp32c3/include/hal/gpio_ll.h` and `esp_adc/Kconfig`, and the TI BQ25185, TPS63070 and INA260 datasheets.

Board facts that decide the plan:

- **Header:** J1 exposes IO2, IO3, IO0, IO1, IO10; J3 exposes TX/RX (UART0), IO9, IO8, IO7, IO6, IO5, IO4, IO18, IO19. GPIO11–17 (in-package flash, VDD_SPI) are not on the header.
- **USB:** the micro-USB connector feeds the CP2102N, which is UART0 (GPIO20/21) with DTR/RTS auto-programming through EN/GPIO9. The direct path to GPIO18/19 goes through R2/R3, which are **not fitted**. Native USB Serial/JTAG is therefore unreachable on this board, and GPIO18/19 are ordinary header pins. ESP-IDF releases the USB pad and its D+ pull-up as soon as either pin is claimed (`gpio_ll_func_sel`).
- **Strapping:** GPIO9 is the BOOT button (weak pull-up, low means download mode). GPIO8 drives the on-board RGB LED through R17 and must be high for download boot. GPIO2 "does not determine" the boot mode but is "recommended" high. No 32 kHz crystal is fitted, and the RTC clock is the internal RC.
- **Reset behaviour:** GPIO6 (MTCK) has a weak (~45 kΩ) pull-up after reset while `EFUSE_DIS_PAD_JTAG` is unburnt. GPIO18 drives a 50 µs high glitch at power-up. GPIO19 carries the USB pull-up until it is claimed. GPIO6, GPIO7 and GPIO10 show only 5 ns low glitches. GPIO5 has neither a pull nor a glitch.
- **ADC:** ADC1 is GPIO0–4. GPIO5 is ADC2, which ESP-IDF refuses on the C3 ("not stable") unless forced, so it is worthless as an analogue input.

| GPIO | Current (before 2026-10-23) | Permanent | Dir | From reset until firmware configures it | External pull expected | Safety-critical | Why this pin |
|---|---|---|---|---|---|---|---|
| 5 | pump | **pump driver input**, active high | out | input, no pull, no glitch | **10 kΩ pull-down, mandatory** (SAFETY-011, F-110-03) | **yes** | The only boot-quiet pin whose sole analogue function (ADC2) is unusable anyway. It shares nothing. HIL-1 and the docs already name it. |
| 6 | sensor rail EN | sensor rail EN (unchanged) | out | input **with weak pull-up** | 10 kΩ pull-down (beats ~45 kΩ, leaving ≈0.6 V) | no (power) | Already wired. The pull-down keeps the rail off through boot and sleep. |
| 7 | RS485 rail EN | RS485 rail EN (unchanged) | out | input, no pull | 10 kΩ pull-down | no (power) | Already wired. |
| 10 | free | **available**: no peripheral claims it; candidate future SCL alternative (see the OLED note) | — | input, no pull, 5 ns low glitch | — | no | Not ADC and not strapping. Deliberately left unreserved. |
| 18 | free (incorrectly "reserved for USB") | **UART1 TX → DFR0845 RS485 module** | out | 50 µs **high** glitch | — | no | TX idles high, so the glitch is a legal idle level. It is also why 18 must never be an enable. |
| 19 | free | **UART1 RX ← DFR0845 RS485 module** | in | USB D+ pull-up until claimed | — | no | RX idles high, so the pull-up is harmless. Not on 18, because its glitch would fight the module's UART output. |
| 3 | free | **I2C SDA** | open-drain | input, no pull | **4.7 kΩ to 3V3 on the node** | no | Not strapping, which SDA must not be. It costs ADC1 ch3, the price of an I2C bus. It is next to GPIO2 on J1 (pins 5 and 4), so the bus is one short adjacent pair. |
| 2 | free | **I2C SCL** | open-drain | floating | **4.7 kΩ to 3V3 on the node, required** | no | The current verified choice, not forced: GPIO10 is a non-strapping alternative (see the OLED note). Only the master drives SCL, and neither the INA260 nor the SH1106 stretches the clock, so nothing can hold it low at reset. The pull-up raises GPIO2 from floating to the recommended high level. Verified on the board (see the OLED section). |
| 0 | free | **leak input** | in | floating | pull-up/conditioning per the chosen sensor | **yes** (refusal input) | ADC1 ch0 and deep-sleep-wake-capable (GPIO0–5), so a sleeping node can later wake on a leak (F-110-31). Not strapping, so a wet sensor at reset cannot change the boot mode. |
| 1 | free | **tank input** | in | floating | pull-up for a float switch | **yes** (refusal input) | ADC1 ch1, so a resistive tank sensor (F-110-20) needs no rewiring. Not strapping, so a closed float at reset cannot change the boot mode. |
| 4 | free (OLED SDA on the bench 2026-10-23, before the move) | **analogue soil probe** (F-100-05) | in (ADC) | input, no pull | — | no | ADC1 ch4. PRD 100 requires that Modbus ↔ analogue be a configuration change rather than a rebuild, so this pin must exist in the same profile as RS485. |
| 8 | on-board RGB LED | on-board RGB LED / status only | out | floating | fitted on board | no | Strapping pin wired to the LED by the board. Nothing external. |
| 9 | BOOT button | BOOT button only | in | weak pull-up | fitted on board | no | A low level at reset selects download mode, so never connect a sensor here. |
| 20 / 21 | UART0 console | UART0 console, provisioning, `espflash` | — | — | fitted on board | no | CP2102N. Do not reuse. |
| 11–17 | flash | flash | — | — | — | — | Not exposed. |

**Kept free:** GPIO10. Pump, two rail enables, RS485 TX/RX, I2C SDA/SCL, leak, tank and analogue soil make 10 signals. With SCL on GPIO2, nine of them use the 10 pins that are neither strapping, flash, console nor LED, leaving GPIO10 unclaimed. GPIO8 (LED), GPIO9 (BOOT) and 20/21 (console) stay as they are. GPIO10 is not reserved for anything.

**Expansion is on the I2C bus, not on pins.** Addresses in use or reserved:

| Address | Device | Role |
|---|---|---|
| 0x3C | SH1106 OLED | bench accessory only (below) |
| 0x40 | INA260 with A1 = A0 = GND (0x40–0x4F selectable) | power telemetry, including battery voltage (0–36 V bus input), which replaces an ADC divider and its pin |
| 0x2A | NAU7802 (proposal) | the M16 reservoir load cell. An HX711 needs two GPIOs this board does not have. |
| 0x20–0x27 | TCA9534-class expander (proposal) | non-safety status lines, e.g. charger STAT1/STAT2 or regulator PG. Avoid PCF8574A, whose 0x38–0x3F range contains 0x3C. |

Leak and tank stay on direct GPIOs and are never moved behind the expander. Their I2C failure mode (one stuck slave blinds every device on the bus) would still fail closed as `Unknown`, but a direct pin also keeps leak-during-dose within F-110-33's one second without depending on the bus.

**Power and battery parts need no GPIO.** The BQ25185 (`CE` tied low enables charging; STAT1/STAT2 are open-drain and "can be left floating"), the TPS63070 (`EN` and `PS/SYNC` strapped; PG is open-drain and optional) and the INA260 (ALERT optional) all work with no MCU pin. Any status signal that is wanted goes through the expander as telemetry, because power is never a safety input (ADR-018 §7). The solar panel needs none. **Discrepancy:** the BQ25185 is a *1-cell* charger (it supports 1S LiFePO4 at 3.6 V), while this guide's battery node is a 12.8 V **4S** LiFePO4 pack. Neither part is selected in this repository, and choosing between those architectures is M14 work.

**Limitations this plan records rather than hides:**

- A two-pin ultrasonic tank sensor (trigger/echo) does not fit. Use an I2C or analogue-output variant, or the float/resistive options PRD 110 already lists.
- The RS485 interface is the **DFRobot DFR0845 Active Isolated RS485 to UART** module. Its MCU side is UART TX/RX only and exposes no DE or /RE control pin, so no GPIO is allocated for direction control. PRD 100's F-100-06, M10-002 and M10-008 were amended on 2026-10-23 to match: no DE/RE GPIO and no firmware turnaround timing on this build. It is not hot-pluggable (see the M10 section).
- While the RS485 rail is off, firmware must not leave TX high into the unpowered module, or it back-powers it through its input clamp. This is an M10 adapter concern; the current sleep configuration isolates the pads.
- ESP-IDF's secondary console on USB Serial/JTAG (`CONFIG_ESP_CONSOLE_SECONDARY_USB_SERIAL_JTAG=y`, the default) should be disabled when M10 claims GPIO18/19. It is harmless but writes to a pad that is no longer USB.
- A different board (XIAO, Super Mini, custom) gets its own table. Nothing here transfers automatically.

### SH1106 OLED: development accessory, verified 2026-10-23

Verified on the official DEVKITM-1-N4X (chip rev v1.1) with `firmware/esp32-node/examples/oled_probe.rs`, a 1.3-inch white 128×64 SH1106 I2C module with pin order `VCC GND SCK SDA`:

- **Address 0x3C**, the only ACK in a 0x08–0x77 scan at 100 kHz. The `sh1106` 0.5.0 driver (page addressing, 2-column offset for 132-column RAM) initialised it and drew a full border and text with no offset error. That is the SH1106 behaviour, not an SSD1306 assumption.
- First bench wiring used SDA = GPIO4, SCL = GPIO5. **That wiring is withdrawn**: GPIO5 is the pump output, and the module's SCK pull-up would fight the pump's mandatory pull-down through every boot. Never run the OLED on GPIO5 with a pump driver connected.
- **Current wiring:** OLED `SDA → GPIO3` (J1 pin 5), OLED `SCK → GPIO2` (J1 pin 4), `VCC → 3V3`, `GND → GND`. GPIO3 and GPIO4 are electrically equivalent for SDA; GPIO3 was chosen so the pair sits on adjacent header pins.
- **Verified on the current pins:** ACK at 0x3C, init and flush OK, and the same screen confirmed by eye. Across five EN resets, the ROM reported `boot:0xf (SPI_FAST_FLASH_BOOT)` every time and the display re-initialised. The ROM strap byte was `0xc` with GPIO2/3 unconnected, `0xd` with SCK on GPIO2, and `0xf` with SDA also on GPIO3. ESP-IDF's `soc/esp32c3/include/soc/boot_mode.h` makes fast flash boot depend on bit 3 alone (GPIO9, `IS_1XXX`), so the bus lines move only don't-care bits. A true supply interruption and a reset during an I2C transfer were not tested. The node image (`devkitm1-VIRTUAL-ACTUATOR-BENCH`) also booted normally with the display attached.

> **Note — status of the GPIO2/GPIO3 choice.** GPIO2/GPIO3 is the current verified I2C allocation on the real ESP32-C3 board. GPIO2 is a strapping pin, so this should not be described as an unconditionally ideal permanent choice. The current wiring has been verified to boot and operate correctly with the OLED attached. GPIO10 remains a possible alternative SCL pin if future expansion or production hardening requires avoiding a strapping pin.
>
> - Do not change the current OLED wiring now.
> - GPIO10 is documented as a future alternative, not an immediate migration. It is currently available: the RS485 interface (DFR0845) needs only UART TX/RX, and no other peripheral claims GPIO10.
> - This creates no requirement to reserve GPIO10 for I2C unless a future hardware need actually appears.
> - The OLED remains a temporary bench/debug accessory, excluded from battery-life, deep-sleep and solar power budgets (below).

**Power-budget treatment.** The OLED is a bench and debug accessory and is **not part of the production battery hardware profile**:

- Exclude it from battery-life estimates, deep-sleep budgets, solar sizing, average-current targets, ≥6-month autonomy calculations and low-power acceptance criteria, including [M10-012](../issues/M10/012-measure-system-energy-budget.md). Reserve no battery margin for it. A powered SH1106 draws non-trivial current, and breakout boards add pull-ups and regulators that keep drawing while it is blank.
- Production firmware must not depend on it. Its absence or failure must never affect monitoring, safety logic, pump control, offline autonomy or recovery. No firmware in the image constructs it today.
- If it is ever kept as a product option, it must be explicitly optional, its power must be measured separately on real hardware, battery and solar figures must be computed both with and without it, and power-gating it should be considered. Power-gating must prevent SDA/SCL from back-powering an unpowered module, and the module must never be powered from a GPIO.

## M10 — real soil sensor

M10 adds the selected physical soil path and proves calibration, stabilization, failure handling, and complete-system energy.

| Purpose | Recommended development item/type | Qty | Approx. dimensions | EUR | Bench or final | Reused by |
|---|---|---:|---|---:|---|---|
| Moisture/temperature/EC input | **DFRobot SEN0601** or selected RS485 Modbus soil probe | 1 | probe roughly 140–150 mm long; verify revision | €35–55 | Both | M11–M14 |
| UART/RS485 interface | **DFRobot DFR0845** Gravity: Active Isolated RS485 to UART (selected for the reference build) | 1, a spare optional | verify on the product page | verify current listing | Both | M11–M14 |
| Sensor supply independent of USB | Current-limited bench supply or certified 12 V / 2 A adapter | 1 | external | €12–30 | Bench | M11 |
| Secure field wiring | Twisted pair, terminals, ferrules or probe-matching waterproof pigtail | 1 set | as needed | €8–20 | Both | M11–M14 |
| Calibration reference | Suitable digital scale, soil containers and labels | 1 set | benchtop | €15–35 | Shared bench | M11 |
| Cold-start/current fixture | Reuse M9 instruments; add shunt/fixture if needed | 1 | small | €3–10 | Shared bench | M11–M14 |

**M10 increment: €72–145; M9 + M10 sensor bench: €224–461.**

```text
DEVKITM-1-N4X UART1 TX/RX, VCC, GND ── DFR0845 (isolated; no DE/RE pin) ── twisted A/B + bus-side reference ── probe
12 V probe supply ──────────────────────────────────────────────────────────────────────────────────────────── probe
```

The DFR0845's MCU side is **TX, RX, GND and VCC only**. The module switches RS485 direction itself, so the ESP32 drives ordinary UART TX/RX, no DE or /RE GPIO is wired, there is no firmware turnaround timing (F-100-06), and **GPIO10 stays free and unreserved**. Because the module is isolated, the probe's signal reference goes to the module's bus side, not to ESP32 GND; confirm terminal labels against DFRobot's documentation.

**Do not hot-plug or disconnect/reconnect the DFR0845 while powered.** DFRobot documents the module as not hot-pluggable. Rewire only with all power removed. To test a lost probe (PRD 100 / M10-013), remove the probe's own 12 V supply instead of pulling the module or its wiring. Power-gating the module through the RS485 rail (GPIO7) is a supply cycle rather than a hot-plug, but it carries the same back-powering hazard; M10 must confirm it against DFRobot's documentation, and UART1 TX/RX stay inactive while the rail is off until M10-012 has measured the back-powering behaviour.

**Reference-build electrical rule (DFR0845 ↔ ESP32-C3).** Power the DFR0845's UART/MCU side (VCC) from **3.3 V**. The module accepts a 3.3–5 V MCU-side supply, its isolated UART transceiver works at 3.3 V or 5 V logic, and its RXD output-high follows that logic-side supply. A 3.3 V supply is therefore what makes a direct connection safe. Under that condition only, connect the module's `T/TX` and `R/RX` directly to ESP32 UART1 (GPIO18 TX, GPIO19 RX), crossed as DFRobot's wiring diagram shows. **Never power the UART side from 5 V and feed its RXD/T output directly into an ESP32 GPIO**: the output would then swing to 5 V, beyond the ESP32-C3's 3.3 V I/O. The isolated RS485-side ground stays separate from ESP32 GND, as above.

Verify the purchased probe's wire colors, polarity, supply, Modbus address/baud rate, and register map before power-up.

Measure repeated reference points, repeatability, cold-start time until the selected stability criterion is met, active current, power-gated leakage, and full sampling-cycle energy. Stabilization time is **TBD until measured**. Report separately:

- ESP32 chip deep-sleep current at an appropriate supply boundary; and
- complete node/system use, including board overhead, the DFR0845 RS485 module (measured on the real module: active, idle, rail-disabled, and any back-power through UART TX/RX, per M10-012; never a transceiver-IC datasheet figure), probe, radio association, MQTT exchange, and wake duration.

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
- exact probe revision/vendor, connector, and RS485 bus termination/biasing needs (the RS485 module itself is selected: DFR0845);
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
