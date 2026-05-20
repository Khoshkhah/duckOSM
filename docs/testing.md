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

When splitting self-loop edges (circular roads), virtual nodes are created:

- **Identification**: `node_id < 0`
- **ID Formula**: `virtual_node_id = -(original_edge_id)`
- **Example**: Edge `456` → Virtual node `-456`

---

## Running All Validations

```bash
# Full validation
python scripts/validate_geometry.py --db data/output/somerset.duckdb

# Compare with legacy importer
python scripts/compare_results.py
```
