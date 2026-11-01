# PicoScope 2205A bench runbook

**Last verified:** 2026-11-01. **Preferred backend:** `windows-ps2000`.
This is the living setup/operation reference for this instrument, including
work after M9. Update it in the same logical phase as changes to wiring,
scripts, SDK/backend, capture settings, recovery procedures or verified limits.
Record newly verified versions and evidence locations; do not convert an
assumption into a verified capability. Keep milestone-specific results and
acceptance status in the [M9 report](../reports/M9.md), and retain the required
counts/criteria in [HIL procedures](../testing/hardware-in-the-loop.md).

## Evidence reuse before physical actions

Read the [current M9 bench state](m9-current-bench.md), this runbook, relevant
current M9 report section and saved local evidence before requesting a probe
move, USB cycle, reset or wiring change. The current-state record must be
updated after every confirmed physical change. Reuse VERIFIED evidence unless
relevant wiring/firmware changed, the measurement did not prove the criterion,
contradictory evidence appeared, or required repetition counts remain.

Classify work as **VERIFIED**, **NEEDS REPEAT COUNT**, or **PENDING**. Only the
latter two justify another physical verification. Existing AWG evidence already
proves functional voltage scaling/x10, both triggers, pulse detection and width
analysis; a 3V3 scope measurement solely to reconfirm scaling is redundant.
Keep the probe on GPIO5 unless a specific unsatisfied criterion needs a move.
Batch applicable software/captures in each confirmed configuration. The clean-
machine procedure below is for initial setup, not a checklist to repeat per turn.

## Hardware and connections

| Item | Verified setup |
|---|---|
| Instrument | PicoScope **2205A**, USB VID:PID **0ce9:1007** |
| Input | Channel **A**, DC coupling; B disabled |
| Probe | Physical switch **10×**; software also describes a physical x10 probe |
| GPIO | ESP32-C3 board profile `devkitm1`, actuator **GPIO5** (not obsolete GPIO4) |
| Pull-down | External **10 kΩ** from GPIO5 to board GND; required even with firmware OFF |
| DMM | **PeakTech 4390**, resistance/continuity/static DC; DT-831 is faulty and excluded |
| Generator adapter | AWG/Gen BNC to red signal and black ground test leads |

With ESP32 power removed, wire:

```text
GPIO5 ──────── Channel A probe tip
  │
  10 kΩ
  │
board GND ──── Channel A ground clip
```

Use a nearby board GND and the shortest practical ground connection. Record
changes to lead length/ground placement: they affect ringing and transient
measurements. A resistor is non-polarized. The fitted resistor measured
9.97 kΩ isolated and 9.22 kΩ in circuit with the PeakTech; in-circuit readings
include board paths. Never use resistance/continuity mode on a powered board.

During early electrical validation, **pump, MOSFET controller and other loads
remain disconnected**. The probe and pull-down are the only GPIO5 connections.
The DMM can establish static LOW/HIGH; it cannot prove absence of short pulses.
Confirm accessory wiring explicitly: an inactive/blank display still loads the
board. The verified display allocation is 3V3/GND, SDA GPIO3 (J1 pin 5),
SCL GPIO2 (J1 pin 4), address 0x3C; earlier GPIO5/GPIO4 wiring is obsolete.
Remove all four display wires with board power off for the isolated early
GPIO bench. Record whether accessories were connected for each measurement.
Do not infer actuator safety from a virtual-actuator firmware image.

For the generator loop, first disconnect **both** probe tip and ground clip
from the ESP32. Leave ESP32 unpowered and its pull-down fitted:

```text
PicoScope AWG/Gen BNC → red lead   → Channel A x10 probe tip
                     → black lead → Channel A ground clip
```

This loop is separate from the board. Never feed the generator into GPIO5 or
leave the probe bridging the generator and ESP32. Before returning the probe
to GPIO5, verify generator zero and disconnect the loop. No PicoScope GUI is
needed for these automated checks; keep it closed to avoid device contention.

## Software and API inventory

These are observed versions, not claims about the latest available release.

