# Issue M12-008 — Implement the profile editor

**Milestone:** M12 · **PRD:** [PRD 120](../../prd/120-rust-ui.md) · **Depends on:** M12-002

## Context

Profiles carry the values the safety gate consumes. Client-side validation
mirrors the server's so the operator gets immediate feedback, and server 422s
render specifically.

## Goal

Let the operator edit profiles safely.

## Scope

- Form with all profile fields
- Client-side validation mirroring the server rules
- **Server 422 rendered inline on the offending field**
- Firmware hard limits shown as the bounds
- Which plants use a profile shown before saving

## Non-goals

- Editing hard limits — impossible by design.

## Dependencies

- M12-002

## Implementation notes

Client validation is a convenience; the server remains authoritative. If the
two disagree, the server wins and the 422 must render — never suppress a server
error because the client thought the value was fine.

Showing which plants use a profile before saving prevents the surprise of
changing five plants while intending to change one.

## Acceptance criteria

- [ ] All fields are editable with immediate validation.
- [ ] Client rules mirror the server's.
- [ ] **A server 422 renders inline naming the violated rule and the limit.**
- [ ] Hard limits are shown as bounds.
- [ ] Affected plants are listed before saving.
- [ ] A client/server disagreement resolves to the server's answer.

## Verification

```bash
cd ui/rhizo-ui && cargo test profile_editor::
```

## Tests required

- Validation mirroring.
- **422 inline rendering.**
- Affected-plant listing.

## Documentation impact

- None.

## Files likely affected

```text
ui/rhizo-ui/src/views/profile_editor.rs
```
