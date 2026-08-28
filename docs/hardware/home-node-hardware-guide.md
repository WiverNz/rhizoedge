# Rhizo Edge — Home Node Hardware Guide

**Status:** practical hardware guide  
**Target:** one indoor plant node, expandable to battery/solar deployment  
**Price snapshot:** 2026-08-28, Germany/EU; prices are approximate and exclude shipping unless stated  
**Related milestones:** M9 ESP32 firmware, M10 real soil sensor, M11 real pump and safety hardware

This document is the practical bill of materials (BOM), enclosure plan, wiring guide, and sourcing checklist for building one physical Rhizo Edge plant node.

It intentionally separates:

1. the **development/test bench**, where accessibility and debugging matter most;
2. the **battery production-style node**, where low power, moisture protection, wiring discipline, and serviceability matter most.

The selected MCU platform is **ESP32-C3**. The initial development/reference board is **ESP32-C3-DevKitC-02**. A later battery deployment may use **Seeed XIAO ESP32-C3** or a custom ESP32-C3 PCB. Board-specific GPIO and peripheral construction must remain isolated behind the firmware board layer.

---

## 1. Recommended hardware at a glance

### Development / bring-up

```text
ESP32-C3-DevKitC-02
        │
        ├── USB → development PC
        │
        ├── UART → MAX3485/SP3485 → RS485 → SEN0601 soil probe
        │
        ├── GPIO → MOSFET driver → 12 V peristaltic pump
        │
        ├── GPIO ← tank float switch
        │
        └── GPIO ← leak detector
```

Use the DevKit while implementing and debugging M9-M11. It exposes the GPIOs, works well on a breadboard, and is convenient for serial logs and hardware bring-up.

### Battery deployment

```text
                 optional 20 W solar
                         │
                         ▼
              LiFePO4 solar controller
                         │
                         ▼
              12.8 V / 6 Ah LiFePO4
                         │
                    MAIN FUSE
                         │
          ┌──────────────┼────────────────┐
          │              │                │
          ▼              ▼                ▼
     low-Iq 5 V     switched sensor   switched/reg.
        rail            rail           pump rail
          │              │                │
          ▼              ▼                ▼
 XIAO ESP32-C3       SEN0601      MOSFET → pump
          │
          ├── MAX3485 → RS485
          ├── tank float
          └── leak input
```

The soil probe and RS485 transceiver should be powered only while sampling. The pump rail should be powered only while a dose is being executed.

---

# 2. Development/test hardware

## 2.1 M9 firmware bring-up

| Item | Recommended part/type | Qty | Expected price | Search phrase |
|---|---|---:|---:|---|
| MCU board | **ESP32-C3-DevKitC-02** | 1 | €8-15 | `ESP32-C3-DevKitC-02` |
| USB cable | matching board connector | 1 | €3-6 | `ESP32 C3 DevKit USB cable` |
| Breadboard | full/half-size solderless | 1 | €4-8 | `830 point breadboard` |
| Jumper wires | male-male + male-female | 1 set | €4-7 | `Dupont jumper wire kit` |

**M9 bench total:** approximately **€19-36**.

M9 does not require a real soil sensor or real pump. Fake sensor/pump adapters should be proven first.

---

## 2.2 M10 real soil sensor bring-up

Add:

| Item | Recommended part/type | Qty | Expected price | Search phrase |
|---|---|---:|---:|---|
| Soil probe | **DFRobot SEN0601**, RS485 Modbus RTU, moisture + temperature + EC, IP68 | 1 | €35-45 | `DFRobot SEN0601 RS485 soil moisture temperature EC` |
| RS485 transceiver | **MAX3485/SP3485**, 3.3 V logic | 1 | €1-4 | `MAX3485 3.3V TTL RS485 module` |
| Test supply | 12 V / 2 A DC supply | 1 | €8-15 | `12V 2A DC power supply` |

