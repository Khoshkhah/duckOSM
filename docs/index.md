---
hide:
  - navigation
  - toc
---

<div class="dk-hero" markdown>
![duckOSM](assets/duckosm-mark.svg)
<div markdown>
# duckOSM

OpenStreetMap → a routable road network in one DuckDB file,
with **edge IDs that survive every rebuild, clip and export**.
</div>
</div>

[Get started](first_network.md){ .md-button .md-button--primary }
[View on GitHub](https://github.com/Khoshkhah/duckOSM){ .md-button }

## Why duckOSM

Most OSM-to-network tools number their edges `0, 1, 2, …`. Rebuild with fresher OSM data, cut out
a smaller area, or export to a simulator, and the numbers change, so every table keyed on them
(traffic counts, speeds, map matches, demand) has to be matched to the network again.

duckOSM's `edge_id` is a **content hash** of three ids that OpenStreetMap already gives the road
segment: the OSM **way** it comes from (`osm_id`) and the OSM **nodes** where it starts and ends
(`source`, `target`):

```text
edge_id = hash(osm_id, source, target)
```

The order of `source` and `target` is the direction, so the two directions of a two-way street get
different ids. None of the three is a number duckOSM makes up, so the id stays the same:

<div class="grid cards" markdown>

-   :material-refresh:{ .lg .middle } **Across rebuilds**

    ---

    The same OSM data gives the same ids. An id only changes if that way or its end nodes change in OSM.

-   :material-crop:{ .lg .middle } **Across clips**

    ---

    Build a country once, cut out a city in seconds: the city keeps the country's ids.

-   :material-export-variant:{ .lg .middle } **Across exports**

    ---

    SUMO, MATSim, GMNS, GeoPackage and networkx all use the `edge_id` as their own id.

</div>

[How the id is built, in detail](concepts/edge-ids.md)

## Quick start

```bash
pip install "duckosm[routing]"

curl -LO https://download.geofabrik.de/europe/monaco-latest.osm.pbf
duckosm build --pbf monaco-latest.osm.pbf --output monaco.duckdb --modes driving
```

```python
import duckdb
from duckosm import route

con = duckdb.connect("monaco.duckdb", read_only=True)
q = "SELECT min(edge_id) FROM driving.edges WHERE name = ?"
a = con.execute(q, ["Boulevard du Larvotto"]).fetchone()[0]
b = con.execute(q, ["Avenue Princesse Grace"]).fetchone()[0]

r = route(con, a, b)                     # fastest path; weight="length" for distance
r["time_s"], r["length_m"], r["edges"]   # ~114 s, ~1.5 km, the ordered edge_ids
```

Next: [Your first network](first_network.md) walks through it step by step, and the
[guides](guides/build.md) cover each task.

## What you get

- **A driving, a walking and a cycling network** in one file, one schema each, split at the same
  junctions so a road has the same `edge_id` in every mode. OSM access tags are respected: private
  roads are kept for the map but never routed ([details](concepts/networks.md#access-private-and-forbidden-roads)).
- **The graph of legal turns** (`edge_graph`), with OSM turn restrictions, for routing in SQL,
  Python or networkx ([Route](guides/route.md)).
- **Base-map layers** (`features.*`: water, land use, buildings, POIs) in the same file
  ([Base-map layers](reference/features.md)).
- **Maps in one offline HTML page:** `duckosm viz` draws the network and the base map from the
  database ([Draw a map](guides/draw-map.md)); `duckosm route-map` is a route planner (drive, walk,
  cycle, walk + drive) where you drag a start and an end and get turn-by-turn directions
  ([try it](guides/route.md#on-a-map)). The pages are drawn by
  [mapstyle](https://khoshkhah.github.io/mapstyle/).
- **The drawing order of the roads:** `duckosm levels` stores which road is painted over which
  (casing and fill numbers) in the file ([Store the drawing order](guides/drawing-order.md)).
- **Any area:** a GeoJSON boundary, a place name, a box or an H3 cell; `duckosm boundary` finds an
  area's border by name, and `duckosm extract` cuts a city out of a country build in seconds
  ([Prepare an area](guides/prepare-area.md)).
- **Your own fixes for OSM errors**, applied on every build ([Fix OSM errors](guides/fix-osm-errors.md)).

## Exports

| Format | Command | How it's verified |
|---|---|---|
| [**SUMO**](exports/sumo.md) | `duckosm sumo` | SUMO's own `netconvert` assembles the network; turn restrictions become explicit connections |
| [**MATSim**](exports/matsim.md) | `duckosm matsim`, `matsim-lanes` | validated against MATSim's official DTD and XSD schemas |
| [**GMNS**](exports/gmns.md) | `duckosm gmns` | follows the GMNS table spec |
| [**GeoPackage / shapefile**](exports/gis.md) | `duckosm export-gis` | `duckosm gis-debug` reads the file back through GDAL and diffs every `edge_id` |
| [**networkx**](exports/networkx.md) | `duckosm export-graph` | unit tests; GraphML or lossless gpickle |

**Experimental:** OpenDRIVE, Lanelet2, railML, lane-level routing, GMNS meso/micro networks,
intermodal routing, elevation, admin boundaries and base-map layers. They work and are
structure-tested, but haven't been checked in the target tools yet. Which export for which tool:
[Exports](exports/index.md).

---

duckOSM is MIT-licensed and built on [DuckDB](https://duckdb.org) and its spatial extension.
It is an independent project, not affiliated with or endorsed by the DuckDB Foundation.
