# Issue M12-013 — M12 verification and exit criteria

**Milestone:** M12 · **PRD:** [PRD 120](../../prd/120-rust-ui.md) · **Depends on:** M12-001, M12-002, M12-003, M12-004, M12-005, M12-006, M12-007, M12-008, M12-009, M12-010, M12-011, M12-012

## Context

Final gate for M12. The decisive property is that the UI **cannot** bypass
safety — a structural claim, verified structurally.

## Goal

Verify every PRD 120 acceptance criterion.

## Scope

- Build on both platforms; manual checklist against a running system
- Verify the forbidden dependencies are absent
- Verify no override control exists
- Update ROADMAP.md; record the report

## Non-goals

- New behaviour.

## Dependencies

- M12-001
- M12-002
- M12-003
- M12-004
- M12-005
- M12-006
- M12-007
- M12-008
- M12-009
- M12-010
- M12-011
- M12-012

## Implementation notes

Verify the leak case manually and completely: the lockout is prominent, no
clear button is shown, manual watering renders the reason rather than a generic
error, and nothing in the interface offers a way around it.

Also verify the disconnected case, because it is the one an operator meets at
the worst moment.

## Acceptance criteria

- [ ] `cargo tauri build` produces a runnable app on Windows and Linux.
- [ ] **No `package.json`, `node_modules`, or JS dependency anywhere.**
- [ ] **No MQTT or `rhizo-domain` dependency in any UI manifest.**
- [ ] All views render against a live edge.
- [ ] Manual watering works and shows the result.
- [ ] A leak lockout is prominent with no clear button; manual watering shows the reason.
- [ ] Enabling automation shows dose, cap, and cooldown first.
- [ ] Stopping the edge shows a banner and greyed data with its age.
- [ ] **No override control exists anywhere.**
- [ ] Charts render with the target band and watering markers.
- [ ] ROADMAP.md updated; report recorded.

## Verification

```bash
cd ui/rhizo-ui && cargo tauri build
find . -name package.json -o -name node_modules | wc -l
grep -rn 'rumqttc\|rhizo-domain' ui/ --include=Cargo.toml
```

## Tests required

- Component suite plus the manual checklist.

## Documentation impact

- ROADMAP.md.
- Milestone report.

## Files likely affected

```text
ROADMAP.md
```