**M9 + M10 bench total:** approximately **€63-100**.

### Why SEN0601

The Rhizo Edge M10 architecture prefers RS485/Modbus RTU as the strategic soil sensor path. SEN0601 fits that model directly and provides the three initial soil measurements required by the project:

- soil moisture;
- soil temperature;
- electrical conductivity (EC).

The probe accepts 5-30 V DC and is IP68. The edge-side protocol and database must not depend on this exact model, so another Modbus probe can later be substituted by configuration/register-map data.

### Important

Use a **3.3 V RS485 transceiver** such as MAX3485/SP3485. Do not connect a 5 V-only MAX485 receiver output directly to an ESP32 GPIO without level compatibility.

---

## 2.3 M11 pump and safety bring-up

Add:

| Item | Recommended part/type | Qty | Expected price | Search phrase |
|---|---|---:|---:|---|
| Pump | 12 V peristaltic pump, roughly 50-100 ml/min | 1 | €10-27 | `12V peristaltic pump 100ml min 3x5mm` |
| Pump driver | 3.3 V logic-compatible MOSFET driver with flyback protection | 1 | €3-7 | `3.3V MOSFET motor driver flyback diode` |
| Gate pull-down | 10 kΩ resistor | 1 | <€1 | `10k resistor 0.25W` |
| Reservoir sensor | horizontal/vertical float switch | 1 | €2-8 | `water level float switch` |
| Leak sensor | wired NO/NC or relay-output detector; DFRobot SEN0454 is one reference | 1 | €15-20 | `DFRobot SEN0454 water leak detector` |
| Tubing | silicone tube sized for selected pump, commonly ~3 mm ID / 5 mm OD | 2-3 m | €3-7 | `silicone tube 3mm 5mm peristaltic` |
| Reservoir | 0.5-1.0 L container with lid | 1 | €3-8 | `water reservoir container 1L lid` |
| Safety/wiring | fuse, holder, cutoff switch, terminal block, pump wire | 1 set | €10-18 | see wiring section |

**Full M9-M11 mains-powered test bench:** approximately **€109-195**.

For initial water tests, use a measuring cup rather than a valuable plant.

---

# 3. Production-style battery node

## 3.1 Controller board

### Recommended

**Seeed XIAO ESP32-C3**

Typical board size:

```text
21 × 17.8 mm
```

It uses the same ESP32-C3 platform as the development board but is much smaller and is more suitable for a low-power enclosure.

The XIAO is a **candidate deployment board**, not a protocol or firmware architecture dependency. A later custom ESP32-C3 PCB should be possible without changing application logic.

### Development-to-production transition

```text
ESP32-C3-DevKitC-02
        ↓
firmware and hardware bring-up
        ↓
XIAO ESP32-C3
        ↓
optional custom Rhizo ESP32-C3 PCB
```

Only the board/HAL layer may change.

The following must not change:

- MQTT protocol;
- offline policy evaluator;
- command validator;
- NVS semantics;
- watering safety logic;
- sensor traits;
- pump trait;
- application state machine;
- Edge Controller.

---

# 4. Battery

## Recommended battery

```text
Chemistry:        LiFePO4
Nominal voltage:  12.8 V
Capacity:         6 Ah
Energy:           ~76.8 Wh
BMS:              integrated
```

A representative 12.8 V / 6 Ah pack is approximately:

```text
151 × 65 × 94 mm
```

Typical current price:

```text
€30-40
```

Search:

```text
12.8V 6Ah LiFePO4 battery BMS
```

Prefer a known battery supplier for unattended operation. AliExpress/Temu is reasonable for passive hardware and inexpensive electronics, but an unknown battery/BMS is not where this project should save €10.

### Charging

Use a charger explicitly designed for **4S / 12 V-class LiFePO4**:

```text
CC/CV charge voltage: approximately 14.4-14.6 V
charge current:       ~1-2 A is sufficient for a 6 Ah Rhizo battery
```

