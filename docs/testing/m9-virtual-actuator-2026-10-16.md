# Real ESP32-C3 virtual-actuator evidence — 2026-10-16

Logical execution and real NVS only. No GPIO/HIL claim. See [M9 report](../reports/M9.md) §13.

## Executed-dose power-cycle duplicate

ID: `3b740075-5b65-4174-9350-418a3c6daaf7`. Two canonical deliveries, one virtual execution, two identical terminal results, two Edge ACKs. One 4 ml Edge accounting contribution. No retry for more than two 15-second intervals after ACK. User confirmed full USB removal, >=10-second wait and reconnect.

Pre/post Edge query results were identical:

```json
{"commands": [{"command_id": "3b740075-5b65-4174-9350-418a3c6daaf7", "device_id": "plant-node-579814", "plant_id": "m9-virtual-bench-20261016", "kind": "water", "requested_ml": 4.0, "mode": "manual", "issued_at": 1792131324081, "expires_at": 1792131444081, "status": "completed", "published_at": 1792131324081, "settled_at": 1792131325330, "reason": null}]}
{"command_results": [{"message_id": "01a14359-dbc8-7f38-b255-066b7b1e037b", "command_id": "3b740075-5b65-4174-9350-418a3c6daaf7", "device_id": "plant-node-579814", "result_json": "{\"command_id\":\"3b740075-5b65-4174-9350-418a3c6daaf7\",\"status\":\"completed\",\"requested_ml\":4.0,\"delivered_ml\":4.0,\"duration_ms\":1000,\"clamped\":false,\"reason\":null,\"delivered_today_ml\":8.0,\"origin\":\"edge_command\"}", "received_at": 1792131325330, "device_time_ms": 1792131324872, "boot_id": "00000000-0000-72bb-8240-24ef4552827f", "sequence": 143419}]}
{"watering_events": [{"watering_event_id": "01a14359-dd92-7401-bee6-1a685a4a5eed", "plant_id": "m9-virtual-bench-20261016", "device_id": "plant-node-579814", "command_id": "3b740075-5b65-4174-9350-418a3c6daaf7", "mode": "manual", "origin": "edge_command", "started_at": 1792131324081, "completed_at": 1792131325330, "requested_ml": 4.0, "delivered_ml": 4.0, "status": "completed", "reason_json": null}]}
```

## Saturation campaign C (corrected NVS layout)

Baseline 21 simulated ml. Edge stopped, MQTT remained running. Each command was issued once only after the previous expected response and durable commit were verified.

| # | Command ID | Outcome | Durable slot |
|---:|---|---|---|
| 1 | `07e3e70f-8b4e-4e2b-95d1-d5c311b69d83` | completed, 1 ml | yes |
| 2 | `e2b0e803-f2a5-4b86-aaf4-bf5e03b14dae` | completed, 1 ml | yes |
| 3 | `2c4465b8-7a7f-4be5-ab9f-b56ec1b108a8` | completed, 1 ml | yes |
| 4 | `d44bd904-b8b7-459e-9950-b4a38f192256` | completed, 1 ml | yes |
| 5 | `c3a1317e-5046-4ba1-b2f0-2a5ed850b6be` | completed, 1 ml | yes |
| 6 | `18f117c7-7fde-4b20-8077-ece7d72aaf40` | completed, 1 ml | yes |
| 7 | `e23e29e4-111c-4cca-89cd-08f6f5dc21da` | completed, 1 ml | yes |
| 8 | `572b4710-5fb2-4516-a58a-5a4156834503` | completed, 1 ml | yes |
| 9 | `d0edf8e8-6e42-48c4-a6de-5ac468c08962` | completed, 1 ml | yes |
| 10 | `63969dc1-e73a-4c88-857b-9071541aaf2e` | completed, 1 ml | yes |
| 11 | `6b0cbf53-c11f-47b2-9d57-903833c33078` | completed, 1 ml | yes |
| 12 | `9a8d52e4-60c9-41f3-aac2-4456555e7e96` | completed, 1 ml | yes |
| 13 | `968bdceb-a7aa-4d9d-87a5-7ecce1e70ec2` | completed, 1 ml | yes |
| 14 | `e666bafe-6cc9-41cb-b943-5f64d65e882a` | completed, 1 ml | yes |
| 15 | `6e446889-62b8-429e-abb0-14023616953a` | completed, 1 ml | yes |
| 16 | `7363a027-c892-4314-874d-2ad1138f59d6` | rejected: result_ledger_full | yes |
| 17 | `8c6b791a-557a-4513-aa64-8f896f229e41` | rejected: result_ledger_full | no; ledger unchanged |

