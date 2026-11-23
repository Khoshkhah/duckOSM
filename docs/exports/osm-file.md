# OSM file

Level 0 of the outputs: the OSM data of your area as one `.osm.pbf` file, **with your OSM fixes in it**, powered by **[pbfcut](https://github.com/Khoshkhah/pbfcut)**. Other tools then read the edited data: OSMnx, osm2pgsql, SUMO's OSM import, QGIS, osmium.

| Reference and format | |
|---|---|
| Format | OpenStreetMap [PBF format](https://wiki.openstreetmap.org/wiki/PBF_Format), sorted by type, then id, with dense nodes |
| Engine | [pbfcut](https://github.com/Khoshkhah/pbfcut): fast Rust tool for cutting to boundaries and partitioning areas into tiles |
| Input | the [Geofabrik downloads](https://download.geofabrik.de): extracts of continents, countries and regions, updated each day |
| Cut & Partition | [pbfcut](https://github.com/Khoshkhah/pbfcut), with osmium's `smart` rule; partitions large areas into tiles for low memory |
| Check | on Monaco, a build of the cut file gives the same tables as a build with `--fixes` of the original file ([Monaco check](#monaco-check)) |

```bash
duckosm get "Monaco"                         # -> monaco.osm.pbf and monaco.geojson
duckosm build --pbf monaco.osm.pbf --boundary monaco.geojson
```

## Powered by `pbfcut`

Boundary cutting and tile partitioning run on **[pbfcut](https://github.com/Khoshkhah/pbfcut)**:

* **Boundary cutting:** cuts extracts to exact polygons and embeds your fixes into the `.osm.pbf`.
* **Tile partitioning:** splits large regions into tiles so duckOSM builds whole countries in low memory.

## `duckosm get`

Find an area, download its OSM data and cut it, in one command.

```bash
duckosm get "Monaco"                         # a place name
duckosm get --osm-id 2220322                 # one OSM boundary relation
duckosm get --bbox 7.40,43.72,7.44,43.76     # a box: west,south,east,north
duckosm get --h3-cell 883969a40bfffff        # an H3 cell
duckosm get --boundary area.geojson          # your boundary file
duckosm get "Tartu" --fixes osm_fixes.yaml   # the file has your fixes in it
```

| Step | What it does |
|---|---|
| 1. The boundary | A name or an OSM id goes to OpenStreetMap's Nominatim search. A box, a cell or a file needs no network. Writes `<name>.geojson` ([the boundary file](../reference/boundary.md)). |
| 2. The extract | Picks the smallest Geofabrik extract that holds the whole boundary, as duckOSM Studio does. A box in Monaco that crosses into France takes Provence-Alpes-Côte-d'Azur. |
| 3. The download | Into `~/.cache/duckosm/geofabrik/`. A second area in the same region downloads nothing. A stopped download continues, and the MD5 sum is checked. |
| 4. The cut | Cuts the extract to the boundary, with `--fixes` when you give it. Writes `<name>.osm.pbf`. |

| Option | Does | Default |
|---|---|---|
| `PLACE` | the name of the area | give one of `PLACE`, `--osm-id`, `--bbox`, `--h3-cell`, `--boundary` |
| `--osm-id ID` | one OSM boundary relation, for a name with more than one match | |
| `--bbox W,S,E,N` | a box in degrees | |
| `--h3-cell CELL` | an H3 cell | |
| `--boundary FILE` | a GeoJSON boundary file | |
| `-o`, `--out DIR` | the output folder | `.` |
| `--name NAME` | the name of the two files | from the place, the file or the cell; `area` for a box |
| `--fixes FILE` | your OSM fixes file | |
| `--cache DIR` | the folder of the downloads | `$DUCKOSM_CACHE`, else `~/.cache/duckosm` |
| `--refresh` | download the extract and the list of extracts again | the files in the cache are used |
| `--dry-run` | find the boundary and the extract, print them as JSON and stop: no download, no files | |

The list of extracts stays in the cache for 7 days. An extract in the cache stays until you use `--refresh`.

## `duckosm cut`

Cut an OSM file that you already have to a boundary.

```bash
duckosm cut estonia-latest.osm.pbf --boundary tartu.geojson -o tartu.osm.pbf --fixes osm_fixes.yaml
```

The input must be sorted by type, then id, as the Geofabrik files are. `--boundary` reads GeoJSON, an osmosis `.poly`
file, WKT or a box `W,S,E,N`.

## The fixes in the file

| Fix | In the file |
|---|---|
| `remove` | the way is not in the file |
| `keep_nodes` | the way has only the nodes from the first to the second node |
| `tags`, `extra.tag_edits` | the changed tags of the way, node or relation |
| `oneway`, `lanes`, `lanes_forward`, `lanes_backward`, `layer` | the OSM tag with the value of the fix |
| `new_ways` | a new way with the id of the fixes file (from 9 000 000 000 000), and its new nodes |
| `turn_restrictions` | a relation `type=restriction` with a negative id, when its two ways are in the file |
| `exclude_modes`, `turn_lanes_forward`, `turn_lanes_backward` | not in the file: no OSM tag holds them |

The new ids are larger than all OSM ids, and the turn relations have negative ids, so the file stays sorted.

## Monaco check

A build of the cut file and `duckosm build --fixes` of the original file give the same 20 tables, the same rows and the
same `edge_id` values. The small differences:

| Difference | Cause |
|---|---|
| `ways.tags` has the fix as an OSM tag (`oneway`, `lanes`, `layer`) | in the cut file, a fix is an OSM tag |
| 1 way has no backward lanes (`oneway: true, lanes: 3` on a two-way road) | `--fixes` gives lanes to each direction; the tags `oneway=yes, lanes=3` do not |
| the last bit of one latitude, on 2 edges | the PBF format keeps a coordinate in steps of 10⁻⁷ degrees |

## Python

```python
import duckosm

area = duckosm.get("Monaco")                   # area.pbf, area.boundary: the two paths
db = duckosm.build(pbf=area.pbf, boundary=area.boundary)
```

To choose the area on a map in a notebook, see [Choose an area in Jupyter](../guides/area-picker.md).
See [Python SDK](../reference/python.md#build-and-areas).