| Component | Observed value |
|---|---|
| Manual GUI | PicoScope 7 T&M **7.2.30.10560 x64** |
| Windows API driver | **3.0.152.6217**, real open/query/close verified |
| API | **ps2000**, NOT ps2000a (similarly named models can use different APIs) |
| Windows runtime | x64 Windows PowerShell **5.1.22621.6133**, C# compiled by `Add-Type` |
| USB ownership tooling | usbipd-win **5.3.0**; discovery/detach only for normal operation |
| WSL during investigation | Ubuntu **26.10**, kernel **6.18.54.1-microsoft-standard-WSL2** |
| Linux packages tried | `picoscope` 7.2.30.10560; `libps2000` 3.0.152-3r6217; `libps2000a` 2.2.159-5r6606 installed but not used |
| Linux USB tools | `libusb-1.0-0` 2:1.0.30-1; `usbutils` 1:019-1 |

Observed DLL, also the backend default (resolved from Windows `ProgramFiles`):

```text
C:\Program Files\Pico Technology\PicoScope 7 T&M Stable\ps2000.dll
```

Discover another installed SDK location in Windows PowerShell:

```powershell
Get-ChildItem "$env:ProgramFiles\Pico Technology" -Filter ps2000.dll -Recurse |
    Select-Object FullName,@{Name='Version';Expression={$_.VersionInfo.FileVersion}}
```

Use `--dll 'C:\actual\SDK\ps2000.dll'` on either WSL wrapper if needed. Keep
vendor dependencies available with the installed SDK; do not copy a lone DLL
and assume its dependencies/drivers are installed. The backend loads the x64
DLL and resolves actual ps2000 exports; it is not a simulator.

The Linux installation (`sudo apt install picoscope`) supplied
`/opt/picoscope/lib/libps2000.so` and
`/opt/picoscope/include/libps2000/ps2000.h`. These helped verify the ABI but
are **not required for the preferred capture path**. Installing usbutils made
`lsusb` available; enumeration never established SDK readiness.

## Why Windows owns the USB device

The investigated Linux ps2000/usbipd path could enumerate the 2205A and
occasionally open it, but repeated opens reset/disconnected USB or hung with
`device descriptor read/64, error -110`. Both the ctypes wrapper and a separate
C program compiled against the vendor header reproduced failure. The C program
blocked in `libusb_reset_device`, with its process in kernel `usb_submit_urb`
D state. Reattaching during first initialization allowed one open to finish,
but the next independent open still failed. This was not a reliable capture
path, and retrying until one open succeeded was rejected.

The approved arrangement is:

```text
WSL shell/Codex → Python CLI → powershell.exe supervisor → C# worker
                                                        → Windows ps2000.dll
                                                        → Windows-owned 2205A
```

Four consecutive Windows open/configure/close checks passed, and access was
verified again after a PC restart. LOW and known-signal captures then passed
the functional checks below. Do not attach the scope to WSL for measurement.
The ESP32 USB-UART may be attached to WSL independently; it is a different
VID:PID. Neither historical Pico BUSID `5-1` nor ESP BUSIDs are permanent.

## Scripts, ownership and readiness

| Script | Role |
|---|---|
| [`picoscope-status.sh`](../../scripts/picoscope-status.sh) | Preferred WSL readiness check through Windows API |
| [`picoscope-capture.sh`](../../scripts/picoscope-capture.sh) | WSL capture entry point |
| [`picoscope_cli.py`](../../scripts/picoscope_cli.py) | Arguments, stale Linux worker/lock checks, evidence paths, native invocation |
| [`picoscope-windows.ps1`](../../scripts/picoscope-windows.ps1) | Bounded native supervisor and worker orchestration |
| [`PicoScopeWindows.cs`](../../scripts/PicoScopeWindows.cs) | Real ps2000 calls, scaling, raw samples and pulse analysis |
| [`picoscope-status.ps1`](../../scripts/picoscope-status.ps1) | Native Windows readiness; `-OwnershipOnly` only discovers USB state |
| [`picoscope-wsl-common.ps1`](../../scripts/picoscope-wsl-common.ps1) | Discovers VID:PID plus Pico description; refuses ambiguous devices |
| [`picoscope-wsl-detach.ps1`](../../scripts/picoscope-wsl-detach.ps1) | Returns discovered scope to Windows, leaving it shared; idempotent |
| [`picoscope-wsl-attach.ps1`](../../scripts/picoscope-wsl-attach.ps1) | Retained diagnostic helper, **not normal measurement operation** |
| [`picoscope_status.py`](../../scripts/picoscope_status.py) | Legacy Linux SDK diagnostic; do not retry the rejected path |

