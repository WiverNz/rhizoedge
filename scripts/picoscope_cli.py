#!/usr/bin/env python3
"""WSL orchestrator for the Windows x64 ps2000 backend. No Linux SDK calls."""
import argparse
import fcntl
import json
import math
import os
from pathlib import Path
import subprocess
import sys
import tempfile
from datetime import datetime, timezone

ROOT = Path(__file__).resolve().parent.parent


def winpath(path):
    return subprocess.check_output(['wslpath', '-w', str(Path(path).resolve())], text=True, timeout=5).strip()


def linux_workers():
    found = []
    for proc in Path('/proc').glob('[0-9]*'):
        try:
            args = (proc / 'cmdline').read_bytes().split(b'\0')
            # Check argv elements, not arbitrary text in parent shell commands.
            if any(Path(os.fsdecode(a)).name.startswith('m9-pico-open') for a in args if a) or (
                b'--probe-child' in args and any(Path(os.fsdecode(a)).name == 'picoscope_status.py' for a in args if a)
            ):
                found.append(int(proc.name))
        except (OSError, ValueError):
            pass
    return found


def parse(argv=None):
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('action', choices=['status', 'capture'])
    p.add_argument('--json', action='store_true')
    p.add_argument('--block-limits', action='store_true', help='Status only: query real single/dual-channel ps2000 timebases and maximum counts; restores B off')
    p.add_argument('--mode', choices=['block', 'stream'], default='block')
    p.add_argument('--timebase', type=int, help='Fixed block timebase index; requires --block-samples, never falls back to slower sampling')
    p.add_argument('--block-samples', type=int, help='Exact block count at --timebase; duration derives from the real driver interval')
    p.add_argument('--channels', choices=['A','AB'], default='A')
    p.add_argument('--trigger-source', choices=['A','B'], default='A')
    p.add_argument('--stream-interval-ns', type=int, default=1000, help='Streaming interval accepted by ps2000; no independent readback')
    p.add_argument('--diagnostic-default-timer', action='store_true', help='Diagnostic comparison only: omit the process-scoped 1 ms timer request')
    p.add_argument('--diagnostic-readback', action='store_true', help='After stop, compare and preserve the driver retained raw sequence; never fills missing callback samples')
    p.add_argument('--test-signal', choices=['square-1khz', 'dc-zero'], help='ISOLATED probe-to-AWG loop only; restores DC zero on normal close')
    p.add_argument('--dll', help='Optional installed Windows x64 ps2000.dll path')
    p.add_argument('--range-v', type=float, default=5, help='Probe-tip +/- range; physical x10 probe required')
    p.add_argument('--duration-ms', type=float, default=10)
    p.add_argument('--samples', type=int, default=16000, help='Upper sample-count target; actual timebase is queried')
    p.add_argument('--count', type=int, default=1, help='Separate block acquisitions (with gaps)')
    p.add_argument('--trigger', choices=['none', 'rising', 'falling'], default='none')
    p.add_argument('--trigger-v', type=float, default=0.8)
    p.add_argument('--pulse-v', type=float, default=0.8, help='Analysis threshold, provisional until bench validation')
    p.add_argument('--pre-ms', type=float, default=0)
    p.add_argument('--post-ms', type=float, help='If set, duration = pre-ms + post-ms')
    p.add_argument('--timeout-s', type=float, default=10, help='Per-acquisition deadline; triggered timeout has no waveform')
    p.add_argument('--open-timeout-s', type=float, default=20, help='Additional total budget for compilation/open/close')
    p.add_argument('--output', type=Path, help='New evidence directory; default local/evidence/picoscope/<unique run>')
    a = p.parse_args(argv)
    if (a.timebase is None) != (a.block_samples is None):
        p.error('--timebase and --block-samples are required together')
    if a.timebase is not None and (a.action != 'capture' or a.mode != 'block' or not 0 <= a.timebase <= 23 or not 2 <= a.block_samples <= 1000000):
        p.error('Invalid fixed block configuration')
    if a.trigger_source == 'B' and (a.channels != 'AB' or a.trigger == 'none'):
        p.error('B trigger requires --channels AB and rising/falling edge')
    if a.channels != 'A' and (a.action != 'capture' or a.mode != 'block'):
        p.error('Dual channel capture is supported only in block mode')
    if a.block_limits and a.action != 'status':
        p.error('--block-limits requires status')
    if (a.diagnostic_default_timer or a.diagnostic_readback) and (a.action != 'capture' or a.mode != 'stream'):
        p.error('Streaming diagnostics require capture --mode stream')
    if a.test_signal and a.action != 'capture':
        p.error('--test-signal requires capture, never status')
    if a.post_ms is not None:
        if a.post_ms <= 0:
            p.error('--post-ms must be positive')
        a.duration_ms = a.pre_ms + a.post_ms
    numbers = [a.range_v, a.duration_ms, a.pre_ms, a.timeout_s, a.open_timeout_s, a.trigger_v, a.pulse_v]
    if not all(math.isfinite(x) for x in numbers):
        p.error('Numeric options must be finite')
    if a.range_v not in [.2, .5, 1, 2, 5, 10, 20, 50, 100, 200]:
        p.error('Unsupported x10 probe-tip range')
    if not 2 <= a.samples <= 1000000 or not 1 <= a.count <= 1000:
        p.error('Require 2..1000000 samples and 1..1000 acquisitions')
    if a.timebase is None and (not 0 < a.duration_ms < a.timeout_s*1000 or not 0 <= a.pre_ms < a.duration_ms):
        p.error('Require 0 <= pre < duration < capture timeout')
    if not 1 <= a.open_timeout_s <= 60 or not 0 < a.timeout_s <= 3600:
        p.error('Invalid timeout')
    if a.pre_ms < 0:
        p.error('Pretrigger must be nonnegative')
    if a.timebase is not None and a.post_ms is not None:
        p.error('Fixed block count cannot be combined with --post-ms')
    if a.trigger == 'none' and a.pre_ms != 0:
        p.error('Pretrigger requires a rising/falling trigger')
    if abs(a.trigger_v) >= a.range_v or abs(a.pulse_v) >= a.range_v:
        p.error('Thresholds must lie inside the probe-tip range')
    if a.mode == 'stream':
        if a.action != 'capture' or a.count != 1 or a.trigger != 'none' or a.pre_ms != 0 or a.test_signal:
            p.error('Streaming requires one untriggered capture, no test signal, no pretrigger')
        if not 100 <= a.stream_interval_ns <= 1000000:
            p.error('Streaming interval must be 100..1000000 ns')
        n = math.ceil(a.duration_ms * 1e6 / a.stream_interval_ns)
        if not 2 <= n <= 100000000:
            p.error('Streaming count must be 2..100000000 samples')
    return a


