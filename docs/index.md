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

Other tools number their edges `0, 1, 2, …`, and the numbers change at each rebuild. duckOSM's `edge_id` is a hash of
OSM ids, so it stays the same:

```text
edge_id = hash(osm_id, source, target)
```

<div class="grid cards" markdown>

-   :material-refresh:{ .lg .middle } **Across rebuilds**

    ---

    The same OSM data gives the same ids.

-   :material-crop:{ .lg .middle } **Across clips**

    ---

    A city cut from a country keeps the country's ids.

-   :material-export-variant:{ .lg .middle } **Across exports**

    ---

    SUMO, MATSim, GMNS, GeoPackage and networkx use the `edge_id`.

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

Next: [Your first network](first_network.md).

## What you get

| | |
|---|---|
| **Networks** | driving, walking and cycling in one file, with the same `edge_id` in each mode ([details](concepts/networks.md#access-private-and-forbidden-roads)) |
| **Routing** | the graph of legal turns (`edge_graph`) with OSM turn restrictions ([Route](guides/route.md)) |
| **Base-map layers** | water, land use, buildings, POIs in `features.*` ([Base-map layers](reference/features.md)) |
| **Maps** | offline HTML maps and a route planner ([Draw a map](guides/draw-map.md)) |
| **Drawing order** | which road is painted over which ([Store the drawing order](guides/drawing-order.md)) |
| **Any area** | a boundary, a place name, a box or an H3 cell ([Prepare an area](guides/prepare-area.md)) |
| **Speed** | the Rust engine builds Estonia in 19 seconds ([The Rust engine](concepts/rust-engine.md)) |
| **Fixes** | your fixes for OSM errors, on every build ([Fix OSM errors](guides/fix-osm-errors.md)) |

## Exports

| Format | Command | How it is checked |
|---|---|---|
| [**SUMO**](exports/sumo.md) | `duckosm sumo` | SUMO's `netconvert` |
| [**MATSim**](exports/matsim.md) | `duckosm matsim`, `matsim-lanes` | MATSim's DTD and XSD schemas |
| [**GMNS**](exports/gmns.md) | `duckosm gmns` | the GMNS table specification |
| [**GeoPackage / shapefile**](exports/gis.md) | `duckosm export-gis` | `duckosm gis-debug` compares each `edge_id` through GDAL |
| [**networkx**](exports/networkx.md) | `duckosm export-graph` | unit tests |

Experimental exports and features: [Exports](exports/index.md). The full text of this page:
[home in detail](https://github.com/Khoshkhah/duckOSM/blob/main/docs/design/home_details.md).

---

duckOSM is MIT-licensed and built on [DuckDB](https://duckdb.org) and its spatial extension.
It is an independent project, not affiliated with or endorsed by the DuckDB Foundation.
