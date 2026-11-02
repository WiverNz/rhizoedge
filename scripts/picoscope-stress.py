#!/usr/bin/env python3
"""Fixed stationary ps2000 continuity matrix. Never resets/actuates the ESP32."""
import argparse
from array import array
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parent.parent


def plan(profile='baseline'):
    if profile == 'hil':
        return [(seconds, 1000, repetition) for seconds in (10, 30, 90) for repetition in (1, 2, 3)]
    return [(seconds, interval, repetition)
            for seconds in (10, 30, 90)
            for repetition in (1, 2, 3)
            for interval in (1000, 2000)] + [(10, 500, r) for r in (1, 2)]


def audit_raw(directory, capture):
    raw = directory / capture['raw_i16']
    digest = hashlib.sha256()
    lost = count = 0
    with raw.open('rb') as stream:
        while chunk := stream.read(2 * 1024 * 1024):
            digest.update(chunk)
            samples = array('h')
            samples.frombytes(chunk)
            if sys.byteorder != 'little':
                samples.byteswap()
            count += len(samples)
            lost += samples.count(-32768)
    return dict(samples=count, missing_markers=lost, sha256=digest.hexdigest(),
                agrees_with_metadata=count == capture['sample_count'] and lost == capture['lost_sample_count'])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--profile', choices=['baseline', 'hil'], default='baseline')
    parser.add_argument('--diagnostic-readback', action='store_true')
    args = parser.parse_args()
    matrix = plan(args.profile)
    directory = args.output.resolve()
    directory.mkdir(parents=True, exist_ok=False)
    sources = ('PicoScopeWindows.cs', 'picoscope-windows.ps1', 'picoscope_cli.py', 'picoscope-stress.py')
    manifest = dict(started_utc=datetime.now(timezone.utc).isoformat(),
                    purpose='stationary GPIO5 reliability diagnostics; NO HIL counts',
                    matrix=matrix, profile=args.profile, diagnostic_readback=args.diagnostic_readback, source_sha256={name: hashlib.sha256((ROOT/'scripts'/name).read_bytes()).hexdigest() for name in sources}, runs=[])
    (directory/'plan.json').write_text(json.dumps(manifest, indent=2)+'\n')
    for index, (seconds, interval, repetition) in enumerate(matrix, 1):
        run = directory/f'{index:02d}-{seconds}s-{interval}ns-r{repetition}'
        cmd = [str(ROOT/'scripts/picoscope-capture.sh'), '--mode', 'stream', '--duration-ms', str(seconds*1000),
               '--stream-interval-ns', str(interval), '--timeout-s', str(seconds+15), '--output', str(run), '--json']
        if args.diagnostic_readback:
            cmd.append('--diagnostic-readback')
        print(f'START {index}/{len(matrix)}: {seconds}s interval={interval}ns repetition={repetition}', flush=True)
        started = time.monotonic()
        proc = subprocess.run(cmd, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        row = dict(index=index, seconds=seconds, interval_ns=interval, repetition=repetition,
                   command=cmd, exit_code=proc.returncode, orchestrator_wall_s=time.monotonic()-started)
        # Backend normally creates the directory. Preserve failure output even when it did not.
        run.mkdir(exist_ok=True)
        (run/'orchestrator.stdout.json').write_text(proc.stdout)
        (run/'orchestrator.stderr.txt').write_text(proc.stderr)
        try:
            report = json.loads(proc.stdout)
            capture = report['captures'][0]
            row['capture'] = {k: v for k, v in capture.items() if k != 'analysis'}
            row['analysis'] = {k: v for k, v in capture['analysis'].items() if k != 'pulses'}
            row['raw_audit'] = audit_raw(run, capture)
            row['clean'] = proc.returncode == 0 and capture['valid'] and row['raw_audit']['agrees_with_metadata'] and row['raw_audit']['missing_markers'] == 0
            row['observed_pulses'] = capture['analysis']['pulse_count']
        except (KeyError, ValueError, OSError, TypeError) as error:
            row.update(clean=False, report_error=str(error))
        manifest['runs'].append(row)
        (directory/'summary.json').write_text(json.dumps(manifest, indent=2)+'\n')
        print(f"END {index}/{len(matrix)}: clean={row['clean']} exit={proc.returncode} audit={row.get('raw_audit')}", flush=True)
        # Every scheduled run is retained; never replace a failure with a retry.
        if row.get('observed_pulses', 0):
            print('Unexpected GPIO threshold crossing; stop matrix for investigation.', flush=True)
            break
    manifest['completed_utc'] = datetime.now(timezone.utc).isoformat()
    manifest['all_clean'] = len(manifest['runs']) == len(matrix) and all(r['clean'] for r in manifest['runs'])
    (directory/'summary.json').write_text(json.dumps(manifest, indent=2)+'\n')
    return 0 if manifest['all_clean'] else 1


if __name__ == '__main__':
    sys.exit(main())
