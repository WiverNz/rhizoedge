# M9 current bench state

Last confirmed: 2026-11-01, user replied “connected” during the cold-boot capture attempt.
Update this record immediately after **every confirmed physical change**;
do not infer a requested action was performed. Before requesting an action,
read this file, the PicoScope runbook, relevant current M9 report section and
local evidence. Label its target VERIFIED, NEEDS REPEAT COUNT or PENDING.
Do not request actions for VERIFIED items without a recorded invalidation.

| Item | Current state |
|---|---|
| ESP32 | Powered over USB; production `devkitm1` / GpioPump |
| Firmware | ELF SHA-256 f484c6f3e75c9cb26687c2f7ae5c2afe30ef5b685eb9ec7c94f8933305d73702 |
| Display | DISCONNECTED; former connections 3V3/GND/SDA GPIO3/SCL GPIO2 |
| Pump / MOSFET / actuator load | All disconnected |
| Actuator wiring | GPIO5 → 10 kΩ → GND |
| PicoScope A | Tip on GPIO5, ground clip on board GND, physical x10, DC |
| PicoScope USB/backend | Windows-owned / validated windows-ps2000 |
| Generator | Returned to DC zero; leads disconnected from board/probe |
| ESP serial | USB reconnected; WSL attachment not yet checked; last port /dev/ttyUSB0 |
| Latest steady LOW | -22.736..+22.889 mV, 0 pulses >0.8 V, 3 valid blocks |
| Pending proposed 3V3 probe move | **CANCELLED as redundant; not performed** |

## Evidence disposition

| Category | Item | Evidence / remaining requirement |
|---|---|---|
| VERIFIED | x10 voltage scaling, trigger directions, pulse counting and ~500 µs widths | AWG rising/falling captures and signal-validation-summary.json in local/evidence/picoscope/20261101T093926Z-i0m4k0af/; falling directory 20261101T093944Z-91kgh7jw |
| VERIFIED | Above-signal trigger timeout and generator zero cleanup | 20261101T094020Z-mxrmkdku/ and 20261101T094044Z-q954xszx/ |
| VERIFIED | GND and unpowered GPIO5 baselines | 50 blocks each; 20261031T232620Z-_ld1lxgx/ and 20261031T232717Z-ex63cxc4/ |
| VERIFIED | Display-free production GPIO5 steady LOW | 20261101T101349Z-8vkxphkg/ |
| VERIFIED | PeakTech static GPIO5 and supply reference | -0.3 mV and 3.297 V; display was connected at those readings |
| PENDING | Whole-boot transient evidence | First cold-boot stream invalid: buffer overrun / 1,927 missing samples; m9-cold-boot-20261101-01. Subsequent stationary diagnostic valid, but not a boot trial |
| PENDING | Flash transition / watchdog reset GPIO evidence | Neither captured as HIL; watchdog deliberate-hang procedure still needed |
| NEEDS REPEAT COUNT | Normal resets | 0/20 counted HIL trials |
| NEEDS REPEAT COUNT | Mid-boot power cuts | 0/10 counted HIL trials |
| PENDING | Hard-limit GPIO, actual executed GPIO dedup, peripheral gating and remaining M9 hardware | Not established by virtual actuator or static LOW evidence |

No probe move, resistor recheck, repeat supply measurement or generic board
identification is needed to repeat already-verified setup facts. Reuse evidence
unless relevant wiring/firmware changed, it did not prove the criterion,
contradictory evidence appeared, or the criterion requires more trials.
Batch software work and all applicable tests for the current probe position.
The next physical criterion remains GPIO5 cold-boot safety. Investigate the long-
stream overrun before requesting another power removal/reconnection. No HIL count is earned
by a setup reset, static baseline, or scope functional-validation capture.
