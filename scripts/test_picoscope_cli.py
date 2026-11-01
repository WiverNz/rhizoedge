"""Argument validation and WSL orchestration tests. No USB access."""
import contextlib
import importlib.util
import io
from pathlib import Path
import unittest

spec=importlib.util.spec_from_file_location('cli',Path(__file__).with_name('picoscope_cli.py'))
cli=importlib.util.module_from_spec(spec)
spec.loader.exec_module(cli)

class CliTests(unittest.TestCase):
    def test_defaults_match_manual_baseline(self):
        a=cli.parse(['capture'])
        self.assertEqual((a.range_v,a.duration_ms,a.samples,a.trigger),(5,10,16000,'none'))

    def test_signal_is_explicit_capture_only(self):
        self.assertIsNone(cli.parse(['capture']).test_signal)
        self.assertEqual(cli.parse(['capture','--test-signal','square-1khz']).test_signal,'square-1khz')
        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
            cli.parse(['status','--test-signal','square-1khz'])

    def test_diagnostic_options_require_stream_capture(self):
        a=cli.parse(['capture','--mode','stream','--diagnostic-readback','--diagnostic-default-timer'])
        self.assertTrue(a.diagnostic_readback)
        self.assertTrue(a.diagnostic_default_timer)
        for args in (['status','--diagnostic-readback'], ['capture','--diagnostic-default-timer']):
            with self.assertRaises(SystemExit), contextlib.redirect_stderr(io.StringIO()):
                cli.parse(args)

    def test_streaming_constraints(self):
        a=cli.parse(['capture','--mode','stream','--duration-ms','2000','--timeout-s','5'])
        self.assertEqual(a.stream_interval_ns,1000)
        for args in [['--count','2'],['--trigger','rising'],['--test-signal','dc-zero'],['--stream-interval-ns','0'],['--stream-interval-ns','1000001']]:
            with self.subTest(args=args), contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
                cli.parse(['capture','--mode','stream']+args)

    def test_pre_post_and_falling(self):
        a=cli.parse(['capture','--trigger','falling','--pre-ms','2','--post-ms','8'])
        self.assertEqual(a.duration_ms,10)
        self.assertEqual(a.pre_ms,2)

    def test_invalid_arguments_fail_before_usb(self):
        for args in [['--range-v','3'],['--duration-ms','nan'],['--pre-ms','1'],['--count','0'],
                     ['--samples','1'],['--timeout-s','0'],['--trigger-v','5'],['--pulse-v','-5'],
                     ['--trigger','rising','--pre-ms','10'],['--post-ms','0']]:
            with self.subTest(args=args), contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as cm:
                cli.parse(['capture']+args)
            self.assertEqual(cm.exception.code,2)

if __name__=='__main__': unittest.main()