Search:

```text
14.6V 2A LiFePO4 charger 4S
```

Expected price:

```text
€10-25
```

A very high-current 10 A charger is unnecessary for a 6 Ah plant node.

### Cold-weather warning

If the battery may be outdoors, choose a BMS with **low-temperature charging cutoff**, or prevent charging below the battery manufacturer's minimum charging temperature.

A representative basic 6 Ah LiFePO4 battery checked during preparation of this guide specifies charging from 0 °C upward and does not provide low-temperature charging protection.

---

# 5. Production enclosure

## 5.1 Recommended main enclosure

Use:

```text
IP66/IP67
ABS or polycarbonate
300 × 200 × 150 mm
opaque or translucent lid
removable mounting plate preferred
```

**Recommended production size: 300 × 200 × 150 mm.**

**Minimum practical target:** approximately 250 × 200 × 120 mm, but this is tight around a ~94 mm-high battery and leaves much less service space.

A 300 × 200 × 150 mm enclosure gives room for:

- the 6 Ah battery;
- controller PCB;
- RS485 interface;
- voltage regulators;
- fuse holders;
- terminal blocks;
- cable bend radius;
- future solar input;
- future service/replacement.

Typical price:

```text
AliExpress/Temu generic:       ~€15-30
EU generic/industrial ABS:     ~€25-35
EU polycarbonate:              ~€40-50+
```

A currently available reference class is an IP66/67 **300 × 200 × 150 mm** CamdenBoss enclosure.

Search:

```text
IP67 ABS enclosure 300x200x150
IP67 polycarbonate enclosure 300x200x150 mounting plate
```

### Do not put the pump inside the electronics compartment

The main sealed box should contain:

```text
battery + electronics + fuses + terminals
```

The peristaltic pump should be mounted separately, ideally **below** the electronics enclosure. This keeps tubing joints and possible leaks away from the battery and controller.

For outdoor use, add a separate small pump cover/enclosure if the selected pump is not weatherproof.

---

## 5.2 Internal layout

Recommended front view:

```text
                  300 mm
┌──────────────────────────────────────────┐
│                                          │
│  BATTERY ZONE          ELECTRONICS ZONE  │
│                                          │
│  ┌──────────────┐     ┌───────────────┐ │
│  │              │     │ XIAO ESP32-C3│ │
│  │ LiFePO4      │     │ MAX3485      │ │
│  │ 12.8 V 6 Ah  │     │ low-Iq buck  │ │
│  │              │     │ load switch  │ │
│  │ 151×65×94    │     │ pump driver  │ │
│  └──────────────┘     └───────────────┘ │
│                                          │
│  battery strap        ┌───────────────┐ │
│                       │ FUSES         │ │
│                       │ TERMINALS     │ │
│                       └───────────────┘ │
│                                          │
├──────────────────────────────────────────┤
│ ↓ soil  ↓ tank  ↓ leak  ↓ pump  ↓ solar │
└──────────────────────────────────────────┘
                 200 mm
depth: 150 mm
```

Keep the battery and low-voltage electronics physically separated enough that battery replacement cannot tear small signal wires from the PCB.

### Mounting

- battery: mechanical strap/bracket, not only double-sided tape;
- electronics: standoffs on a mounting plate;
- terminal blocks: along the bottom edge;
- fuses: accessible without removing the controller;
- cable glands: bottom face wherever possible;
- antenna: keep the ESP32 antenna away from the battery, metal fasteners, and dense wiring.

---

# 6. Enclosure fittings

## Cable glands

Recommended:

| Function | Typical gland |
|---|---|
| Soil sensor cable | M12 IP68 |
| Tank sensor | M12 IP68 |
| Leak sensor | M12 IP68 |
| Pump cable | M12 or M16 IP68 |
| Solar cable | M16 IP68 |
| Spare/service | one plugged M12/M16 opening |