From the repository root in WSL:

```sh
./scripts/picoscope-status.sh --json
./scripts/picoscope-capture.sh --help
```

Windows PowerShell equivalents for status and ownership recovery:

```powershell
.\scripts\picoscope-status.ps1 -Json
.\scripts\picoscope-status.ps1 -OwnershipOnly -Json
.\scripts\picoscope-wsl-detach.ps1
```

Where the host execution policy blocks repository scripts, invoke with
`powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\scripts\picoscope-status.ps1 -Json`.
This is process-local; the tooling does not change the machine execution policy.

`READY` / exit 0 means real API **open, query model 2205A, query serial without
printing it, configure A/DC and B-off, and successfully close**. It does not
mean the probe is connected correctly or the measurement is calibrated.
Inspect both status and exit code; `backend` must be `windows-ps2000`.

| Status/code | Meaning/action |
|---|---|
| NOT CONNECTED / 11 | Check scope USB cable; no API success is claimed |
| ATTACHED TO WSL / 13 | Detach this scope using its helper before Windows capture |
| MULTIPLE DEVICES / 16 | Discovery refuses to guess; isolate the intended scope |
| DRIVER/API MISSING / 20 | Check x64 SDK/DLL installation and `--dll` |
| Failure / 21 | Open/close/worker/lock failure; read `message`, do not retry blindly |
| CAPTURE TIMEOUT / 22 | No completed triggered waveform; not a zero-pulse trace |
| INVALID CAPTURE / 23 | Inspect overflow, clipping or data validity |
| NOT SHARED / 18; WINDOWS AVAILABLE / 19 | Ownership-only diagnostics; both can be usable by Windows |

`usbipd` is currently required by the scripts for ownership discovery, even
though **binding/attachment is not required** for Windows capture. Only an
explicit diagnostic attach needs binding. If its helper reports Not shared,
it prints `usbipd bind --busid <discovered-current-BUSID>` for a one-time
Administrator PowerShell session. Helpers never elevate or use `--force`.

## Clean-machine procedure

1. Install Windows x64 PicoScope 7 or the appropriate PicoSDK/USB driver from
   Pico Technology. Verify `ps2000.dll` exists and record its version/location.
   The GUI may be installed but must remain closed during automated work.
2. Install WSL with Python 3 and working Windows interoperability (`powershell.exe`,
   `wslpath`), plus usbipd-win for the helper discovery. No Linux Pico SDK is
   needed. Use the repository on a Windows-mounted drive as in this bench.
3. Connect the scope USB to Windows. Check ownership; detach from WSL if needed.
   Run the preferred status command and require READY/0 with the correct model
   and backend. Repeat independent checks to establish reliable reopen.
4. With ESP32 unpowered and all loads disconnected, fit/verify the external
   GPIO5 pull-down using the DMM. Select x10 on the probe. Connect A, DC,
   ±5 V probe-tip range. For initial LOW validation, connect tip **and** ground
   clip to board GND and collect the baseline below.
5. Keeping ESP32 unpowered, move only the tip to GPIO5 and repeat the baseline.
   Investigate unexpected noise, clipping, missing samples or HIGH regions.
6. Disconnect both probe connections from ESP32. Connect the isolated generator
   loop described above and run the known-signal checks below. Verify scaling,
   trigger direction, width analysis, expected timeout and zero after cleanup.
7. Disconnect the generator loop. Return the probe to GPIO5/GND while the board
   is unpowered. Recheck LOW. Then follow the specific powered GPIO/HIL procedure,
   including capture coverage, instrument limits and required reset counts.
   Instrument validation alone does not authorize a pump or complete HIL.

During guided bench work, give one physical action at a time and wait for the
operator's confirmation. A restart/replug is a reason to rediscover ownership,
not to reuse a remembered BUSID or assume an old process still owns the scope.

## Capture configuration and scaling

Manual baseline context: A/DC, x10 probe, ±5 V displayed, **1 ms/div**. The
matching automated LOW window is 10 ms. The requested 16,000-sample target
produced **15,625 samples at 1.5625 MS/s**, or **640 ns/sample**.

These three quantities must remain separate:

