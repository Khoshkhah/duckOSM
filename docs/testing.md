# duckOSM Testing Guide

This document describes the validation tests available for verifying the integrity of the generated road network.

---

## Geometry Validation

**Script**: `scripts/validate_geometry.py`

**Usage**:
```bash
python scripts/validate_geometry.py --db data/output/somerset.duckdb
```

### Tests Performed

| Test | Description | SQL Logic |
|------|-------------|-----------|
| **Self-Loop Detection** | Ensures no edge starts and ends at the same node | `source = target` |
| **Endpoint Matching** | Verifies edge geometry starts/ends at source/target node coordinates | `ST_Distance(ST_StartPoint(geometry), source_node.geom) < 1e-9` |
| **Degenerate Geometry** | Ensures no zero-length edges exist | `ST_Length(geometry) = 0` |

### Expected Results
- **Self-loops**: 0 (circular roads are split at midpoint)
- **Endpoint mismatches**: 0 (geometry must align with topology)
- **Degenerate edges**: 0

---

## Virtual Node ID Scheme

When splitting self-loops, same-direction parallel arcs, or antiparallel arcs, virtual nodes
are created:

- **Identification**: `node_id < 0`
- **ID Formula**: `-(hash(osm_id, source, refs) >> 2)` (self-loop midpoint) /
  `-(hash(osm_id, source, target, refs) >> 2)` (arc-split midpoint) — content-derived, so
  stable across rebuilds and unique per arc (see `docs/design/split_same_direction_parallels.md`)

---

## Running All Validations

```bash
# Full validation
python scripts/validate_geometry.py --db data/output/somerset.duckdb
```

---

## Unit / integration tests (pytest)

```bash
pip install -e .          # + pytest
pytest tests/ -q
```

`tests/test_pipeline_a.py` covers the build pipeline:

- **config schema** — `source.type: duckdb` parsing and the back-compat flat keys
  (`pbf_path` / `boundary_path`).
- **ComponentFilter** — keeps the largest weakly-connected component and drops a
  disconnected fragment (and its orphaned nodes); no-op when already connected.
- **Validator** — fails on a multi-component graph when `assert_single_component`.
- **duckdb clip (integration)** — clips the parent `data/db/sodermalm.duckdb`, asserting
  every clipped `edge_id` exists **verbatim** in the parent (edge_ids preserved) and the
  result is smaller (fragments dropped). *Skipped if the parent db isn't built.*

## Build-time validation

Every build with `validation.enabled: true` runs the same invariants (single dominant
component, no stranded named edge) and **fails the build** on a breach when
`fail_on_error: true`. See [`pipeline.md`](pipeline.md).
