# M9 current bench state

Last confirmed: 2026-11-01, user reports unplug/replug of both ESP32 and PicoScope; current wiring unchanged.
Update this record immediately after **every confirmed physical change**;
do not infer a requested action was performed. Before requesting an action,
read this file, the PicoScope runbook, relevant current M9 report section and
local evidence. Label its target VERIFIED, NEEDS REPEAT COUNT or PENDING.
Do not request actions for VERIFIED items without a recorded invalidation.

| Item | Current state |
|---|---|
| ESP32 | Powered over USB; user confirmed reconnect after corrected-LWT retest; `devkitm1` + advisory `bench-sensors` / production GpioPump |
| Firmware | ELF SHA-256 63f0fbd28da3ca0f22b8646801a136deb57ed51e6dcb7de90aade8990ab7fe4c |
| Display | DISCONNECTED; former connections 3V3/GND/SDA GPIO3/SCL GPIO2 |
| Pump / MOSFET / actuator load | All disconnected |
| Actuator wiring | GPIO5 → 10 kΩ → GND |
| PicoScope A | Tip on GPIO5, ground clip on board GND, physical x10, DC |
| PicoScope USB/backend | Windows-owned / windows-ps2000; block validation 70/70; streaming NOT QUALIFIED for M9 safety |
| Generator | Returned to DC zero; leads disconnected from board/probe |
| ESP serial | Attached using discovery helper; status exit 0 READY at /dev/ttyUSB0 on 2026-11-01 |
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
GPIO5 cold-boot safety is deferred by user instruction. Block capture is
functional but lacks full-boot depth at microsecond-scale sensitivity: 16,256 A
samples or 8,064/ch with B reference, versus roughly 0.6 s startup coverage.
Streaming remains disqualified. No current block parameters prove full boot.
No physical action is requested during this qualification phase. No HIL count is earned
by a setup reset, static baseline, or scope functional-validation capture.

Stationary diagnostics after the user’s USB replug do not change bench wiring
or HIL counts. The old cold-boot capture remains INVALID (1,927 missing samples).
See the M9 report for the full baseline/timer comparison and sensitivity limits.

No physical changes during block investigation. B remained unwired; dual-channel
software captures were capacity/structure diagnostics only. Existing block AWG
evidence is reused. See the runbook block-window table before any probe request.


Current phase: independent M9 acceptance work. No probe/wiring/load changes.
Scope transient investigation is deferred; no HIL counts change. The ESP attach
helper restored WSL ownership after discovery; status check confirmed READY.
That status utility performs a setup reset; it is not a HIL trial.

2026-11-01 protocol phase: advisory bench-sensors image flashed, NVS preserved;
production safety inputs unchanged, pump disabled, always-on. Scope/probe/load
wiring unchanged. LAN listeners restored by user Administrator action.
LWT observation armed; physical USB removal CONFIRMED by user. LWT observed; Edge remained online due to duplicate nil message_id. Board stays unplugged pending serializer fix.

LWT defect reproduced and fixed in source; focused host/conformance and ESP
builds passed. Corrected image not flashed while board is unplugged. Next action
is reconnect USB for flashing/retest; do not count this as measured HIL-1.

Latest state: user reconnected ESP32 USB; discovery helper reattached WSL.
Corrected Last Will image flashed with NVS preserved; online/synchronized,
config 12, production GpioPump with advisory bench-sensors. Serial monitor
closed. LWT retest armed with host USB-removal polling; awaiting next unplug.

Corrected LWT retest: user confirmed unplugged, host USB disappearance observed;
unique will reached Edge, API offline/isolated. GPIO/HIL counts unchanged.

Latest result: corrected will durably accepted; Edge offline/isolated. USB remains
unplugged. Keepalive mismatch corrected to canonical 60 s in source and builds,
not yet flashed. Functional LWT fix verified; timing criterion remains open.

Latest: user reconnected; discovery helper restored WSL attachment. Canonical
60-second keepalive image flashed; broker confirms k60, Edge online/synced.
Serial monitor closed, lwt60 capture armed; board currently powered.