Typical cost for a small set:

```text
€4-8
```

Search:

```text
M12 IP68 cable gland 3-6.5mm
M16 IP68 cable gland
```

Always verify the actual cable outside diameter before drilling.

### Drip loop

All external cables should enter from below and form a drip loop:

```text
       enclosure
          │
          │
          │ cable
          │
          U
           \________ sensor
```

Water should reach the bottom of the loop before it can reach the gland.

---

## Pressure-equalization vent

Add one hydrophobic enclosure vent:

```text
M12
IP68
membrane pressure-equalization / breather vent
```

Expected price:

```text
€4-8
```

Search:

```text
M12 IP68 waterproof breathable vent plug
M12 Druckausgleichselement IP68
```

A sealed enclosure experiences temperature and pressure changes. A membrane vent reduces pressure pumping and condensation risk without creating an open air hole.

---

# 7. Power architecture

## 7.1 Recommended topology

```text
12.8 V LiFePO4
      │
      ├── MAIN FUSE near battery
      │
      ├── low-Iq regulator → 5 V → XIAO ESP32-C3
      │
      ├── high-side switch → SEN0601
      │                       + MAX3485
      │
      └── switched/reg. 12 V → MOSFET → pump
```

### Important: a "12 V" LiFePO4 battery is not always 12 V

The battery may be near 14.4 V while charging/full.

The SEN0601 accepts 5-30 V, so this is not a problem for the probe.

A nominal 12 V pump, however, must either:

1. explicitly tolerate the battery's maximum voltage; or
2. receive a regulated 12 V pump rail.

Do not assume every 12 V pump is safe at the battery's full-charge voltage.

---

## 7.2 ESP32 regulator

The always-on ESP32 regulator must have low quiescent current.

Target:

```text
input range:    must cover full LiFePO4 voltage
output:         5 V for XIAO VIN
continuous:     >= 500 mA recommended
quiescent Iq:   < 50 µA target
                < 20 µA preferred
```

A regulator family such as TI TPS6217x demonstrates the required class of device: wide input and single-digit/tens-of-microamp sleep/quiescent current.

Search:

```text
low quiescent current buck 12V 5V module 20uA
TPS62177 module
```

**Do not use a generic LM2596 module as the final always-on battery regulator** unless its measured standby current meets the power budget.

---

## 7.3 Soil sensor power gating

The soil probe is a much larger load than the sleeping ESP32 and should not stay powered continuously in battery mode.

Use:

```text
ESP32 GPIO
    │
    ▼
high-side MOSFET/load switch
    │
    ▼
SEN0601 + RS485 transceiver
```

Required switch characteristics:

```text
3.3 V-controllable
voltage rating safely above 14.6 V
current capacity >= 1 A
low off-state leakage
default OFF during MCU reset
```

Battery sampling cycle:

```text
deep sleep
   ↓
wake
   ↓
sensor power ON
   ↓
wait for measured/verified stabilization time
   ↓
Modbus read
   ↓
sensor power OFF
   ↓
Wi-Fi / MQTT exchange
   ↓
deep sleep
```

The exact SEN0601 cold-start stabilization delay should be measured before the final battery-life claim is frozen.

---

# 8. Pump hardware

## Recommended first pump

```text
12 V DC peristaltic pump
target flow: roughly 50-100 ml/min
replaceable silicone tube preferred
```

Expected price:

```text
generic:        €10-15
better/Kamoer:  €20-30
```

The pump's advertised flow is not trusted as calibration. Rhizo Edge measures actual delivered water and stores `ml_per_second`.

### Pump driver requirements

Use a **logic-level MOSFET solution verified at 3.3 V gate drive**.

Requirements:

```text
3.3 V logic compatible
load voltage > maximum pump rail
current rating comfortably above measured pump current
flyback protection
hardware gate pull-down
default physical state = pump OFF
```