- Physical probe attenuation: tip voltage is divided by 10 before Channel A.
- API input range: select **±0.5 V** (`PS2000_500MV`, index 5).
- CLI/report range: `--range-v 5` means **±5 V at the probe tip**.

Conversion is `tip_volts = adc * 5 / 32767` for this range. Equivalently,
`adc * 0.5 / 32767 * 10`; never multiply the first expression by 10 again.
The ADC is physically 8-bit; the API scales samples to signed 16-bit values.
This format does not create 16-bit analog resolution. Trigger conversion is
the inverse; metadata records requested and quantized ADC/voltage thresholds.

`ps2000_get_timebase` returns interval in **nanoseconds**. Its separate time-unit
enum is a suggested unit for timestamp retrieval, not another multiplier for
that interval. The CLI chooses the fastest available timebase covering the
requested duration within target count/device memory. Actual rate, count,
span, duration and nominal pretrigger position must be reviewed per capture.

The **0.8 V candidate alert threshold** is about 11.7× the measured positive
LOW extreme, below nominal ESP32 3.3 V HIGH, and works on the nominal 2 V test
signal. It is not a proven MOSFET switching threshold, not a universal HIL
acceptance limit, and not permission to ignore smaller transients. Reassess
it after probe/ground/range changes and actual GPIO HIGH verification.

## Baseline and evidence commands

After confirming the corresponding physical setup:

```sh
./scripts/picoscope-capture.sh --count 50 --range-v 5 --duration-ms 10 --samples 16000 --pulse-v 0.8 --json

# Explicit new evidence directory: choose an unused name for every run.
./scripts/picoscope-capture.sh --count 50 --range-v 5 --duration-ms 10 --samples 16000 --pulse-v 0.8 --output local/evidence/picoscope/my-new-low-run --json
```

| Verified setup | Blocks | Observed extrema | >0.8 V pulses |
|---|---:|---|---:|
| Unpowered board, GND-to-GND | 50 × 10 ms | −68.514 to +68.514 mV | 0 |
| Unpowered GPIO5 + 10 kΩ | 50 × 10 ms | −68.514 to +68.514 mV | 0 |

Each set contains 781,250 samples at the rate above. Raw CSV/metadata agreement,
clipping, overflow and lost-data checks passed. Blocks have dead time between
them: **these are not continuous boot/reset coverage or HIL evidence**.
Local evidence under `local/evidence/picoscope/`:
`20261031T232620Z-_ld1lxgx/` (GND) and `20261031T232717Z-ex63cxc4/` (GPIO5).
Directory names use UTC; these captures occurred on 2026-11-01 local bench time.
Exact assertions and subsequent powered measurements belong in the M9 report.

An independent powered static check used the PeakTech 4390 in DC voltage mode
(red on GPIO5, black on board GND): approximately -0.3 mV, as reported by the
operator. This agrees with steady LOW scope captures on the production image.
It validates static OFF only; a DMM reading never replaces transient capture.
The operator subsequently clarified that the display was still connected for
that DMM reading and the 3V3 rail measurement of 3.297 V. After powered-down
display removal and USB reconnection, three new steady GPIO5 blocks measured
-22.736..+22.889 mV with no >0.8 V pulses. Do not retroactively describe the
earlier powered DMM readings as display-free or attribute the noise difference
to the display without a controlled test. Detailed setup/evidence is in M9.

Default evidence is ignored `local/evidence/picoscope/<unique-run>/`: request
JSON, metadata JSON, per-block metadata/pulses, and lossless ADC CSV with
sample/time offsets. `--output` requires a new directory. Keep large raw files
untracked; preserve selected summaries/references in milestone reports.
`armed.txt` and stderr ARMED notices expose capture arming. Host UTC is not an
absolute timestamp for every sample. A rapid multi-block run may finish blocks
between supervisor notifications; each block has its own metadata.

## Known-signal check (isolated loop only)

```sh
./scripts/picoscope-capture.sh --test-signal square-1khz --trigger rising --trigger-v 0.8 --pulse-v 0.8 --pre-ms 0.25 --post-ms 4.75 --count 3 --json
./scripts/picoscope-capture.sh --test-signal square-1khz --trigger falling --trigger-v 0.8 --pulse-v 0.8 --pre-ms 0.25 --post-ms 4.75 --count 3 --json

# Negative trigger check: expected exit 22, no waveform (do not treat as a pass trace).
./scripts/picoscope-capture.sh --test-signal square-1khz --trigger rising --trigger-v 3 --pre-ms 0.25 --post-ms 4.75 --timeout-s 1 --json

# Verify zero after cleanup without enabling the generator again.
./scripts/picoscope-capture.sh --duration-ms 10 --count 3 --json
```

