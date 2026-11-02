#!/usr/bin/env bash
# Preferred readiness path: real Windows ps2000 open/configure/close.
set -eu
exec python3 "$(dirname "$0")/picoscope_cli.py" status "$@"
