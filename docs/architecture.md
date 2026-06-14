# duckOSM Architecture

## Overview

duckOSM converts OpenStreetMap PBF files to routing-ready DuckDB databases using pure SQL processing.

## Pipeline Overview

```
                     ┌─────────────────────────────────────────┐
                     │             GLOBAL (Once)               │
                     ├─────────────────────────────────────────┤
                     │  1. Connect to DuckDB                   │
                     │  2. Load PBF (ST_READOSM)               │
                     │     → raw.nodes, raw.ways, raw.relations│
                     └─────────────────────────────────────────┘
                                      │
            ┌─────────────────────────┼─────────────────────────┐
            ▼                         ▼                         ▼
     ┌───────────┐             ┌───────────┐             ┌───────────┐
     │  driving  │             │  walking  │             │  cycling  │
     └─────┬─────┘             └─────┬─────┘             └─────┬─────┘
           │                         │                         │
           ▼                         ▼                         ▼
     ┌─────────────────────────────────────────────────────────────┐
     │  1. Filter Roads  →  2. Build Edges  →  3. Simplify Graph  │
     │  4. Process Speeds → 5. Calculate Costs → 6. Restrictions  │
     │  7. Edge Graph  →  8. H3 Indexing  →  9. Create Indexes    │
     └─────────────────────────────────────────────────────────────┘
```

---

## Step 1: Load PBF

```sql
CREATE TABLE raw.nodes AS 
  SELECT * FROM ST_READOSM('file.pbf') WHERE kind = 'node';
CREATE TABLE raw.ways AS 
  SELECT * FROM ST_READOSM('file.pbf') WHERE kind = 'way';
CREATE TABLE raw.relations AS 
  SELECT * FROM ST_READOSM('file.pbf') WHERE kind = 'relation';
```

