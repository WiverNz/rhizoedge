#!/usr/bin/env python3
"""LEGACY Linux ps2000 diagnostic; direct USB/IP path failed on this host.
Preferred readiness: ./scripts/picoscope-status.sh (Windows backend).
No capture or signal generation.

Codes: 0 READY; 2 tool error; 10 usbipd missing; 11 NOT CONNECTED;
12 PERMISSION ERROR; 13 ATTACHED TO WSL (ownership only); 16 MULTIPLE DEVICES;
18 NOT SHARED; 19 WINDOWS AVAILABLE; 20 DRIVER/API MISSING; 21 DEVICE OPEN FAILED.
Opening ps2000 may reset/re-enumerate USB. Never retries or attaches implicitly.
"""
import argparse
import ctypes as c
import ctypes.util
import json
import fcntl
import os
from pathlib import Path
import subprocess
import sys


def usb_nodes(base=Path('/sys/bus/usb/devices')):
    nodes = []
    for path in base.iterdir():
        try:
            if (path / 'idVendor').read_text().strip() != '0ce9' or (path / 'idProduct').read_text().strip() != '1007':
                continue
            bus = int((path / 'busnum').read_text())
            dev = int((path / 'devnum').read_text())
            nodes.append(Path(f'/dev/bus/usb/{bus:03d}/{dev:03d}'))
        except (OSError, ValueError):
            continue
    return nodes


def load_api():
    library = ctypes.util.find_library('ps2000')
    if not library:
        raise OSError('libps2000 is not in the dynamic linker cache')
    api = c.CDLL(library)
    signatures = {
        'open_unit': [],
        'get_unit_info': [c.c_int16, c.c_char_p, c.c_int16, c.c_int16],
        'set_channel': [c.c_int16] * 5,
        'close_unit': [c.c_int16],
    }
    for name, arguments in signatures.items():
        fn = getattr(api, 'ps2000_' + name)
        fn.argtypes = arguments
        fn.restype = c.c_int16
    return api, library


def probe():
    api, library = load_api()
    handle = api.ps2000_open_unit()
    if handle <= 0:
        raise RuntimeError(f'ps2000_open_unit returned {handle}')
    try:
        def info(index):
            buf = c.create_string_buffer(256)
            if api.ps2000_get_unit_info(handle, buf, len(buf), index) <= 0:
                raise RuntimeError(f'ps2000_get_unit_info({index}) failed')
            return buf.value.decode('ascii', errors='replace')
        model = info(3)
        if model != '2205A':
            raise RuntimeError(f'Expected 2205A, got {model}')
        serial_queried = bool(info(4))  # Never emit the device identifier.
        if api.ps2000_set_channel(handle, 0, 1, 1, 5) <= 0:
            raise RuntimeError('Channel A configuration failed')
        if api.ps2000_set_channel(handle, 1, 0, 1, 5) <= 0:
            raise RuntimeError('Channel B disable failed')
        return dict(model=model, serial_queried=serial_queried, driver_version=info(0), library=library,
                    channel='A', coupling='DC', input_range_v=0.5, probe_attenuation=10, probe_tip_range_v=5)
    finally:
        if api.ps2000_close_unit(handle) <= 0:
            raise RuntimeError('ps2000_close_unit failed')


def run_bounded(command, timeout, lock_path):
    """Bound user-space waiting even if SIGKILL cannot release a kernel USB wait.

    The child inherits the flock, so a stranded worker prevents another open.
    Never use subprocess.run/context-manager cleanup here: both can wait forever
    for an uninterruptible child after the timeout.
    """
    lock_path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    fd = os.open(lock_path, os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600)
    try:
        try:
            fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            return None, 'Another ps2000 worker still holds the device lock. Do not retry; inspect that worker or disconnect the scope.'
        child = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                 text=True, pass_fds=(fd,))
        try:
            stdout, _stderr = child.communicate(timeout=timeout)
        except subprocess.TimeoutExpired:
            child.kill()
            # No communicate()/wait() after kill: D-state tasks cannot exit yet.
            child.stdout.close()
            child.stderr.close()
            alive = child.poll() is None
            detail = f' Worker PID {child.pid} has not exited; its lock blocks subsequent opens until it exits.' if alive else ''
            return None, f'ps2000 open/query/close exceeded {timeout:g}s; termination requested.' + detail
        try:
            result = json.loads(stdout)
        except ValueError:
            return None, 'ps2000 worker failed without a valid report.'
        if not isinstance(result, dict):
            return None, 'ps2000 worker returned an invalid report.'
        if child.returncode:
            return None, result.get('error', 'ps2000 worker failed')
        if result.get('model') != '2205A':
            return None, 'ps2000 worker did not verify model 2205A.'
        return result, None
    finally:
        # Do not LOCK_UN: that would unlock the child's shared open description.
        os.close(fd)


def bounded_probe(timeout):
    lock = Path.home() / '.cache' / 'rhizoedge' / 'picoscope-ps2000.lock'
    return run_bounded([sys.executable, str(Path(__file__).resolve()), '--probe-child'], timeout, lock)


def windows_status():
    script = Path(__file__).with_name('picoscope-status.ps1').resolve()
    winpath = subprocess.check_output(['wslpath', '-w', str(script)], text=True, timeout=5).strip()
    child = subprocess.run(['powershell.exe', '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', winpath, '-Json', '-OwnershipOnly'],
                           capture_output=True, text=True, timeout=15)
    return json.loads(child.stdout)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--json', action='store_true')
    parser.add_argument('--timeout', type=float, default=20, help='Bound real driver probe in seconds (1..60).')
    parser.add_argument('--probe-child', action='store_true', help=argparse.SUPPRESS)
    args = parser.parse_args()
    if not 1 <= args.timeout <= 60:
        parser.error('--timeout must be between 1 and 60 seconds')
    if args.probe_child:
        try:
            print(json.dumps(probe()))
            return 0
        except (OSError, AttributeError, RuntimeError) as exc:
            print(json.dumps({'error': str(exc)}))
            return 21
    result = dict(vid_pid='0ce9:1007', ready=False)
    def finish(state, code, message):
        result.update(status=state, exit_code=code, ready=code == 0, message=message)
        print(json.dumps(result, indent=2) if args.json else f'{state}: {message}')
        return code
    nodes = usb_nodes()
    if len(nodes) > 1:
        return finish('MULTIPLE DEVICES', 16, 'Refusing to guess between matching scopes.')
    if not nodes:
        try:
            win = windows_status()
            result['windows'] = win
            return finish(win['status'], win['exit_code'], win['message'])
        except (OSError, subprocess.SubprocessError, ValueError, KeyError):
            return finish('NOT CONNECTED', 11, 'No scope visible in Linux; inspect Windows ownership outside the sandbox.')
    node = nodes[0]
    result['usb_node'] = str(node)
    if not os.access(node, os.R_OK | os.W_OK):
        return finish('PERMISSION ERROR', 12, 'USB node is not readable/writable; check outside the sandbox.')
    try:
        load_api()
    except (OSError, AttributeError) as exc:
        return finish('DRIVER/API MISSING', 20, str(exc))
    info, error = bounded_probe(args.timeout)
    if error:
        return finish('DEVICE OPEN FAILED', 21, error)
    result.update(info)
    return finish('READY', 0, 'Real 2205A open/query/configure/close succeeded (no waveform acquired).')


if __name__ == '__main__':
    sys.exit(main())
