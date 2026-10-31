"""Focused status/timeout tests; no real USB access."""
import fcntl
import importlib.util
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import MagicMock, patch

spec = importlib.util.spec_from_file_location('status', Path(__file__).with_name('picoscope_status.py'))
status = importlib.util.module_from_spec(spec)
spec.loader.exec_module(status)


class StatusTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.lock = Path(self.temp.name) / 'probe.lock'

    def test_successful_worker(self):
        data, error = status.run_bounded([sys.executable, '-c', 'print(\'{"model":"2205A"}\')'], 2, self.lock)
        self.assertIsNone(error)
        self.assertEqual(data['model'], '2205A')

    def test_worker_failure_not_ready(self):
        data, error = status.run_bounded([sys.executable, '-c', 'print(\'{"error":"open=0"}\'); exit(21)'], 2, self.lock)
        self.assertIsNone(data)
        self.assertEqual(error, 'open=0')

    def test_invalid_report_not_ready(self):
        data, error = status.run_bounded([sys.executable, '-c', 'print("[]")'], 2, self.lock)
        self.assertIsNone(data)
        self.assertIn('invalid report', error)

    def test_live_lock_refuses_spawn(self):
        with self.lock.open('w') as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            with patch.object(status.subprocess, 'Popen') as popen:
                data, error = status.run_bounded(['unused'], 1, self.lock)
                popen.assert_not_called()
        self.assertIsNone(data)
        self.assertIn('still holds', error)

    def test_uninterruptible_worker_never_waited_after_kill(self):
        child = MagicMock(pid=123)
        child.communicate.side_effect = subprocess.TimeoutExpired('worker', 1)
        child.poll.return_value = None
        with patch.object(status.subprocess, 'Popen', return_value=child):
            data, error = status.run_bounded(['unused'], 1, self.lock)
        self.assertIsNone(data)
        child.kill.assert_called_once()
        child.wait.assert_not_called()
        child.communicate.assert_called_once_with(timeout=1)
        child.stdout.close.assert_called_once()
        child.stderr.close.assert_called_once()
        self.assertIn('PID 123', error)

    def test_child_inherits_lock_until_exit(self):
        # A separate process owns the inherited flock even after the parent's
        # descriptor closes. Reopening must fail until that child exits.
        fd = status.os.open(self.lock, status.os.O_CREAT | status.os.O_RDWR, 0o600)
        fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        child = subprocess.Popen([sys.executable, '-c', 'import sys; sys.stdin.read()'], stdin=subprocess.PIPE, pass_fds=(fd,))
        status.os.close(fd)
        try:
            with patch.object(status.subprocess, 'Popen') as spawn:
                data, error = status.run_bounded(['unused'], 1, self.lock)
                spawn.assert_not_called()
            self.assertIsNone(data)
        finally:
            child.communicate(timeout=2)
        data, error = status.run_bounded([sys.executable, '-c', 'print(\'{"model":"2205A"}\')'], 2, self.lock)
        self.assertIsNone(error)


if __name__ == '__main__':
    unittest.main()
