# The OSM file

Level 0 of the outputs: the OSM data of your area as one `.osm.pbf` file, with your OSM fixes in it.

`duckosm get` finds the area, downloads the OSM data and cuts it in one command. `duckosm cut --fixes` writes the
fixes of [Edit OSM data](../guides/edit-osm-data.md#the-fixes-file) into the file, so other tools read the edited data: OSMnx,
osm2pgsql, SUMO's OSM import, QGIS, osmium.

| Reference and format | |
|---|---|
| Format | OpenStreetMap [PBF format](https://wiki.openstreetmap.org/wiki/PBF_Format), sorted by type, then id (`Sort.Type_then_ID`), with dense nodes |
| Input | the [Geofabrik downloads](https://download.geofabrik.de): the extracts of continents, countries and regions, updated each day |
| Cut | [pbfcut](https://github.com/Khoshkhah/pbfcut): the nodes inside the boundary, each way with a node inside (whole), the relations with a member inside, and all members of the multipolygons among them (osmium's `smart` strategy) |
| Check | on Monaco, `duckosm build` of the cut file without `--fixes` gives the tables of `duckosm build --fixes` on the original file ([Monaco check](#monaco-check)) |

```bash
duckosm get "Monaco"                         # -> monaco.osm.pbf and monaco.geojson
duckosm build --pbf monaco.osm.pbf --boundary monaco.geojson
```

## `duckosm get`

```bash
duckosm get "Monaco"                         # a place name (OpenStreetMap's Nominatim search)
duckosm get --osm-id 2220322 -o out          # one OSM boundary relation, into the folder out/
duckosm get --bbox 7.40,43.72,7.44,43.76     # a box: west,south,east,north
duckosm get --h3-cell 883969a40bfffff        # an H3 cell
duckosm get --boundary area.geojson          # your boundary file
duckosm get "Tartu" --fixes osm_fixes.yaml   # the cut file has the fixes in it
```

The steps:

1. **The boundary.** A name or an OSM id goes to Nominatim, as [`duckosm boundary`](../reference/cli.md#boundary).
   A box, an H3 cell or a file needs no network. The command writes `<name>.geojson` ([the boundary file](../reference/boundary.md)).
2. **The extract.** The smallest extract of Geofabrik's list ([`index-v1.json`](https://download.geofabrik.de/index-v1.json))
   that holds the whole boundary. duckOSM Studio chooses its extract with the same rule.
   An area at a border of an extract goes to the larger extract: a box in Monaco that crosses into France takes
   Provence-Alpes-Côte-d'Azur (390 MB), not Monaco (0.7 MB). The command names the extract before the download.
3. **The download**, into the cache folder `~/.cache/duckosm/geofabrik/` (or `$DUCKOSM_CACHE`). A second area in the
   same region downloads nothing. The download shows its progress, continues after a stop when you run the command
   again, and checks the MD5 sum that Geofabrik gives (the `.md5` file).
4. **The cut** of the extract to the boundary, with `--fixes` when you give it. The command writes `<name>.osm.pbf`.

| Option | Does | Default |
|---|---|---|
| `PLACE` | the name of the area | one of `PLACE`, `--osm-id`, `--bbox`, `--h3-cell`, `--boundary` |
| `--osm-id ID` | one OSM boundary relation (for a name with more than one match) | |
| `--bbox W,S,E,N` | a box in degrees | |
| `--h3-cell CELL` | an H3 cell | |
| `--boundary FILE` | a GeoJSON boundary file | |
| `-o`, `--out DIR` | the output folder | `.` |
| `--name NAME` | the name of the two files | from the place, the file or the cell; `area` for a box |
| `--fixes FILE` | an OSM fixes file: the cut file has the fixes in it | |
| `--cache DIR` | the folder of the downloads | `$DUCKOSM_CACHE`, else `~/.cache/duckosm` |
| `--refresh` | download the extract and the list of extracts again | the files in the cache are used |

The list of extracts stays in the cache for 7 days. An extract in the cache stays until you use `--refresh`.

## `duckosm cut`

Cut a region file that you have to a boundary.

```bash
duckosm cut estonia-latest.osm.pbf --boundary tartu.geojson -o tartu.osm.pbf
duckosm cut estonia-latest.osm.pbf --boundary tartu.geojson -o tartu.osm.pbf --fixes osm_fixes.yaml
```

The region file must be sorted by type, then id, as the files of Geofabrik are. `--boundary` reads GeoJSON, an osmosis
`.poly` file, WKT or a box `W,S,E,N`.

## The fixes in the file

`--fixes` writes the fixes with the rules of [`quack --fixes`](../concepts/quackosm.md#osm-fixes):

| Fix | In the file |
|---|---|
| `remove` | the way is not in the file |
| `keep_nodes` | the way has only the nodes from the first to the second node |
| `tags`, `extra.tag_edits` | the tags of the way, node or relation are changed |
| `oneway`, `lanes`, `lanes_forward`, `lanes_backward`, `layer` | the OSM tags `oneway=yes` or `no`, `lanes`, `lanes:forward`, `lanes:backward`, `layer` |
| `new_ways` | a new way, with the id of the fixes file (9,000,000,000,000 and more), and its new nodes with the id `way id × 1000 + position` |
| `turn_restrictions` | a relation `type=restriction` with the members `from`, `via` and `to`, and the id of the build: -1, -2, ... It is in the file only when its two ways are |
| `exclude_modes`, `turn_lanes_forward`, `turn_lanes_backward` | not in the file: they have no OSM tag |

The new ids are larger than all OSM ids, and the ids of the turn rules are negative, so the file stays sorted.

## Monaco check

`duckosm build` of the cut file without `--fixes` and `duckosm build --fixes` of the original file give the same tables.
The fixes file is `tests/data/osm_fixes.yaml`.

Three fixes files: `osm_fixes.yaml` (the global fixes file of the tests: in Monaco, its `lanes` and `layer` rules act),
`page_fixes/osm_fixes.yaml` (each kind of fix of Edit OSM data, with a new way and a turn rule)
and `osm_fixes_quack.yaml` (`remove`, `keep_nodes`, `tags`, two new ways, tag edits of a node, a way and a relation).
The build writes the 20 GeoParquet tables; each table has the same number of rows in the two builds.

| Table, column | Difference | Cause |
|---|---|---|
| `main.visualization_metadata.built_at` | the time | the time of the build |
| `<mode>.ways.tags` | `oneway`, `lanes`, `lanes:forward`, `lanes:backward`, `layer` | in the cut file, a network rule is an OSM tag. `--fixes` changes only the columns of the mode, not `tags` |
| `<mode>.ways.lanes_bwd` | `3` against empty, for 1 way | the rule `oneway: true, lanes: 3` on a two-way road: `--fixes` gives the lanes to each direction; the tags `oneway=yes, lanes=3` give no lanes backward |
| `<mode>.lane_profile.source` | `override` against `lanes` | the same lane count, from the rule or from the tag |
| `<mode>.edges.geometry` | the last bit of one latitude, for 2 edges | a new node: the PBF format keeps a coordinate in steps of 10⁻⁷ degrees |

All other tables and columns are equal, and each `edge_id` is the same.

## Python

```python
import duckosm

area = duckosm.get("Monaco")                                   # area.pbf, area.boundary: paths
db = duckosm.build(pbf=area.pbf, boundary=area.boundary)
area = duckosm.get(bbox=(7.40, 43.72, 7.44, 43.76), name="monaco_box", fixes="osm_fixes.yaml")
```

See [Python SDK](../reference/python.md#build-and-areas).
