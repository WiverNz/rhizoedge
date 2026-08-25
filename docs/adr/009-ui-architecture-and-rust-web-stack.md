# ADR-009 — UI architecture: Tauri 2 + Leptos desktop application

## Status

Accepted — 2026-08-25. Supersedes the "Dockerized web UI" assumption in the
original implementation prompt. Implemented in M12.

## Context

The operator needs to see plant state, history, recommendations, and safety
lockouts, and to trigger manual watering and toggle automatic mode.

Constraints:

- Rust-first. No Node.js or TypeScript is permitted in this project.
- The UI must never bypass the Edge Controller's safety logic.
- The primary development machine is Windows.
- The V1 deployment is one operator on a home LAN with no authentication.

The original planning documents assumed `rhizo-ui` would be a Dockerized web
service. The project owner has since directed that a native desktop application
is preferred for V1. This ADR records that decision and its consequences.

## Decision

### Stack: Tauri 2 + Leptos (CSR) + Trunk

```text
┌──────────────────────────────────────────────┐
│ Tauri 2 shell  (Rust, thin)                  │
│  • window management                          │
│  • edge base URL from local settings          │
│  • WebView2 on Windows / WebKitGTK / WKWebView│
│  ┌────────────────────────────────────────┐  │
│  │ Leptos CSR app (Rust → wasm32)         │  │
│  │  built by Trunk                        │  │
│  │  fetch → http://<edge>:8080/api/v1/…   │  │
│  └────────────────────────────────────────┘  │
└──────────────────────────────────────────────┘
                    │ HTTP/JSON
                    ▼
          Edge Controller REST API
                    │
          domain / state machine / safety
                    │ MQTT
                    ▼
                 device
```

- **Tauri 2**, using the Cargo-based workflow (`cargo tauri dev`,
  `cargo tauri build`). No `npm`, no `package.json`, no Node toolchain.
- **Leptos in CSR mode**, compiled to `wasm32-unknown-unknown`, bundled by
  **Trunk**. SSR is deliberately not used: there is no server to render on — the
  app is a local binary, and SSR would introduce a second Rust runtime inside
  the desktop app for no benefit.
- **WebView2 on Windows** through Tauri's standard integration. No custom
  WebView2 host is implemented; that is a large amount of unsafe COM interop to
  reimplement what Tauri already ships.

### The UI is a thin client

The Tauri Rust side is deliberately minimal: window lifecycle, persisted
settings (edge URL, window geometry), and optionally a secret store for a future
token. It contains:

- **no** irrigation logic
- **no** safety evaluation
- **no** MQTT client
- **no** database

Every piece of state the UI displays comes from the Edge REST API, and every
action it takes is an HTTP call. If the UI is closed, deleted, or never
installed, the system behaves identically — which is the correctness test for
"thin".

### Manual watering flow — the one that matters

```text
operator clicks "Water 30 ml"
        ↓
POST /api/v1/plants/{id}/water  { "ml": 30, "mode": "manual" }
        ↓
Edge: load state → rhizo_domain safety gate → 409 Conflict + lockout reason
                                            or command persisted + published
        ↓
MQTT → device → validate_water_command → pump or refuse
```

The UI receives a `409 Conflict` with a structured lockout reason when safety
refuses, and renders it. It has **no** override, no force flag, and no
"advanced" path. A UI that could bypass the gate would nullify SAFETY-003 and
SAFETY-004.

Forbidden by architecture: the UI has no MQTT dependency in its `Cargo.toml`,
so `UI → MQTT pump command` is not merely discouraged, it does not compile.

### Real-time updates: polling, not WebSockets

The UI polls `/api/v1/overview` every 5 seconds, and per-plant detail views
poll their endpoint every 5 seconds while open.

Rationale: telemetry arrives every 300 seconds and the control loop ticks every
30 seconds, so there is nothing to see at sub-second latency. Polling over a LAN
costs a few kilobytes, has trivially correct reconnection semantics, and avoids
a stateful transport in both the server and the client. Server-Sent Events are
the documented upgrade path if a future need for push appears — the API is
shaped so adding `/api/v1/events` (SSE) would be additive.

### Charts: pure Rust, inline SVG

