# Issue M14-007 — Plan optional Helm packaging for server-side components

**Milestone:** M14 · **PRD:** [PRD 140](../../prd/140-field-readiness.md) · **Depends on:** M14-001

## Context

Kubernetes is not required for an indoor plant deployment and must not leak into
the product architecture. It is a packaging option for server-side components,
later.

## Goal

Specify what Helm would package, and what it must never package.

## Scope

- Chart scope: `cloud-api`, optionally Prometheus and Grafana
- PostgreSQL only when not operator-managed or external
- **The plant-side edge controller is explicitly out of scope**
- Container interface prerequisites: config via env, health endpoints, graceful SIGTERM, stateless replicas
- Explicitly excluded: service mesh, operators, distributed databases, microservice decomposition
- A statement that home deployment remains Compose or systemd

## Non-goals

- Writing a chart.
- Any Kubernetes dependency in V1.
- Running the edge controller in Kubernetes.

## Dependencies

- M14-001

## Implementation notes

State the boundary in a way that survives enthusiasm: the edge controller is
metres from a pump and must run when the network is down. Putting it behind a
scheduler adds failure modes to the one component whose entire purpose is working
when things fail.

Helm is worth planning only after the container interfaces are stable, which is
why this is M14 and not earlier. Record the prerequisites so the eventual chart is
a packaging exercise rather than a redesign.

The presence of a configured `kubectl` on a developer's machine is not an
architectural input.

## Acceptance criteria

- [ ] Chart scope is specified with clear inclusions and exclusions.
- [ ] The edge controller is explicitly excluded, with the reasoning.
- [ ] Container interface prerequisites are listed.
- [ ] Anti-goals are named explicitly.
- [ ] Home deployment is documented as Compose or systemd.
- [ ] **No chart is written and no Kubernetes dependency is added.**

## Verification

```bash
cargo run -p rhizo-docscheck
```

## Tests required

- Review-based; the document is the artefact.

## Documentation impact

- docs/architecture/deployment-model.md future section.

## Files likely affected

```text
docs/architecture/deployment-model.md
docs/prd/140-field-readiness.md
```
