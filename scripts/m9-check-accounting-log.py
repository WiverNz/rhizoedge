#!/usr/bin/env python3
"""Check M9 bare-board serial evidence; never substitutes for GPIO/HIL traces.

Fixtures: pump disabled, 40 ml budget, zero window elapsed, 120 s cooldown,
90 s disabled-policy window and 60 s sleep interval. Run against captured logs.
"""
import argparse
import re
from pathlib import Path

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("mode", choices=("cold", "timer", "invalid"))
parser.add_argument("log", type=Path)
args = parser.parse_args()
text = args.log.read_text(errors="replace")
pattern = r"wake_reason (\w+), credited (\d+)ms; accounting used_ml=([\d.]+) window_ms=(\d+) cooldown_ms=(\d+) rtc_valid=(true|false)"
records = [(r, int(e), float(u), int(w), int(c), valid == "true")
           for r, e, u, w, c, valid in re.findall(pattern, text)]

def require(condition, message):
    if not condition:
        raise SystemExit("FAIL: " + message)

require(bool(records), "no accounting records captured")
if args.mode == "cold":
    require(all(r == "ColdBoot" and e == 0 and (u, w, c) == (40, 0, 120000)
                for r, e, u, w, c, _ in records), "cold boot changed the seeded accounting")
elif args.mode == "invalid":
    require("BENCH: deliberately corrupting retained checksum" in text, "missing corruption injection")
    timer = [x for x in records if x[0] == "Timer"]
    require(bool(timer), "no physical timer wake captured")
    require(all(e == 0 and not valid and (u, w, c) == (40, 0, 120000)
                for _, e, u, w, c, valid in timer), "invalid retention granted credit")
else:
    require(records[0][0] == "ColdBoot" and records[0][1:5] == (0, 40, 0, 120000), "missing seeded cold baseline")
    timer = [x for x in records if x[0] == "Timer"]
    require(len(timer) >= 2, "two real timer wakes are required")
    used, window, cooldown = 40, 0, 120000
    for _, elapsed, actual_used, actual_window, actual_cooldown, valid in timer:
        require(valid and 59000 <= elapsed <= 62000, "invalid or implausible measured 60 s sleep")
        window += elapsed
        if window >= 90000:
            used = 0
        window %= 90000
        cooldown = max(0, cooldown - elapsed)
        require((actual_used, actual_window, actual_cooldown) == (used, window, cooldown), "accounting does not match measured elapsed")
print(f"PASS: {args.mode}, {len(records)} accounting records; no GPIO claim")
