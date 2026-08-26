# Issue M13-015 — Add the optional Prometheus and Grafana deployment profile

**Milestone:** M13 · **PRD:** [PRD 130](../../prd/130-multi-plant-home.md) · **Depends on:** M13-009

## Context

[ADR-010](../../adr/010-observability-strategy.md) makes Grafana an **optional**
profile, never a dependency. The two data classes must stay separate: operational
metrics in Prometheus, plant history in the databases.

## Goal

Offer an opt-in observability stack without making anything depend on it.

## Scope

- A Compose profile `observability` adding Prometheus and Grafana
- Prometheus scraping the edge and cloud `/metrics`
- Grafana provisioned with a Prometheus datasource for operational dashboards
- Grafana provisioned with a SQL datasource for plant history
- Starter dashboards: system health, and plant history
- **Raw plant telemetry is not exported to Prometheus**

## Non-goals

- Making Grafana required for anything.
- Replacing the Tauri UI.
- Any control path from Grafana.
- Alerting rules.

## Dependencies

- M13-009

## Implementation notes

The profile must be genuinely optional: `docker compose up` without
`--profile observability` must behave exactly as before, and the M8 acceptance
suite must not reference it.

Do not put per-plant, per-kind measurement series into Prometheus. It is the
tempting shortcut because Grafana already reads Prometheus, and it would push
high-cardinality ledger data into a store designed for low-cardinality operational
data with the wrong retention semantics. Plant history is read from SQLite or
PostgreSQL through a SQL datasource.

Grafana is read-only. No control path, no configuration, no actuation.

## Acceptance criteria

- [ ] `--profile observability` starts Prometheus and Grafana.
- [ ] Without the profile, nothing changes and nothing fails.
- [ ] Prometheus scrapes both services.
- [ ] **No plant measurement series appear in Prometheus.**
- [ ] A plant-history dashboard reads from the database, not Prometheus.
- [ ] The M8 suite does not reference the profile.
- [ ] Grafana has no write path to the system.

## Verification

```bash
docker compose --profile observability up -d
curl -s localhost:9090/api/v1/label/__name__/values | jq  # no plant series
docker compose up -d   # unchanged without the profile
```

## Tests required

- Profile opt-in behaviour.
- An explicit assertion that no plant telemetry reaches Prometheus.

## Documentation impact

- ADR-010 Grafana section verified.
- deployment-model.md observability profile.

## Files likely affected

```text
deploy/docker-compose.yml
deploy/observability/prometheus.yml
deploy/observability/grafana/
```
