#!/usr/bin/env python3
"""Offline checks of the independent waveform audit; no device access."""
import importlib.util
from pathlib import Path
import struct
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('stress', Path(__file__).with_name('picoscope-stress.py'))
stress = importlib.util.module_from_spec(spec)
spec.loader.exec_module(stress)

class AuditTests(unittest.TestCase):
    def test_preserves_and_counts_missing_markers(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            data = struct.pack('<5h', 1, -32768, 0, 32767, -32768)
            (root/'raw.i16le').write_bytes(data)
            report = stress.audit_raw(root, dict(raw_i16='raw.i16le', sample_count=5, lost_sample_count=2))
            self.assertTrue(report['agrees_with_metadata'])
            self.assertEqual(report['missing_markers'], 2)
            self.assertEqual((root/'raw.i16le').read_bytes(), data)
            report = stress.audit_raw(root, dict(raw_i16='raw.i16le', sample_count=5, lost_sample_count=0))
            self.assertFalse(report['agrees_with_metadata'])

    def test_truncation_does_not_match_declared_count(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            (root/'raw.i16le').write_bytes(struct.pack('<2h', 0, 0))
            report = stress.audit_raw(root, dict(raw_i16='raw.i16le', sample_count=3, lost_sample_count=0))
            self.assertFalse(report['agrees_with_metadata'])

    def test_odd_byte_file_is_invalid(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            (root/'raw.i16le').write_bytes(b'X')
            with self.assertRaises(ValueError):
                stress.audit_raw(root, dict(raw_i16='raw.i16le', sample_count=1, lost_sample_count=0))

if __name__ == '__main__':
    unittest.main()
