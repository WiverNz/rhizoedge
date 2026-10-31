#!/usr/bin/env bash
# Preferred capture path: native Windows ps2000, orchestrated from WSL.
set -eu
exec python3 "$(dirname "$0")/picoscope_cli.py" capture "$@"
