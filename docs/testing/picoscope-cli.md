# M9 PicoScope CLI (Windows ps2000, controlled from WSL)

Preferred backend: **windows-ps2000**, PicoScope 2205A, Windows x64 ps2000 DLL.
No GUI is used. Leave the scope owned by Windows; neither binding nor WSL
attachment is required. The direct Linux ps2000/USB-IP path failed repeated
opens, independently in Python and C, and is not the measurement path.

## State of validation

Software checks pass, but **real Windows device open and waveform accuracy are
not yet verified**. Do not use generated captures to close M9 criteria until
LOW, known-HIGH scaling, trigger and pulse-width checks pass. No HIL has begun.

## Commands

From WSL, outside sandboxes that hide Windows interoperability:

```sh
./scripts/picoscope-status.sh --json
./scripts/picoscope-capture.sh --help
```

`READY` requires the actual Windows ps2000 API to open the 2205A, query its model
and serial (serial is not printed), configure A/DC and disable B, and close
successfully. USB presence alone is insufficient. The response always identifies
its backend. Windows can run `scripts/picoscope-status.ps1 -Json` directly.
Use `-OwnershipOnly` for USB discovery without opening the device.

If ownership reports ATTACHED TO WSL, use the existing identity-discovering
`scripts/picoscope-wsl-detach.ps1` from Windows PowerShell. It leaves the device
shared. No force-bind, implicit elevation, or automatic open retries occur.
The attach/detach helpers remain available for diagnostics, but attachment is
not part of normal measurement operation. The legacy Linux diagnostic is
`scripts/picoscope_status.py`; do not run it in parallel or retry the failed
Linux path. The WSL launcher checks for old workers and takes their advisory
lock; Windows workers also hold a named mutex through close.

Once the physical setup for each stage has been confirmed, examples are:

```sh
# 50 separate 10 ms LOW captures; NOT continuous coverage.
./scripts/picoscope-capture.sh --count 50 --range-v 5 --duration-ms 10 --samples 16000 --pulse-v 0.8 --json

# Example triggered capture only; not an approved HIL threshold/procedure.
./scripts/picoscope-capture.sh --trigger rising --trigger-v 0.8 --pre-ms 0.1 --post-ms 0.4 --timeout-s 10 --pulse-v 0.8 --json
```

`--range-v` is the **probe-tip** range, with physical x10 fixed: 5 means
±5 V at the tip and ±0.5 V at the scope input. Voltage is ADC × tip range /
32767, exactly one attenuation factor. Trigger conversion is the inverse;
metadata records requested and quantized threshold. ADC samples are stored
unchanged. Driver timebase intervals are nanoseconds, regardless of the
separate suggested timestamp-unit enum. See the installed ps2000 header and
[Pico's ps2000 programmer guide](https://www.picotech.com/download/manuals/picoscope-2000-series-programmers-guide.pdf).

The driver selects the fastest available timebase covering the requested
duration within the sample target and device memory. Actual interval, rate,
count, duration, span, and nominal pretrigger position are recorded. This is
block acquisition, without ETS or oversampling. Longer windows can reduce
sample rate: review it against each criterion's required pulse resolution.
No absence-of-pulse claim may exceed the capture bandwidth or coverage.

`--trigger none` is an untriggered block. Rising/falling use a hardware trigger
with auto-trigger **disabled**. A trigger deadline returns exit 22, no waveform,
and invalid capture; it is not a measured zero-pulse trace. Censored pulses
(start/end outside the block) have null full width and explicit boundary flags.
Complete widths use sample crossings, with sample-interval quantization and
additional analog/trigger uncertainty. Clipping, overflow, lost samples and
incomplete data cannot produce valid evidence. The provisional 0.8 V default
must be reviewed against measured LOW noise and known 3.3 V HIGH; it is **not**
an accepted HIL threshold yet.

Evidence defaults to ignored `local/evidence/picoscope/<unique run>/`:
request JSON, aggregate metadata JSON, per-block metadata/pulse analysis and
lossless ADC CSV with sample/time offsets. `armed.txt` identifies acquisition
arming; host UTC is not an absolute timestamp for each sample. An ARMED notice
is emitted on stderr while waiting. Explicit `--output` must name a new directory;
existing evidence is never overwritten. Large raw waveforms stay untracked.

A native supervisor enforces the total worker budget (open/compile/close budget
plus per-acquisition deadlines). A killed/stalled worker returns failure, not
READY; partial files are not valid evidence. Stop on such failures and inspect
process state before attempting another open. Status codes include 0 READY,
11 NOT CONNECTED, 13 ATTACHED TO WSL, 16 MULTIPLE DEVICES, 20 DRIVER/API MISSING,
21 backend/open failure, 22 acquisition timeout and 23 invalid capture. The
ownership-only view additionally distinguishes NOT SHARED/18 and WINDOWS
AVAILABLE/19; either ownership state can be usable by the Windows driver.

## Focused software checks

```sh
PYTHONDONTWRITEBYTECODE=1 python3 scripts/test_picoscope_cli.py
PYTHONDONTWRITEBYTECODE=1 python3 scripts/test_picoscope_status.py
python3 scripts/test_picoscope_watchdog.py
```

Run `scripts/test-picoscope-windows.ps1` using Windows PowerShell for native
math/JSON/discovery tests. These tests do not invoke the device API. The watchdog
test runs the real supervisor with a sleeping worker copy in ignored local
storage, verifying timeout and process termination without USB access.