Avoid using an IRF520-based hobby board as the final driver merely because it is cheap. It is not an ideal MOSFET choice for low-voltage 3.3 V gate drive.

### Hardware pull-down

Install a real resistor:

```text
pump MOSFET gate/control → 10 kΩ → GND
```

The pump must remain off while the ESP32 pin is floating during reset/boot.

---

# 9. Reservoir and tubing

## Reservoir

For the first node:

```text
capacity: 0.5-1.0 L
lid:      yes
```

Expected price:

```text
€3-8
```

A small reservoir also places a physical bound on the maximum possible spill.

## Tank level

The first sensor can be a binary float switch.

Expected price:

```text
€2-8
```

It is acceptable for V1 to report only:

```text
water available
tank low/empty
```

rather than inventing a fake continuous percentage.

## Tubing

Typical small peristaltic tubing:

```text
~3 mm inner diameter
~5 mm outer diameter
```

but the exact tube must match the selected pump head.

Buy at least:

```text
2-3 metres
```

so the first installation is not constrained by cutting mistakes.

Optional but useful:

- anti-siphon/check valve;
- small tube clips;
- drip emitter/nozzle at the pot;
- strain relief near the pump.

---

# 10. Leak detection

Recommended reference:

```text
DFRobot SEN0454
wired NO/NC / relay-output leak detector
IP66
```

Typical current EU price:

```text
~€16-19
```

Cheaper wired detectors can be used if their failure behavior is understood and they can be tested electrically.

The safety requirement is more important than the brand:

```text
leak detected before dose  → refuse
leak detected during dose  → pump off immediately
sensor unknown/failed      → refuse
```

The leak detector must be actively monitored for the entire pump run.

---

# 11. Wiring and connectors

## Recommended wire sizes

These are practical starting points, not substitutes for verifying actual current and cable length.

| Circuit | Suggested wire |
|---|---|
| Battery → main fuse / distribution | 0.75-1.0 mm² stranded |
| Pump power | 0.5-0.75 mm² stranded |
| 12 V sensor power | 0.25-0.5 mm² |
| RS485 A/B | twisted pair ~0.2-0.34 mm² |
| Float/leak inputs | 0.14-0.25 mm² |
| Short internal logic wiring | 0.14-0.25 mm² |
| Solar panel, longer outdoor run | typically 1.0-1.5 mm² |

Use ferrules on stranded wire going into screw terminals.

## Connectors

Inside the enclosure prefer:

- pluggable screw terminals;
- WAGO-style lever connectors where appropriate;
- JST for low-current internal board connections only;
- proper crimp terminals for battery wiring.

Avoid permanent Dupont jumpers in the final node.

---

# 12. Fuse and physical cutoff

## Main fuse

Install the main fuse as close to the battery positive terminal as practical.

Conceptual layout:

```text
battery +
   │
 [main fuse]
   │
   ├── electronics branch
   └── pump branch
```

Practical starting point for a small node may be around **3 A main**, but the final value must be selected from:

- selected pump current;
- converter current;
- cable gauge;
- expected fault current.

Do not copy a fuse value without checking the actual pump.

## Separate branches

Prefer:

```text
main fuse
   │
   ├── electronics fuse ~0.5-1 A
   └── pump fuse sized to measured pump current
```

## Physical pump cutoff

Provide a hardware switch that removes pump power independently of firmware.

During HIL testing it must be reachable immediately.

Expected cost:

```text
€3-7
```

---

# 13. Battery-mode board and GPIO portability

Board wiring belongs in the firmware board layer.

A possible layout:

```text
src/
├── board/
│   ├── mod.rs
│   ├── devkitc02.rs
│   └── xiao_esp32c3.rs
├── app/
├── sensors/
├── pump/
├── net/
└── main.rs
```

Concrete GPIO numbers must not leak into application, safety, MQTT, policy, or domain logic.

The board profile owns at least:

