# PRD 120 — Rust UI (Tauri + Leptos Desktop Application)

**Milestone:** M12 · **Status:** PLANNED · **Depends on:** M6 (functionally), M11 (for the full picture)

## Summary

A native desktop application — Tauri 2 shell, Leptos CSR frontend, no Node.js —
that shows plant state, history, recommendations, and safety lockouts, and lets
the operator water manually and toggle automation. It is a thin HTTP client of
the Edge Controller.

## Problem

Until M12 the system is operated with `curl` and `sqlite3`. That is fine for
development and useless for the actual goal: a person deciding whether to trust
this thing with a plant. Trust comes from seeing what the system believes and
why — particularly why it refused to do something.

## Goals

1. A native desktop app with no JavaScript toolchain anywhere.
2. Current state and history for every plant and device.
3. Recommendations with their reasons rendered legibly.
4. Manual watering and automation toggles that cannot bypass safety.
5. Lockouts shown prominently with their reason and what clears them.
6. Edge and cloud health, including pending sync.

## Non-goals

- **A web frontend.** The API is kept CORS-capable so one could be added later,
  but building it is out of V1 scope
  ([ADR-009](../adr/009-ui-architecture-and-rust-web-stack.md)).
- Remote access. The app talks to a LAN address; there is no auth and no
  Internet path. Stated as a known limitation.
- Any business logic in the UI. If the UI is deleted, the system behaves
  identically — that is the correctness test for "thin".
- Mobile.
- Editing anything the API does not expose.

## User/system flows

**Daily check:**

```text
open app → overview: every plant, state, moisture, next estimate,
           any lockout, cloud sync status
        → click a plant → charts, history, recommendation with reasons
```

**Manual watering:**

```text
click "Water 30 ml" → POST /plants/{id}/water
   200/202 → "Command sent" → poll → "Delivered 30 ml"
   409     → render the lockout reason and what clears it
             — there is NO override control
```

**Enabling automation:**

```text
toggle → confirmation dialog showing the profile's dose, daily cap,
         and cooldown → POST /auto-watering/enable
```

The confirmation is deliberate: enabling automation is the moment a person hands
a pump to a program, and the limits should be visible at that moment.

## Functional requirements

### Stack

| ID | Requirement |
|---|---|
| F-120-01 | Tauri 2, Cargo workflow (`cargo tauri dev` / `build`) — **no `npm`, no `package.json`** |
| F-120-02 | Leptos CSR compiled to `wasm32-unknown-unknown`, bundled by Trunk |
| F-120-03 | No SSR — there is no server to render on |
| F-120-04 | WebView2 on Windows via Tauri's standard integration; no custom host |
| F-120-05 | Own Cargo workspace, excluded from the root |
| F-120-06 | The Tauri Rust side depends on **no** Rhizo crate except a shared API DTO crate |

### Views

| ID | Requirement |
|---|---|
| F-120-10 | Overview: plants, states, latest moisture, lockouts, device online count, cloud sync |
| F-120-11 | Plant detail: current values, state, recommendation with reasons, water budget, last watering |
| F-120-12 | Charts: moisture, EC, weight over selectable ranges, with the target band shaded and watering events marked |
| F-120-13 | Watering history with mode, requested, delivered, and status |
| F-120-14 | Device page: online state, firmware, sample age, sensor health, config drift, hard limits |
| F-120-15 | Profile editor with **client-side validation mirroring the server's**, and server 422 errors rendered specifically |
| F-120-16 | Events view: device events with severity |
| F-120-17 | Sync view: pending count, last success, quarantined events |

### Safety presentation

| ID | Requirement |
|---|---|
| F-120-20 | An active lockout is the **most prominent element** on the plant view |
| F-120-21 | The reason is rendered in plain language plus what will clear it |
| F-120-22 | `clearable: false` renders **no** clear button |
| F-120-23 | A 409 from any action renders the specific reason, never a generic failure |
| F-120-24 | **No override, force, or advanced control exists anywhere in the UI** |
| F-120-25 | Automation toggle shows dose, daily cap, and cooldown before enabling |
| F-120-26 | Stale data is shown as stale — a greyed value with its age, never a fresh-looking number |
| F-120-27 | The manual/automatic privilege difference is explained where it matters (manual works with a faulty sensor; nothing works during a leak) |

### Technical

| ID | Requirement |
|---|---|
| F-120-30 | Polling every 5 s for the open view; no WebSockets |
| F-120-31 | Charts are inline SVG generated in Leptos — **no JS charting library** |
| F-120-32 | Edge base URL configurable and persisted |
| F-120-33 | Connection loss shows a clear banner and stops silently retrying forever |
| F-120-34 | No MQTT client dependency — the shortcut must not compile |

## Interfaces

Consumes the Edge REST API
([http-api-boundaries.md](../protocol/http-api-boundaries.md) §2) exclusively.
`/api/v1/overview` is the one composite endpoint, existing for this app.

Tauri commands (deliberately minimal):