Moisture, EC, weight, and watering history are rendered as inline SVG generated
in Leptos from the API's time series. No JavaScript charting library, because
that would reintroduce the JS toolchain this project excludes.

The chart component is deliberately simple: line series, a shaded target band,
and watering-event markers. Anything more elaborate is out of V1 scope.

### Deployment: on the host, not in Compose

`docker compose up --build` remains the complete definition of the **software
infrastructure** — Mosquitto, simulator, edge, cloud, PostgreSQL — and is
sufficient for the M8 acceptance environment. The UI is not part of it.

Consequences of that split, all intended:

- M8's end-to-end suite stays headless and CI-runnable.
- The UI can be developed and released independently of the backend milestones.
- The operator runs a normal desktop application rather than opening a browser
  at a port.

The Edge REST API is nonetheless kept **transport-agnostic and CORS-capable**
(configurable allowed origins, default none) so a browser-hosted Leptos frontend
could be added later against the same API with no Edge Controller changes.
Building that second frontend is explicitly out of V1 scope.

### Security posture for V1

No authentication. The Edge API binds to loopback by default and to the LAN
address when configured. The network boundary is the security boundary, matching
[ADR-011](011-configuration-and-secrets-model.md) §5. The UI stores no
credentials because there are none.

This is stated plainly because it is a real limitation: anyone on the home LAN
who can reach port 8080 can water a plant. Acceptable for V1's threat model,
and the first thing to change in a multi-user or exposed deployment (M13).

## Alternatives considered

**Dockerized Leptos SSR web service** (the original plan). Rejected per the
project owner's direction, and independently a poorer fit: it adds a container,
a port, and an SSR runtime to serve one local user, and a browser tab is a worse
home for a monitoring app that should sit in the tray.

**Axum + Askama + HTMX.** A legitimate, simpler option — server-rendered HTML
from the edge itself, no wasm. Rejected because it would put presentation
concerns inside the Edge Controller binary, blurring the boundary that keeps the
control plane auditable, and because chart interactivity would be awkward.

**egui / iced native GUI.** Rejected: charting and text layout are more work,
and the HTML/CSS path is far more productive for a dashboard. Tauri's WebView
also means the same Leptos code can later serve a browser frontend.

**Any JS framework (React, Svelte, Vue).** Excluded by project constraint.

**Tauri with a JS frontend build step.** Excluded: it reintroduces `npm` through
the back door. The Cargo/Trunk workflow avoids Node entirely.

## Consequences

Positive:

- Zero Node.js anywhere in the repository.
- One language across firmware, edge, cloud, and UI; types can be shared with
  the API layer if desired.
- The UI's inability to bypass safety is structural (no MQTT dependency), not a
  policy.
- M8 remains headless and CI-friendly.

Negative, accepted:

- A desktop app must be built per platform, and needs signing for painless
  distribution. V1 ships unsigned local builds; this is documented, not solved.
- No remote access. Checking plants from a phone requires the deferred web
  frontend or a VPN. Stated as a known V1 limitation.
- WebView differences across platforms are a real source of rendering bugs. The
  UI stays visually simple partly for this reason.
- Leptos and Tauri both evolve quickly; versions are pinned and bumped
  deliberately.

## Risks

- **Logic creeping into the Tauri side** because it is convenient. *Mitigation:*
  `ui/rhizo-ui/src-tauri/Cargo.toml` depends on no Rhizo crate except a shared
  API DTO crate; adding `rhizo-domain` there is an obvious review flag.
- **Windows WebView2 runtime absent** on a target machine. *Mitigation:* Tauri's
  installer can bootstrap it; documented in M12-011.
- **The API growing UI-shaped endpoints** that leak presentation into the
  control plane. *Mitigation:* one composite `/api/v1/overview` endpoint is
  permitted for the dashboard; everything else stays resource-oriented.

## Follow-up

- [PRD 120](../prd/120-rust-ui.md) — UI requirements.
- [docs/protocol/http-api-boundaries.md](../protocol/http-api-boundaries.md) — the API contract.
- M12-001…M12-012 implement the UI.
- CORS configuration is an M4 API concern (issue M4-009), so the deferred web
  frontend stays possible without later rework.