Nominal signal: 0..2 V, 1 kHz. Verified median HIGH plateau ≈2.033 V;
complete HIGH widths 500.16..500.80 µs at 320 ns/sample. Rising blocks each
contained five complete pulses; falling blocks each contained four complete
pulses plus two boundary-censored regions. The 3 V trigger timed out as expected;
three subsequent untriggered blocks measured approximately ±22.9 mV with no
>0.8 V pulses. Detailed evidence is in the M9 report's Windows CLI validation.

Observed edge extrema **−0.205 to +2.216 V** are retained, not dismissed as
noise. Generator and acquisition share an instrument; this is a functional
cross-check, not independent traceable calibration. The generator's specified
rise time is <2 µs ([vendor datasheet](https://www.picotech.com/download/datasheets/picoscope-2000-series-data-sheet-en.pdf)).
This check neither isolates the cause of overshoot nor proves full-bandwidth
probe response/compensation or the shortest detectable GPIO pulse.

Normal status/capture never enables the generator. Explicit `--test-signal`
is capture-only and restores DC zero before normal close. A killed/hung worker
cannot guarantee cleanup. Keep the loop isolated until zero output is verified;
`--test-signal dc-zero` is available for explicit recovery once reliable opening
is restored. Do not reconnect a possibly live generator to the board.

## Validity and recovery

Hardware rising/falling triggering has auto-trigger **disabled**. Trigger timeout
produces no waveform and cannot establish a measured zero-pulse trace. Samples
that overflow/clip, contain lost-data markers, or are incomplete cannot be valid
evidence. A HIGH region cut off by a capture boundary has a null complete width;
its observed interval is only a bound. Short-pulse claims must respect actual
sample interval, analog bandwidth, probe response and trigger/observation gaps.

For an open/worker failure:

1. Stop further opens. Preserve metadata, logs and exit code. Close any GUI or
   unrelated capture process; inspect USB ownership using `-OwnershipOnly`.
2. Inspect worker PIDs. The native supervisor reports its worker PID and whether
   it exited after timeout; it never silently retries. In WSL inspect commands
   for `picoscope_status.py --probe-child` or legacy `m9-pico-open` probes.
   The launcher checks these and takes `~/.cache/rhizoedge/picoscope-ps2000.lock`;
   Windows workers hold `Local\RhizoEdge.PicoScope.ps2000` through close.
3. **Do not delete a lock file to bypass a live worker.** File presence alone is
   not a stale lock: advisory ownership ends when its last holder exits. Inspect
   the holder first. An inherited lock deliberately blocks new opens if a killed
   Linux worker remains in a kernel wait.
4. If USB reset/descriptor timeouts leave a worker in D state, stop and have the
   operator unplug **only the scope**. Confirm the worker exits, then reconnect
   in a separate confirmed action. Do not retry a busy attach, force-bind, cycle
   unrelated USB devices, or assume process kill released a kernel USB operation.
5. Rediscover the current device. Return it to Windows if attached to WSL, then
   run native readiness. If a fault repeats, investigate SDK/driver/transport;
   do not loop retries until green. The failed Linux path is not the fallback.

Run commands outside agent sandboxes that hide USB or Windows interoperability;
a sandbox-only missing-device result does not establish physical disconnection.
On Windows NOT SHARED is acceptable for measurement; do not bind it merely to
make ownership output change. Driver/API errors require the correct x64 ps2000
SDK and dependencies, not substitution with ps2000a.

## Maintenance and references

After tool changes, use focused tests rather than repeatedly running the project
exit gate: `scripts/test_picoscope_cli.py`, `scripts/test_picoscope_status.py`,
`scripts/test_picoscope_watchdog.py` (from WSL), and
`scripts/test-picoscope-windows.ps1` (Windows PowerShell). The last two require
Windows interoperability; these software tests do not open the scope. Repeat
physical checks when a change affects scaling, timing, triggering or cleanup.

- [Official programmer guide, preserved PDF](../vendor/pico/picoscope-2000-series-programmers-guide.pdf)
- [Vendor provenance, checksum and notices](../vendor/pico/README.md)
- [CLI reference entry point](../testing/picoscope-cli.md)
- [Home-node hardware guide](home-node-hardware-guide.md)
- [M9 report: actual milestone evidence and pending criteria](../reports/M9.md)

Update this runbook whenever verified setup/behavior/limits change. Keep raw
waveforms local, summarize reusable facts here, and record the acceptance
implications in the milestone report. No runbook edit alone closes a criterion.

## Continuous capture for boot/reset coverage (2026-11-01)

Short block acquisitions have gaps and cannot demonstrate the whole boot window.
The Windows backend now supports non-aggregated fast streaming, leaving the
probe on GPIO5 and the generator disabled:

```sh
./scripts/picoscope-capture.sh --mode stream --duration-ms 2000 \
  --stream-interval-ns 1000 --timeout-s 5 --json
```

Streaming requires one acquisition, no trigger, no pre-trigger and no test
signal. Duration and interval determine the sample count (maximum 100 million).
It saves `capture-0001.i16le`: headerless signed little-endian int16 ADC samples.
Reconstruct volts as `adc * range_v / 32767`; `range_v` is already probe-tip
range, so do not multiply by ten again. Metadata contains interval, count,
analysis, overrun, auto-stop and contiguous-completion checks. The accepted
stream interval is supplied to the driver; ps2000 provides no independent
interval readback. Callback buffers use aggregation 1. A timeout, missing or
excess samples, overrun, clipping or missing-data marker cannot pass.

The CLI announces ARMED only after samples arrive. For an event capture, start
acquisition before requesting the event; verify that the event and complete
recovery fit within the saved waveform. A successful steady capture alone does
not establish boot/reset safety. Streaming at 1 µs intervals cannot exclude
sub-microsecond pulses; pulse widths are sample-quantized. Record this limit
with each HIL observation, and do not claim unlimited transient sensitivity.

Implementation correction: ps2000pg.en-12 §5.14 reverses the return convention
for `ps2000_overview_buffer_status`. [Pico acknowledged this documentation
error](https://www.picotech.com/support/viewtopic.php?t=31221). The Windows
backend checks positive success, while separately rejecting the overrun flag;
it does not ignore failed status calls. The original PDF remains unmodified.

Focused real-device validation saved 2,000,000 samples over 2 s at accepted
1 µs intervals, auto-stopped, with no overrun, missing samples or clipping.
GPIO5 extrema were ±68.514 mV, zero crossings above 0.8 V. Evidence:
`local/evidence/picoscope/20261101T104251Z-_d4igc3c/`. This validates streaming
transport on the unchanged setup; it is **not a HIL trial**. The failed initial
status interpretation remains recorded in `20261101T102405Z-m5hyd0d9/`.

### Long-stream overrun investigation (2026-11-01)

The first 90 s cold-boot stream is **invalid**, not HIL evidence:
`local/evidence/picoscope/m9-cold-boot-20261101-01/`. All 90 million sample
positions were returned, but the driver reported overview-buffer overrun and
inserted 1,927 missing-data markers (`-32768`), starting at sample 86,544,674.
The apparent -5.00015 V minimum is that sentinel converted by the generic
analysis; it is **not an observed negative GPIO transient**. The invalid flag
must take precedence over voltage summaries and zero pulse counts. Raw data,
loss intervals, confirmation time and assessment are preserved together.

The streaming callback now copies native samples directly into the preallocated
array rather than allocating/copying an intermediate array for every callback.
The overview buffer is increased from 250,000 to 1,000,000 samples. Metadata
adds maximum polling gap, first overrun time, buffer size and callback count.
These changes reduce allocation pressure and increase buffering tolerance;
they do not establish what originally delayed data handling.

One stationary 90 s diagnostic with those changes completed all 90 million
samples with no loss or overrun; largest observed polling gap 46.3374 ms,
1,966 callbacks, extrema ±68.514 mV and no >0.8 V pulses. Evidence:
`local/evidence/picoscope/m9-stream-loss-investigation-01/`.
This is a diagnostic, not a repeated boot test or proof of a permanent fix.
The original scheduling/driver loss cause remains unresolved; do not rerun
physical safety trials until green or silently discard the failed acquisition.