Exactly 15 virtual executions. Sixteenth refusal occupies reserved slot; seventeenth refusal is unledgered. All first 16 IDs preserved, total 36 ml, pending delivered amount 15 ml. Two user-confirmed >=10-second USB power cycles preserved the exact first 16 IDs, total 36 ml and pending amount 15 ml. The second cycle used the corrected retry/dedup image. No virtual execution occurred after either saturated reboot. A further command `0165fa23-e3d4-41fe-8810-fcbad8ae3b91` was refused without changing the full ledger.


## ACK recovery and final database reconciliation (October 17)

Real Edge ACKs removed the 16 table entries in order, one matching ID per ACK:
16→15→14→13→12→11→10→9→8→7→6→5→4→3→2→1→0.
At 15 entries the reserved threshold still refuses execution; at 14 there is
normal capacity again. No entry disappeared before its own ACK. Device total
remained 36 ml. Independent SQLite queries found exactly 16 terminal-result
rows for these IDs and 15 completed watering rows, summing to 15 ml.
Recovery command `d4aa6ad2-5ff2-44bd-a638-21a2ba56ef12` then executed once,
reported 1 ml, received its real Edge ACK and raised device total to 37 ml.

Fixed-image saturated cold boot retried the oldest pending ID at monotonic
9469, 24589 and 40069 ms, with zero virtual executions. An oldest-ID replay
returned its original completed result even after refusal traffic aged it out
of the short dedup history. Pending results are authoritative until ACK.

## Lost ACK after durable Edge commit

For `0cbcaa6e-0cdf-4539-8d31-9e2000527ba0`, a temporary Mosquitto per-user
`topic deny rhizo/v1/devices/plant-node-579814/commands/result/ack` rule
withheld delivery to the board only. Edge and the privileged MQTT spy remained
connected. The exact original ACL was restored and reloaded afterward; no ACL
diff remains. No authentication or other permissions were weakened.

One command delivery caused one virtual execution (per-boot counter 1→2,
250 ms requested duration), one completed 1 ml terminal result, and one Edge
watering row. Device accounting became 38 ml. Despite 71 result publications
and 71 broker-observed ACK publications, there was no second execution or
accounting contribution. All result **data/command identities** were equal;
transport envelopes may change across retry/boot. SQLite rows before and after
were exactly equal, including the first stored result envelope.

Edge settlement timestamp: **1792187839642 ms**. First result observed at
**1792187839642177690 ns**, first ACK at **1792187839645063522 ns**. The durable
SQLite row was independently queried while ACK delivery was still denied.
The shared Edge handler commits settlement before publishing application ACK;
MQTT timestamps alone do not establish transaction ordering.

A reset while ACKs were denied loaded total 38 ml, that one pending ID, pending
amount 1 ml, and no in-flight dose. It retried at **9379, 24509, 39889 ms** after
boot with **zero executions**. Restoring the ACL allowed ACK at UART 209274 ms,
removed the matching entry, and left accounting unchanged. No subsequent
result publication during **67640 ms** of observation (>4 retry intervals).

Final cleanup applied retained config **12**, pump **disabled**, always-on,
10-second explicit fake-sensor telemetry. A final 50-second reset/monitor observation loaded 38 ml, no pending results,
and retained config 12 (`IgnoredNotNewer` on re-delivery). There were zero
virtual executions and zero result publications, proving durable ACK removal. The virtual image remains installed: GPIO5 is untouched.
This pass proves real ESP32-C3 NVS/radio/command logic, not electrical actuation.

Structured SQLite rows, ACK removal steps and selected UART evidence are saved
in [the evidence JSON](m9-virtual-actuator-evidence.json).
