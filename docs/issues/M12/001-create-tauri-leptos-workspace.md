# Issue M12-001 — Create the Tauri + Leptos workspace

**Milestone:** M12 · **PRD:** [PRD 120](../../prd/120-rust-ui.md) · **Depends on:** M6-019

## Context

ADR-009: Tauri 2 with the Cargo workflow, Leptos CSR via Trunk, and **no
Node.js anywhere**. The Tauri Rust side stays thin by construction.

## Goal

Establish the UI workspace with no JavaScript toolchain.

## Scope

- `ui/rhizo-ui/` as its own workspace, excluded from the root
- Tauri 2 with `cargo tauri dev` / `build` — **no `package.json`**
- Leptos CSR targeting `wasm32-unknown-unknown`, bundled by Trunk
- **No MQTT dependency and no `rhizo-domain` dependency**
- A shared API DTO crate as the only Rhizo dependency

## Non-goals

- SSR — there is no server to render on.

## Dependencies

- M6-019

## Implementation notes

The absent dependencies are the architecture. With no MQTT crate, `UI ->
MQTT pump command` does not compile; with no domain crate, the UI cannot
recompute a safety decision and disagree with the system that actually waters.

Assert both absences in a test that reads the manifests.

## Acceptance criteria

- [ ] `cargo tauri dev` runs the app.
- [ ] `cargo tauri build` produces a binary.
- [ ] **No `package.json` or `node_modules` anywhere in the repository.**
- [ ] **No MQTT dependency in any UI manifest.**
- [ ] **No `rhizo-domain` dependency.**
- [ ] The workspace is excluded from the root.

## Verification

```bash
cd ui/rhizo-ui && cargo tauri build
find . -name package.json -o -name node_modules | wc -l   # expect 0
grep -r 'rumqttc\|rhizo-domain' ui/*/Cargo.toml ui/*/*/Cargo.toml   # expect none
```

## Tests required

- A manifest test asserting the forbidden dependencies are absent.

## Documentation impact

- None.

## Files likely affected

```text
ui/rhizo-ui/Cargo.toml
ui/rhizo-ui/src-tauri/Cargo.toml
ui/rhizo-ui/Trunk.toml
```
