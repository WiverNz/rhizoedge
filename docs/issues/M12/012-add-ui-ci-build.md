# Issue M12-012 — Add the UI build CI job

**Milestone:** M12 · **PRD:** [PRD 120](../../prd/120-rust-ui.md) · **Depends on:** M12-011

## Context

Keeps the UI building without requiring every developer to install the wasm
target and Trunk.

## Goal

Build the UI in CI.

## Scope

- A job triggered by `ui/**` or `crates/api-dto/**`
- wasm target and Trunk installed
- `cargo tauri build` on Linux
- **A check asserting no `package.json` or `node_modules` exists**
- Caching

## Non-goals

- Automated browser testing — it would need the JS tooling this project excludes.

## Dependencies

- M12-011

## Implementation notes

The no-Node assertion is worth automating: the constraint is easy to violate
accidentally, since many Tauri examples assume a JS frontend, and the violation
would be invisible until someone reads the tree.

## Acceptance criteria

- [ ] The job builds the UI on the specified paths.
- [ ] **It fails if a `package.json` or `node_modules` appears.**
- [ ] Caching keeps builds reasonable.
- [ ] A DTO change that breaks the UI fails the job.

## Verification

```bash
# observe a CI run touching ui/
```

## Tests required

- Manual: add a package.json, confirm the job fails, revert.

## Documentation impact

- testing/strategy.md CI table verified.

## Files likely affected

```text
.github/workflows/ci.yml
```
