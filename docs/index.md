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

[Get started](user_manual.md){ .md-button .md-button--primary }
[View on GitHub](https://github.com/Khoshkhah/duckOSM){ .md-button }

## Why duckOSM

Most OSM-to-network tools number their edges `0, 1, 2, …`. Rebuild with fresher OSM data, cut out
a smaller area, or export to a simulator, and the numbers change, so every table keyed on them
(traffic counts, speeds, map matches, demand) has to be matched to the network again.

duckOSM's `edge_id` is a **content hash** of the road segment, so it stays the same:

<div class="grid cards" markdown>

-   :material-refresh:{ .lg .middle } **Across rebuilds**

    ---

    The same OSM data gives the same ids. An id only changes where that road was edited in OSM.

-   :material-crop:{ .lg .middle } **Across clips**

    ---

    Build a country once, cut out a city in seconds: the city keeps the country's ids.

-   :material-export-variant:{ .lg .middle } **Across exports**

    ---

    SUMO, MATSim, GMNS, GeoPackage and networkx all use the `edge_id` as their own id.

</div>

One real segment of Högbergsgatan, Stockholm:

```text
stockholm_county.duckdb   driving.edges      edge_id = 7968481847680619937   # county build
sodermalm.duckdb          driving.edges      edge_id = 7968481847680619937   # separate Södermalm build
sodermalm (clipped)       driving.edges      edge_id = 7968481847680619937   # duckosm extract from the county
SUMO    soder.edg.xml     <edge id="7968481847680619937" from="7767910376" to="21761577" …>
MATSim  network.xml       <link id="7968481847680619937" from="7767910376" to="21761577" …>
GMNS    gmns_driving.link  link_id = 7968481847680619937
```

Attach your data to an `edge_id` once and it follows the road everywhere.

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
r["time_s"], r["length_m"], r["edges"]   # ~130 s, ~1.7 km, the ordered edge_ids
```

Next: the [user manual](user_manual.md) for every command, or the
[data dictionary](data_dictionary.md) for what's in a build.

## Exports

| Format | Command | How it's verified |
|---|---|---|
| **SUMO** | `duckosm sumo` | SUMO's own `netconvert` assembles the network; turn restrictions become explicit connections |
| **MATSim** | `duckosm matsim`, `matsim-lanes` | validated against MATSim's official DTD and XSD schemas |
| **GMNS** | `duckosm gmns` | follows the GMNS table spec |
| **GeoPackage / shapefile** | `duckosm export-gis` | `duckosm gis-debug` reads the file back through GDAL and diffs every `edge_id` |
| **networkx** | `duckosm export-graph` | unit tests; GraphML or lossless gpickle |

**Experimental:** OpenDRIVE, Lanelet2, railML, lane-level routing, GMNS meso/micro networks,
intermodal routing, elevation, admin boundaries and base-map layers. They work and are
structure-tested, but haven't been checked in the target tools yet. See the **Experimental** tab.

---

duckOSM is MIT-licensed and built on [DuckDB](https://duckdb.org) and its spatial extension.
It is an independent project, not affiliated with or endorsed by the DuckDB Foundation.