```rust
#[tauri::command] fn get_edge_url() -> String;
#[tauri::command] fn set_edge_url(url: String) -> Result<(), String>;
```

That is the whole Tauri surface. Anything more would be logic in the wrong place.

## Data model

None. The UI holds no authoritative state. In-memory view state only; window
geometry and edge URL persisted by Tauri.

## State model

```text
Disconnected ──► Connecting ──► Connected ──► Polling
      ▲                                          │
      └────────── request failure ───────────────┘
```

Per view: `Loading → Loaded | Error`. Actions: `Idle → Submitting → Success |
Refused(reason) | Error`.

`Refused` is a **distinct state from `Error`**, because a safety refusal is the
system working correctly and must not be presented as a malfunction. Conflating
them would teach the operator to distrust correct behaviour.

## Failure modes

| Failure | UI behaviour |
|---|---|
| Edge unreachable | banner "Cannot reach controller at {url}"; last-known data greyed with its age; **never a blank screen** |
| 409 on an action | specific lockout reason and what clears it |
| 422 on a profile save | the violated rule inline on the field |
| 503 (not ready) | "Controller starting" with the failing readiness checks listed |
| Slow response | spinner after 500 ms; timeout at 10 s |
| Clock skew between app and edge | timestamps rendered from edge values, never recomputed locally |
| WebView2 missing on Windows | the installer bootstraps it; documented |

The "never a blank screen" rule matters: an operator checking on a plant during
a network problem needs to see the last known state with its age, not nothing.

## Safety implications

The UI enforces no invariant — it **must be incapable of violating one**.

Structural guarantees:

- **No MQTT dependency** (F-120-34). `UI → MQTT pump command` does not compile.
- **Every action goes through the Edge API**, which runs the domain safety gate
  ([ADR-006](../adr/006-irrigation-state-machine-ownership.md)).
- **No override control exists** (F-120-24). Not hidden, not behind a flag —
  absent.
- **No logic duplication.** The UI renders the API's `lockout`, `reasons`, and
  `blocked_by`; it does not recompute them. A UI that decided independently
  whether a plant needed water would eventually disagree with the system that
  actually waters.

Presentation is itself safety-relevant. F-120-20, -22, -26, and the
`Refused ≠ Error` distinction exist so the operator forms an accurate model of
what the system will do. A UI that shows a stale reading as current, or a
refusal as a bug, produces exactly the wrong human response.

## Observability

The UI consumes observability rather than producing it: pending sync count, last
cloud success, device online state, control loop health, and device events are
all rendered from the API.

Local logging to a file for troubleshooting the app itself. No telemetry is sent
anywhere.

## Testing strategy

- Unit (Rust): API response deserialisation including error envelopes; chart
  data transformation; profile validation mirroring; relative-time formatting.
- Component: lockout renders with no clear button when `clearable: false`;
  409 renders as `Refused`, not `Error`; stale values render greyed with age.
- Manual: a checklist against a running system, including the disconnected case,
  the leak-lockout case, and the automation-enable confirmation.
- Cross-platform: Windows (WebView2), Linux (WebKitGTK) at minimum.

Automated browser testing is **not** done — it would require the JS tooling this
project excludes ([strategy.md](../testing/strategy.md) §11).

## Acceptance criteria

- [ ] `cargo tauri build` produces a runnable app on Windows and Linux.
- [ ] **No `package.json`, no `node_modules`, no JS dependency** anywhere in the
      repository.
- [ ] All views render against a live edge.
- [ ] Manual watering works and shows the delivered result.
- [ ] A leak lockout renders prominently with **no** clear button, and manual
      watering shows the reason rather than a generic failure.
- [ ] Enabling automation shows dose, daily cap, and cooldown first.
- [ ] Stopping the edge shows a banner and greyed last-known data with its age.
- [ ] `ui/rhizo-ui/**/Cargo.toml` contains no MQTT dependency and no
      `rhizo-domain` dependency.
- [ ] Charts render moisture, EC, and weight with the target band and watering
      markers, in inline SVG.

## Dependencies

- M6 (there must be state and actions worth showing).
- M4 (device API), M5 (plant API and recommendations), M7 (sync status).
- M11 is not a hard dependency — the UI works against the simulator — but the
  full picture needs real hardware.

## Open questions

1. **Chart interaction depth.** V1 is static SVG with a selectable range. Zoom
   and hover tooltips are nice; the target-band-plus-events rendering carries
   most of the value. Scope decided in M12-007.
2. **Whether to bundle the edge controller inside the Tauri app** for a
   single-binary home deployment. Appealing, but it would blur the boundary that
   keeps the control plane auditable and would make the edge's lifetime the
   app's lifetime — a closed laptop must not stop watering. Rejected for V1,
   recorded here because it will be asked.
3. **Code signing and distribution.** V1 ships unsigned local builds. Documented,
   not solved.

## Future work

- Browser-hosted Leptos frontend against the same API (the reason CORS exists).
- Notifications on lockouts (M13).
- Multi-plant comparison views (M13).
- Cloud API as an alternate data source for remote viewing (post-V1).
