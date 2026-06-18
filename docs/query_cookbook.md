# duckOSM Query Cookbook

This document provides useful SQL snippets for exploring and analyzing the road network data in your `.duckdb` file.

## Feature Exploration

### Count the number of features in the processed network
```sql
SELECT 
    (SELECT COUNT(*) FROM nodes) as total_nodes,
    (SELECT COUNT(*) FROM ways) as total_ways,
    (SELECT COUNT(*) FROM edges) as total_edges,
    (SELECT COUNT(*) FROM turn_restrictions) as total_restrictions;
```

### Road Network Breakdown
Show road types ordered by frequency:
```sql
SELECT highway, COUNT(*) as count
FROM ways
GROUP BY highway
ORDER BY count DESC;
```

---

## Road Analysis

### Finding High-Speed Roads
List roads with a speed limit greater than 100 km/h:
```sql
SELECT name, highway, maxspeed_kmh, length_m
FROM edges
WHERE maxspeed_kmh > 100
ORDER BY maxspeed_kmh DESC;
```

### Top 10 Longest Road Segments
```sql
SELECT name, highway, length_m, geometry
FROM edges
ORDER BY length_m DESC
LIMIT 10;
```

### Checking Surface Quality
Compare road surfacing across different highway types:
```sql
SELECT highway, surface, COUNT(*) as count
FROM ways
WHERE surface IS NOT NULL
GROUP BY highway, surface
ORDER BY count DESC;
```

---

## Routing & Adjacency

### Check Turn Restrictions
Identify the most restricted intersections (via_node):
```sql
SELECT via_node, COUNT(*) as restriction_count
FROM turn_restrictions
GROUP BY via_node
ORDER BY restriction_count DESC
LIMIT 10;
```

### Explore Edge Connectivity
Find all outgoing edges from a specific source node:
```sql
SELECT target, name, highway, cost_s, geometry
FROM edges
WHERE source = 123456789; -- Replace with a real node ID
```

### Trace a Path from Edge Graph
Find all possible next steps from a specific edge:
```sql
SELECT e.edge_id, e.name, e.highway
FROM edge_graph eg
JOIN edges e ON eg.to_edge = e.edge_id
WHERE eg.from_edge = 1234567890;   -- a real edge_id
```

### Shortest path between two edges
`edge_graph` is the edge-based routing graph (each node is an `edge_id`; illegal turns already
removed). The one-call helper returns the route between two edge_ids:

```python
import duckdb
from duckosm import route

con = duckdb.connect("data/db/sodermalm.duckdb", read_only=True)
r = route(con, FROM_EDGE, TO_EDGE)            # defaults: fastest (time), service roads excluded
#   r["edges"]    -> ordered list of edge_ids
#   r["time_s"]   -> total travel time (door-to-door)
#   r["length_m"] -> total length
#   r["path"]     -> per-edge name/highway/length_m/cost_s/geometry, in order
#   route(con, a, b, weight="length")   -> shortest by distance
#   route(con, a, b, with_service=True) -> may use service roads
```

`route()` rebuilds the graph each call. For **many** routes use `Router`, which builds the graph
**once**:

```python
from duckosm import Router
r = Router(con)                       # builds the graph once (time, no service)
r.route(FROM_EDGE, TO_EDGE)           # reuses it — no rebuild
r.route(OTHER_FROM, OTHER_TO)
# r.graph is the underlying networkx DiGraph if you want nx algorithms directly
```

Needs `networkx` (`pip install duckosm[routing]`). This loads the whole graph into memory — fine
for a city; at country scale run an on-disk A\* over `edge_graph` instead.

---

## Spatial & H3 Analysis

### Count Roads by H3 Cell
Identify which H3 areas have the highest density of road source points:
```sql
SELECT source_h3, COUNT(*) as road_count
FROM edges
GROUP BY source_h3
ORDER BY road_count DESC
LIMIT 10;
```

### Find Edges within a specific H3 Cell
```sql
SELECT name, highway, length_m
FROM edges
WHERE source_h3 = '882b9b46a1fffff'; -- Replace with your cell ID
```

---

## Raw Data Exploration

### Search for Points of Interest (POIs) in Raw Data
The `raw.nodes` table contains features that are not necessarily part of the road network (like shops, hospitals, etc.)
```sql
SELECT osm_id, tags['name'], tags['amenity']
FROM raw.nodes
WHERE tags['amenity'] IN ('hospital', 'school', 'pharmacy')
  AND tags['name'] IS NOT NULL;
```

### Explore Non-Highway Ways
Find buildings, parks, or rivers from the raw data:
```sql
SELECT osm_id, tags['name'], tags['leisure'], tags['building']
FROM raw.ways
WHERE tags['leisure'] = 'park' OR tags['building'] IS NOT NULL
LIMIT 10;
```
