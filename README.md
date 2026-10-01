<p align="center">
  <img src="https://raw.githubusercontent.com/Khoshkhah/duckOSM/main/images/duckosm-banner.svg" alt="duckOSM" width="720">
</p>

<p align="center">
  <b>OpenStreetMap → a routable road network in one DuckDB file,<br>
  with edge IDs that survive every rebuild, clip and export.</b>
</p>

<p align="center">
  <a href="https://pypi.org/project/duckosm/"><img alt="PyPI" src="https://img.shields.io/pypi/v/duckosm"></a>
  <a href="https://github.com/Khoshkhah/duckOSM/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/Khoshkhah/duckOSM/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://khoshkhah.github.io/duckOSM/"><img alt="Docs" src="https://img.shields.io/badge/docs-khoshkhah.github.io%2FduckOSM-17191f"></a>
  <img alt="Python 3.10+" src="https://img.shields.io/badge/python-3.10%2B-3776AB">
  <img alt="License MIT" src="https://img.shields.io/badge/license-MIT-blue">
</p>

<p align="center">
  <a href="https://khoshkhah.github.io/duckOSM/"><b>Documentation</b></a> ·
  <a href="https://khoshkhah.github.io/duckOSM/first_network/">Your first network</a> ·
  <a href="https://khoshkhah.github.io/duckOSM/guides/build/">Guides</a> ·
  <a href="https://khoshkhah.github.io/duckOSM/reference/database/">Database schema</a> ·
  <a href="https://khoshkhah.github.io/duckOSM/guides/query/">Query the database</a>
</p>

---

## Why duckOSM

Most OSM-to-network tools number their edges `0, 1, 2, …`, so the numbers change whenever you
rebuild, cut out an area or export, and every table keyed on them has to be matched again.
duckOSM's `edge_id` is a **content hash** of three ids that OpenStreetMap already gives the road
segment: the OSM **way** it comes from (`osm_id`) and the OSM **nodes** where it starts and ends
(`source`, `target`):

```text
edge_id = hash(osm_id, source, target)
```

The order of `source` and `target` is the direction, so the two directions of a two-way street get
different ids. None of the three is a number duckOSM makes up, so the id stays the same
**across rebuilds, clips and exports** ([details](https://khoshkhah.github.io/duckOSM/concepts/edge-ids/)).

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

## What you get

- **A driving, a walking and a cycling network** in one file, one schema each, split at the same
  junctions so a road has the same `edge_id` in every mode. OSM access tags are respected: private
  roads are kept for the map but never routed ([details](https://khoshkhah.github.io/duckOSM/concepts/networks/#access-private-and-forbidden-roads)).
- **The graph of legal turns** (`edge_graph`), with OSM turn restrictions, for routing in SQL,
  Python or networkx.
- **Base-map layers** (`features.*`: water, land use, buildings, POIs) in the same file.
- **Maps in one offline HTML page:** `duckosm viz` draws each network; `duckosm route-map` is a route
  planner where you drag a start and an end and get turn-by-turn directions ([try it](https://khoshkhah.github.io/duckOSM/guides/route/)).
  For a full base map drawn from the database and a walk + drive planner, see
  [mapstyle](https://github.com/Khoshkhah/mapstyle) (`pip install mapstyle`).
- **Any area:** a GeoJSON boundary, a place name, a box or an H3 cell; `duckosm boundary` finds an
  area's border by name, and `duckosm extract` cuts a city out of a country build in seconds.
- **Your own fixes for OSM errors**, applied on every build ([how](https://khoshkhah.github.io/duckOSM/guides/fix-osm-errors/)).

## Exports

| Format | Command | How it's verified |
|---|---|---|
| **SUMO** | `duckosm sumo` | SUMO's own `netconvert` assembles the network |
| **MATSim** | `duckosm matsim`, `matsim-lanes` | validated against MATSim's official DTD and XSD schemas |
| **GMNS** | `duckosm gmns` | follows the GMNS table spec |
| **GeoPackage / shapefile** | `duckosm export-gis` | `duckosm gis-debug` reads it back through GDAL and diffs every `edge_id` |
| **networkx** | `duckosm export-graph` | unit tests |

**Experimental:** OpenDRIVE, Lanelet2, railML, lane-level routing, GMNS meso/micro, intermodal
routing, elevation, admin boundaries, base-map layers. See the
[documentation](https://khoshkhah.github.io/duckOSM/).

## For AI agents

- To use duckOSM: the agent skill [`skills/duckosm/SKILL.md`](skills/duckosm/SKILL.md) (also a
  Claude Code plugin: `/plugin marketplace add Khoshkhah/duckOSM`), and the whole documentation as
  one text file: [`llms-full.txt`](https://khoshkhah.github.io/duckOSM/llms-full.txt)
  ([`llms.txt`](https://khoshkhah.github.io/duckOSM/llms.txt) is its index).
- `duckosm info DB --json` says what a database holds; `way`, `route-lanes` and `gis-debug` also
  take `--json`.
- To change duckOSM: the rules are in [`AGENTS.md`](AGENTS.md).

## License

MIT. duckOSM is built on [DuckDB](https://duckdb.org) and its spatial extension; it is an
independent project, not affiliated with or endorsed by the DuckDB Foundation.
