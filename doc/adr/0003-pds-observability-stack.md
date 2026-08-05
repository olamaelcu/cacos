# 3. PDS Observability Stack

Date: 2026-08-05

## Status

Accepted

## Context

The PDS needs structured logs, metrics, and latency visibility for its HTTP handlers, sequencer, and blob storage.

## Decision

An `observability` module owns a global tracing registry that layers `EnvFilter`, `MetricsLayer`, `fmt`, and a `TimingLayer`, returning a downcaster handle. A Prometheus recorder is installed globally with a shared `cacos_` metric namespace; its render handle backs a poem `GET /metrics` route. A background reporter folds tracing-timing histogram percentiles (p50/p90/p99) into gauges. Global installs are idempotent, and unit tests scope metrics locally.

## Consequences

Every subsystem shares one `cacos_` metric namespace referenced through constants. Timing percentiles are exposed as three per-quantile gauges because Prometheus names cannot contain `*`. Repeated initialization is safe. The `/metrics` render drains histograms, so no upkeep task is needed.
