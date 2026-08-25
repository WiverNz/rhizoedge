# Issue M12-011 — Add packaging and WebView2 bootstrapping

**Milestone:** M12 · **PRD:** [PRD 120](../../prd/120-rust-ui.md) · **Depends on:** M12-010

## Context

ADR-009 risk: a Windows machine without the WebView2 runtime cannot run the
app. Tauri's installer can bootstrap it.

## Goal

Produce installable builds on the target platforms.

## Scope

- Windows installer with WebView2 bootstrapping
- Linux AppImage or deb
- Application icon and metadata
- Edge URL configuration on first run
- Documented unsigned-build limitation

## Non-goals

- Code signing — V1 ships unsigned, documented not solved.

## Dependencies

- M12-010

## Implementation notes

State the unsigned-build limitation plainly in the documentation rather than
letting a user discover it via an OS warning. It is a real distribution
limitation, not a defect.

First-run URL configuration matters: the default `localhost:8080` is wrong for
an operator whose edge runs on a Pi.

## Acceptance criteria

- [ ] A Windows installer is produced and bootstraps WebView2.
- [ ] A Linux package is produced.
- [ ] The app runs on a clean Windows machine without WebView2 pre-installed.
- [ ] First run prompts for the edge URL.
- [ ] The URL persists.
- [ ] The unsigned-build limitation is documented.

## Verification

```bash
cd ui/rhizo-ui && cargo tauri build
# install on a clean VM
```

## Tests required

- Manual installation on a clean machine per platform.

## Documentation impact

- Installation and first-run documentation.

## Files likely affected

```text
ui/rhizo-ui/src-tauri/tauri.conf.json
```