- UART TX/RX;
- RS485 DE/RE;
- pump-control pin;
- sensor power-enable pin;
- tank input;
- leak input;
- active-high/active-low polarity;
- board-specific power control;
- concrete ESP-IDF peripheral construction.

Compile-time board selection is preferred.

---

# 14. Production BOM

| Item | Recommended specification | Budget |
|---|---|---:|
| XIAO ESP32-C3 | ESP32-C3 deployment board | €5-8 |
| SEN0601 | DFRobot RS485 moisture/temp/EC | €35-45 |
| RS485 | MAX3485/SP3485 3.3 V | €1-4 |
| Pump | 12 V peristaltic | €10-27 |
| Pump MOSFET | 3.3 V logic, flyback, pull-down | €3-7 |
| Tank sensor | float switch | €2-8 |
| Leak detector | wired NO/NC/relay | €15-20 |
| Battery | LiFePO4 12.8 V / 6 Ah with BMS | €30-40 |
| MCU regulator | low-Iq battery → 5 V | €5-12 |
| Sensor switch | high-side 12 V load switch | €2-5 |
| Pump rail allowance | regulated/switched 12 V if needed | €4-10 |
| Main enclosure | IP66/IP67 300×200×150 | €25-45 |
| Cable glands | M12/M16 IP68 set | €4-8 |
| Membrane vent | M12 IP68 | €4-8 |
| Fuses/holders | main + branch | €3-7 |
| Terminals/mounts | terminals, standoffs, battery strap | €5-12 |
| Wiring | power/signal wire + ferrules | €6-12 |
| Tubing | 2-3 m silicone | €3-7 |
| Reservoir | 0.5-1 L | €3-8 |
| Pump cutoff | independent hardware switch | €3-7 |
| Charge inlet | connector/gland/protection | €2-6 |
| LiFePO4 wall charger | 14.4-14.6 V, ~1-2 A | €10-25 |

**Estimated complete battery node:** approximately **€183-328**, before shipping.

A realistic target when combining AliExpress/Temu passive parts with a trusted soil sensor and battery is approximately **€190-230** for a polished one-off node. The very bottom of the range assumes aggressive sourcing and re-use of small parts already on hand; the top end assumes mostly EU retail and better-branded parts.

---

# 15. Optional solar add-on

## Recommended starting point

```text
20 W monocrystalline panel
        ↓
LiFePO4-compatible solar controller
        ↓
12.8 V / 6 Ah LiFePO4 battery
```

### BOM

| Item | Specification | Budget |
|---|---|---:|
| Solar panel | 20 W, 12 V-class mono | €23-40 |
| Controller | explicit LiFePO4 support, ~10 A class | €30-55 |
| Cable/mounting/glands | outdoor-rated | €8-15 |

**Solar add-on:** approximately **€61-110**.

**Complete production-style node with solar:** approximately **€244-438**.

### Controller warning

For a very low-power Rhizo node, the solar controller's own standby consumption matters. A cheap controller consuming several tens of milliamps can waste more energy than the sleeping plant node.

Before selecting the final controller, verify:

- explicit 4S LiFePO4 support;
- configurable/correct charge voltage;
- low-temperature charging behavior;
- standby/self-consumption;
- panel voltage/current limits;
- whether the product is truly MPPT rather than only labelled MPPT.

For a 20 W system, low self-consumption can matter more than buying a large controller.

---

# 16. What is safe to buy cheaply

## Good AliExpress/Temu candidates

Usually reasonable to source inexpensively:

- ESP32-C3 development boards;
- XIAO-compatible boards **if provenance is acceptable for a prototype**;
- MAX3485/SP3485 modules;
- cable glands;
- membrane vents;
- terminal blocks;
- ferrules;
- wire;
- heat-shrink;
- float switches;
- silicone tubing;
- enclosure;
- standoffs and mounting hardware;
- pump for early calibration/testing.

## Prefer a traceable source