def main(argv=None):
    a = parse(argv)
    workers = linux_workers()
    if workers:
        print(json.dumps(dict(backend='windows-ps2000', ready=False, status='LINUX WORKER ACTIVE', exit_code=21, pids=workers)))
        return 21
    # Share the old helper's lock throughout the Windows invocation. No Linux
    # device API is loaded; a concurrent legacy invocation must refuse to open.
    lockpath = Path.home() / '.cache/rhizoedge/picoscope-ps2000.lock'
    lockpath.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    with lockpath.open('a') as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            print(json.dumps(dict(backend='windows-ps2000', ready=False, status='LINUX WORKER ACTIVE', exit_code=21)))
            return 21
        base = ROOT / 'local/evidence/picoscope'
        base.mkdir(parents=True, exist_ok=True)
        if a.output:
            directory = a.output.resolve()
            directory.mkdir(parents=True, exist_ok=False)
        else:
            directory = Path(tempfile.mkdtemp(prefix=datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%SZ-'), dir=base))
        request = dict(fixed_timebase=a.timebase if a.timebase is not None else -1, fixed_count=a.block_samples or 0, channels=a.channels, trigger_source=a.trigger_source, block_limits=a.block_limits, high_resolution_timer=not a.diagnostic_default_timer, diagnostic_readback=a.diagnostic_readback, action=a.action, mode=a.mode, stream_interval_ns=a.stream_interval_ns, test_signal=a.test_signal, output_dir=winpath(directory), result_file=winpath(directory/'metadata.json'),
                       dll=a.dll, range_v=a.range_v, duration_s=a.duration_ms/1000, samples=a.samples,
                       count=a.count, trigger=a.trigger, trigger_v=a.trigger_v, pulse_v=a.pulse_v,
                       pre_s=a.pre_ms/1000, timeout_s=a.timeout_s,
                       process_timeout_s=a.open_timeout_s+(a.timeout_s*a.count if a.action=='capture' else 0))
        reqfile = directory/'request.json'
        reqfile.write_text(json.dumps(request, indent=2)+'\n')
        # Native supervisor enforces the deadline and owns Windows child cleanup.
        child = subprocess.run(['powershell.exe', '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File',
                                winpath(ROOT/'scripts/picoscope-windows.ps1'), '-RequestFile', winpath(reqfile)],
                               stdout=subprocess.PIPE, text=True)
        try:
            result = json.loads(child.stdout.lstrip('\ufeff'))
        except ValueError:
            print(json.dumps(dict(backend='windows-ps2000', ready=False, status='INVALID BACKEND REPORT', exit_code=21)))
            return 21
        result['evidence_directory'] = str(directory)
        if a.json:
            print(json.dumps(result, indent=2))
        else:
            print(f"{result['status']} [{result['backend']}]: model={result.get('model', 'unverified')}")
            if result.get('message'):
                print(result['message'])
            for capture in result.get('captures', []):
                analysis = capture.get('analysis') or {}
                print(f"capture {capture['acquisition']}: {capture['status']}; samples={capture['sample_count']}; "
                      f"rate={capture['sample_rate_hz']:g} Hz; triggered={capture.get('triggered')}; "
                      f"min/max={analysis.get('min_v')}/{analysis.get('max_v')} V; "
                      f"pulses={analysis.get('pulse_count')}; max_width={analysis.get('max_pulse_width_s')} s")
            print(f'Evidence: {directory}')
        return int(result['exit_code'])


if __name__ == '__main__':
    sys.exit(main())
