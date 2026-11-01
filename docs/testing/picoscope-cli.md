# PicoScope CLI reference

The reusable setup and operation reference is now the living
[PicoScope 2205A runbook](../hardware/picoscope-2205a.md). It covers the hardware,
Windows ps2000 backend controlled from WSL, clean-machine setup, USB ownership,
x10 scaling, capture commands, evidence format, LOW/known-signal validation,
timeouts/locks and verified measurement limitations. Update that runbook when
the scripts, setup or observed behavior changes; avoid parallel instructions
that can diverge.

Quick entry points, from the repository root in WSL:

```sh
./scripts/picoscope-status.sh --json
./scripts/picoscope-capture.sh --help
```

The scope must be Windows-owned. Do not attach it to WSL or run the rejected
Linux ps2000 path for measurements. Read the runbook and confirm physical
wiring before any capture or explicit isolated test-signal operation.

M9-specific evidence and acceptance status remain in the
[M9 report](../reports/M9.md). The [HIL procedures](hardware-in-the-loop.md)
retain their full required counts and criteria. Functional scope validation
alone does not close a GPIO/HIL criterion.

The [original programmer guide](../vendor/pico/picoscope-2000-series-programmers-guide.pdf)
and its [provenance/checksum](../vendor/pico/README.md) are preserved in the
repository without changes to vendor notices.