Spend more carefully on:

- **LiFePO4 battery and BMS**;
- **soil probe used for safety decisions**;
- **leak detector**;
- **pump driver in unattended automatic operation**;
- solar charge controller;
- mains-powered charger.

The distinction is between a component whose failure is easy to see and replace and a component whose silent failure can create an unsafe watering decision.

---

# 17. Suggested exact shopping list for the first build

Buy now for development:

```text
[ ] 1 × ESP32-C3-DevKitC-02
[ ] 1 × USB cable
[ ] 1 × breadboard
[ ] 1 × jumper-wire kit
[ ] 1 × DFRobot SEN0601
[ ] 2 × MAX3485/SP3485 modules (one spare)
[ ] 1 × 12 V / 2 A supply
```

Buy when starting M11:

```text
[ ] 1 × 12 V peristaltic pump
[ ] 2 × 3.3 V-compatible MOSFET driver modules (one spare)
[ ] assorted 10 kΩ resistors
[ ] 2 × float switches (one spare)
[ ] 1 × wired leak detector
[ ] 3 m silicone tubing
[ ] 1 × 0.5-1 L reservoir
[ ] fuse holders + fuse assortment
[ ] physical pump cutoff switch
[ ] 0.5-1.0 mm² stranded power wire
[ ] 0.14-0.34 mm² signal/twisted-pair wire
[ ] ferrules + crimp tool
```

Buy when converting to the battery enclosure:

```text
[ ] 1 × Seeed XIAO ESP32-C3
[ ] 1 × 12.8 V / 6 Ah LiFePO4 battery with BMS
[ ] 1 × low-Iq 12 V → 5 V regulator
[ ] 1 × high-side sensor power switch
[ ] 1 × regulated/switched pump supply if pump cannot tolerate 14.4-14.6 V
[ ] 1 × IP66/IP67 enclosure 300×200×150 mm
[ ] M12/M16 IP68 glands
[ ] 1 × M12 IP68 membrane vent
[ ] internal terminal blocks
[ ] battery strap/bracket
[ ] standoffs/mounting plate
[ ] protected external charging connector
[ ] 14.4-14.6 V LiFePO4 charger, ~1-2 A
```

Optional solar:

```text
[ ] 1 × 20 W monocrystalline solar panel
[ ] 1 × low-self-consumption LiFePO4 solar controller
[ ] outdoor solar cable
[ ] panel mounting bracket
```

---

# 18. Assembly order

Do not assemble the whole system and debug everything at once.

## Stage A — firmware board

```text
DevKitC-02 → USB → serial logs
```

Verify flash, reboot, NVS, Wi-Fi, MQTT, time sync, and offline policy state.

## Stage B — soil probe

```text
DevKit → MAX3485 → SEN0601
```

Verify Modbus without any pump connected.

Measure:

- sensor startup time;
- active current;
- power-off leakage;
- repeated-reading stability.

## Stage C — fake load before real pump

Drive an LED/test load through the final MOSFET path.

Verify:

- boot = OFF;
- reset = OFF;
- watchdog reset = OFF;
- GPIO floating = OFF;
- run timeout removes power.

## Stage D — pump into measuring cup

```text
pump → tubing → measuring cup
```

Calibrate five runs before putting the outlet into a plant.

## Stage E — tank + leak

Test both sensor failure states physically.

## Stage F — battery

Only after the mains-powered HIL path is stable:

```text
wall supply → LiFePO4
```

Measure sleep and active current with a meter. Do not derive final battery life from datasheet numbers alone.

## Stage G — enclosure

Drill only after every actual cable has been selected and measured.

## Stage H — solar

Add solar last. First prove the battery node independently.

---

# 19. Price reference snapshot

The following current examples were used only to anchor the price ranges. They are not mandatory vendors and may change.

