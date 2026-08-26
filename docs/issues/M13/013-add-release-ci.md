# Issue M13-013 — Add release binary CI with checksummed artefacts

**Milestone:** M13 · **PRD:** [PRD 130](../../prd/130-multi-plant-home.md) · **Depends on:** M13-009

## Context

Using Rhizo Edge should not require installing Rust and building a workspace. A
tagged release should produce downloadable artefacts.

## Goal

Publish release binaries automatically from a tag.

## Scope

- Workflow triggered by a `v*` tag
- Build `edge-controller`, `device-simulator`, `cloud-api`, `rhizo-provision`, `rhizo-backup` for `linux-x86_64` and `linux-aarch64`
- Windows `x86_64` where the component supports it
- Version embedded in each binary and reported by `--version`
- Archives named `<component>-<version>-<target>.tar.gz` (`.zip` on Windows)
- `SHA256SUMS` covering every artefact
- Attached to the GitHub Release
- Tauri installers added when M12 exists

## Non-goals

- Docker/OCI image publication — a later, separate decision.
- Signing or notarisation.
- Targets nobody tests.

## Dependencies

- M13-009

## Implementation notes

Only promise targets that are actually built and smoke-tested in CI. An
advertised `aarch64` artefact that nobody ever ran is worse than not offering it,
because someone will deploy it to a Raspberry Pi and discover the problem there.

Embed the version at build time from the tag and assert `--version` matches, so a
mis-tagged release fails in CI rather than in a user's hands.

This must not block M1–M8. It is packaging for a system that already works.

## Acceptance criteria

- [ ] A `v*` tag produces artefacts for every declared target.
- [ ] Every artefact is smoke-tested in CI before publication.
- [ ] `--version` matches the tag.
- [ ] `SHA256SUMS` is complete and correct.
- [ ] Artefacts attach to the GitHub Release.
- [ ] Only tested targets are advertised.
- [ ] Nothing in M1–M8 depends on this workflow.

## Verification

```bash
git tag -a v0.1.0 -m 'test' && git push --tags   # observe the run
sha256sum -c SHA256SUMS
```

## Tests required

- A dry-run tag on a branch.
- Checksum verification.
- `--version` assertion.

## Documentation impact

- README installation section.
- A release procedure document.

## Files likely affected

```text
.github/workflows/release.yml
```