> [!TIP]
> For more information on loading OSM data into DuckDB, see these resources:
> - [How to read OSM data with DuckDB (Medium)](https://medium.com/data-science/how-to-read-osm-data-with-duckdb-ffeb15197390)
> - [DuckDB Spatial Extension GitHub](https://github.com/duckdb/duckdb-spatial)

---

## Step 2: Filter Roads

```sql
CREATE TABLE ways AS
SELECT osm_id, tags['highway'] AS highway, tags['name'] AS name, refs, ...
FROM raw.ways
WHERE tags['highway'] IS NOT NULL
  AND tags['highway'] IN ('motorway', 'primary', 'secondary', 'residential', ...);
```

### Lane count (`lanes`)

The `lanes` column on each edge is an **integer lane count for that edge's direction of
travel**, and is never NULL. It is computed in two parts.

**1. Per-direction counts on `ways`** (in `road_filter`). The OSM tags are parsed to
integers — `regexp_extract(..., '\d+')` takes the first integer, so `"2"`, `"2;3"` and
`"1.5"` all yield a value; `0`, blanks and non-numeric values become NULL. A forward and a
backward count are then derived:

```sql
-- n_total = lanes, n_fwd = lanes:forward, n_bwd = lanes:backward (parsed ints)
-- Forward direction:
CASE
    WHEN n_fwd IS NOT NULL THEN n_fwd                      -- explicit lanes:forward wins
    WHEN oneway                                            -- one-way (boolean): all lanes forward
        THEN COALESCE(n_total, default_lanes)
    WHEN n_total IS NOT NULL AND n_bwd IS NOT NULL
        THEN GREATEST(n_total - n_bwd, 1)                  -- remainder of the total
    WHEN n_total IS NOT NULL
        THEN GREATEST(CEIL(n_total / 2.0)::INT, 1)         -- two-way split, larger half
    ELSE default_lanes                                     -- nothing tagged → fallback
END AS lanes_fwd
-- Backward direction is symmetric: lanes:backward, else (n_total - n_fwd),
-- else FLOOR(n_total / 2), else default_lanes.
```

`default_lanes` is a class-based fallback applied **only when nothing is tagged**:
**motorway / trunk → 2, every other highway class → 1.**

**2. Each edge picks its side** (in `graph_builder` / `graph_simplifier`). The forward edge
takes `lanes_fwd`; the reverse edge created for two-way roads takes `lanes_bwd` (looked up
by joining the edge back to its way on `osm_id`). One-way roads have no reverse edge, so all
lanes stay on the single forward edge.

> [!NOTE]
> OSM lane tagging is dense on major roads but sparse on local roads (in Sweden:
> motorway ~100%, trunk ~93%, residential/service <3%), so the class-based fallback is what
> keeps the column fully populated.

---

## Step 3: Build Initial Edges

```sql
CREATE TABLE edges AS
SELECT 
    row_number() OVER ()::INTEGER AS edge_id,
    refs[1] AS source,
    refs[len(refs)] AS target,
    osm_id,
    ST_MakeLine(coordinates) AS geometry
FROM ways;
```

---

## Step 4: Graph Simplification

### 4a. Find Junctions
```sql
CREATE TABLE junctions AS
SELECT node_id FROM node_counts
WHERE way_count > 1 OR endpoint_count > 0;
```

### 4b. Segment Ways at Junctions
```sql
-- Split ways into segments between consecutive junctions
```

### 4c. Calculate Lengths with Haversine
```sql
-- Note: ST_Length_Spheroid has precision issues for certain coordinate ranges
-- We use a manual Haversine summation over all geometry vertices instead
WITH edge_points AS (
    SELECT edge_id, idx,
        ST_X(ST_PointN(geometry, idx)) as lon,
        ST_Y(ST_PointN(geometry, idx)) as lat
    FROM point_indices
)
SELECT edge_id, SUM(
    12742000 * ASIN(SQRT(
        POWER(SIN(RADIANS(p2.lat - p1.lat) / 2), 2) +
        COS(RADIANS(p1.lat)) * COS(RADIANS(p2.lat)) *
        POWER(SIN(RADIANS(p2.lon - p1.lon) / 2), 2)
    ))
) AS length_m
FROM edge_points p1
JOIN edge_points p2 ON p1.edge_id = p2.edge_id AND p2.idx = p1.idx + 1
GROUP BY edge_id;
```

### 4d. Split Self-Loops
```sql
-- Split edge where source = target
SELECT 
    -(edge_id) AS virtual_node_id,  -- Negative ID
    ST_LineSubstring(geometry, 0, 0.5) AS first_half,
    ST_LineSubstring(geometry, 0.5, 1) AS second_half
FROM edges WHERE source = target;
```

### 4e. Add Reverse Edges (Two-Way Roads)
```sql
INSERT INTO edges
SELECT 
    e.target AS source, e.source AS target,
    w.lanes_bwd AS lanes,          -- reverse edge carries the backward lane count
    ST_Reverse(e.geometry),
    TRUE AS is_reverse
FROM edges e
JOIN ways w ON w.osm_id = e.osm_id
WHERE NOT e.oneway;          -- oneway is a boolean: FALSE = two-way
```

---

## Step 5: Process Speeds

```sql
ALTER TABLE edges ADD COLUMN maxspeed_kmh FLOAT;
UPDATE edges SET maxspeed_kmh = 
    CASE 
        WHEN maxspeed LIKE '%mph%' THEN CAST(regexp_extract(maxspeed, '\d+') AS FLOAT) * 1.60934
        WHEN maxspeed ~ '^\d+$' THEN CAST(maxspeed AS FLOAT)
        ELSE (SELECT default_speed FROM highway_defaults WHERE highway = edges.highway)
    END;

-- maxspeed_kmh is the normalized, always-populated speed; drop the raw OSM string.
ALTER TABLE edges DROP COLUMN maxspeed;
```

---

## Step 6: Calculate Costs

```sql
ALTER TABLE edges ADD COLUMN cost_s FLOAT;
UPDATE edges SET cost_s = length_m / (maxspeed_kmh / 3.6);
```

---

## Step 7: Build Edge Graph (Line Graph)

```sql
CREATE TABLE edge_graph AS
SELECT 
    e1.edge_id AS from_edge,
    e2.edge_id AS to_edge,
    e2.edge_id AS via_edge,
    e1.cost_s AS cost
FROM edges e1
JOIN edges e2 ON e1.target = e2.source
WHERE e1.edge_id != e2.edge_id;
```

---

## Step 8: H3 Indexing

```sql
ALTER TABLE edges ADD COLUMN from_cell UBIGINT;
ALTER TABLE edges ADD COLUMN to_cell UBIGINT;
UPDATE edges SET 
    from_cell = (SELECT h3_cell FROM nodes WHERE node_id = edges.source),
    to_cell = (SELECT h3_cell FROM nodes WHERE node_id = edges.target);
```

---

## Type Summary

| Column | Type | 
|--------|------|
| `edge_id` | INTEGER |
| `source/target` | BIGINT |
| `from_cell/to_cell` | UBIGINT |
| `length_m/cost_s/maxspeed_kmh` | FLOAT |
| `lanes` | INTEGER |
| `oneway` | BOOLEAN |