| Part | Example source checked 2026-08-28 | Observed price/fact |
|---|---|---|
| ESP32-C3-DevKitC-02 | TME / AliExpress listings | ~€10-13 |
| XIAO ESP32-C3 | Reichelt | €5.40 without headers / €6.60 with headers |
| SEN0601 | DigiKey Germany / DFRobot | ~€34.10 ex VAT at DigiKey; $39 manufacturer price |
| SEN0454 leak sensor | DigiKey Germany / DFRobot | ~€15.69 ex VAT / $17.90 manufacturer |
| 12.8 V 6 Ah LiFePO4 | LiTime Germany | €39.99; 76.8 Wh; ~150.8×65×94 mm |
| 300×200×150 IP66/67 ABS box | Reichelt CamdenBoss | ~€31.40 |
| 300×200×150 IP66/67 polycarbonate box | Reichelt CamdenBoss | ~€44.90 |
| 12 V peristaltic pump | Funduino / Kamoer marketplace examples | ~€9.90 to ~€26.57 |
| M12 IP68 cable glands | Reichelt / commodity sets | a few euros per pair/set |
| M12 IP68 vent | DigiKey / enclosure suppliers | ~€4.69-7.09 |
| 20 W panel | Victron / generic marketplace examples | ~€23-40 |
| 10 A LiFePO4-capable solar controller | current German listings | ~€30-55 for practical mid-range options |

Reference pages:

- Espressif ESP32-C3-DevKitC-02 user guide:  
  https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32c3/esp32-c3-devkitc-02/user_guide.html
- Seeed XIAO ESP32-C3 specifications:  
  https://wiki.seeedstudio.com/XIAO_ESP32C3_Getting_Started/
- DFRobot SEN0601:  
  https://wiki.dfrobot.com/sen0601/
- DFRobot SEN0454:  
  https://www.dfrobot.com/product-2316.html
- LiTime 12 V / 6 Ah reference battery:  
  https://www.litime.de/products/12v-6ah-lifepo4-batterie

---

# 20. Known decisions still requiring measurement

Do not freeze these values purely from this document:

1. **SEN0601 cold-start stabilization time.** Measure on the actual probe.
2. **Pump current and actual flow.** Measure the purchased pump.
3. **Pump maximum supply voltage.** Check the selected model against the LiFePO4 full-charge voltage.
4. **Final fuse ratings.** Select from actual current and wire gauge.
5. **Final battery autonomy.** Measure complete-node sleep and wake-cycle energy.
6. **Solar-controller self-consumption.** Measure or obtain a trustworthy specification before calling the solar system energy-positive.
7. **Exact gland hole sizes.** Select from the actual cables, not from this drawing.
8. **Production board.** XIAO ESP32-C3 remains a candidate until its real sleep-current and peripheral wiring are verified.

---

# 21. Safety and construction rules

- Keep mains voltage **outside** the Rhizo enclosure. Use certified external low-voltage adapters/chargers.
- Fuse the battery close to its positive terminal.
- The pump must default electrically OFF when the MCU is unpowered or resetting.
- Do not power the pump from the ESP32 board.
- Do not rely on software alone for pump-off safety.
- Keep wet tubing and pump connections physically below/away from electronics.
- Use a physical pump power cutoff during HIL testing.
- Treat failed/unknown leak and tank sensors as a watering refusal.
- Do not charge a LiFePO4 battery below the manufacturer's allowed charging temperature.
- Re-check gland seals after any service.
- Before unattended use, run several full watering cycles into a measuring container and deliberately test leak, empty-tank, reset, and power-loss cases.

---

# 22. Recommended repository location

Suggested path:

```text
docs/hardware/home-node-hardware-guide.md
```

Link it from `docs/README.md`, and reference it from PRD 090, PRD 100, PRD 110, and the hardware-in-the-loop guide.

This document is a practical procurement/assembly guide. Normative safety behavior remains defined by the ADRs, PRDs, safety-invariant registry, and MQTT protocol.
