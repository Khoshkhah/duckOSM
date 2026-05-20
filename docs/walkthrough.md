# duckOSM Development Walkthrough

## Overview

duckOSM is a high-performance OSM-to-routing-network converter built entirely on DuckDB.

## Performance Results

| Dataset | Nodes | Edges | Time |
|---------|-------|-------|------|
| Somerset (test) | 13,207 | 3,428 | **0.22s** |
| GTA (production) | 2,383,475 | 1,031,410 | **5.68s** |

## Key Technical Details

### PBF Parsing
Uses DuckDB's native `ST_READOSM` function from the spatial extension:
```sql
SELECT * FROM ST_READOSM('input.osm.pbf')
```

### Tag Access
DuckDB returns tags as `MAP(VARCHAR, VARCHAR)`. Access with:
```sql
tags['highway'][1]  -- Get highway value
```

### Edge Creation
Creates bidirectional edges by default, respects oneway tags:
```sql
-- Forward edge: source → target
-- Reverse edge: target → source (if not oneway)
```

### H3 Integration
Attempts DuckDB extension first, falls back to Python:
```python
con.execute("INSTALL h3 FROM community; LOAD h3;")
```

## Test Commands

```bash
# Somerset (small)
python -m duckosm --pbf files/Somerset.osm.pbf --output test.duckdb

# GTA (large)
python -m duckosm --pbf cell_832b9bfffffffff.osm.pbf --output gta.duckdb
```

## Future Work

- [ ] Graph simplification (contract degree-2 nodes)
- [ ] Boundary filtering during import
- [ ] Full geometry linestrings (currently just endpoints)
- [ ] Parquet export option
