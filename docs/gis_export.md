# GIS export — GeoPackage / shapefile (`duckosm export-gis`)

Write a built duckOSM network to the **universal GIS interchange formats** — GeoPackage (primary)
or ESRI shapefile (fallback) — so a QGIS / ArcGIS / FME user, or any tool with a GIS importer
(Aimsun, PTV Visum, …), can consume the network without ever touching DuckDB. This is the
*widest-reach* export: unlike the NetworkX / SUMO / GMNS exports (aimed at programmers and
modellers), it hands the network to the entire GIS world, including non-coders.

Like every duckOSM exporter it **preserves `edge_id`** — the stable content-hash id — as a plain
attribute, so measured data, sensor readings and flows keyed to `edge_id` drape straight back onto
the GIS layer after a round-trip.

**Elevation:** the export keeps every scalar column, so if the db was enriched by
[`duckosm elevation`](design/elevation.md) the `ele` (nodes) and `z_from`/`z_to` (edges) columns
come through automatically as attributes — no flag, no code path of their own. Geometry stays 2D
(edges only carry endpoint z, so a 3D `LineStringZ` would be degenerate); symbolise or analyse by
the `ele` attribute in QGIS/ArcGIS instead.

```bash
duckosm export-gis data/db/sodermalm.duckdb                    # -> sodermalm.gpkg (all modes + boundary)
duckosm export-gis data/db/sodermalm.duckdb -m driving         # only the driving schema
duckosm export-gis data/db/sodermalm.duckdb -o out/net.gpkg    # explicit output file
duckosm export-gis data/db/sodermalm.duckdb --format shp -o gis/   # shapefile set into gis/
```

```python
import duckdb
from duckosm import to_gis
con = duckdb.connect("data/db/sodermalm.duckdb", read_only=True)
to_gis(con, "sodermalm.gpkg")                    # GeoPackage, every mode present + boundary
```

## What gets exported — geometry only, not routing adjacency

duckOSM builds a schema per mode (`driving` / `walking` / `cycling`), each holding `edges`, `nodes`,
`edge_graph` and `turn_restrictions`. The GIS export writes only the **geographic** tables:

| Source table        | Exported | Why |
|---------------------|----------|-----|
| `<mode>.edges`      | ✅ layer `edges_<mode>` | the road segments — the thing a GIS user wants |
| `<mode>.nodes`      | ✅ layer `nodes_<mode>` | junctions, for snapping / display |
| `main.boundary`     | ✅ layer `boundary` (if present) | the clip/area outline |
| `<mode>.edge_graph` | ❌ | routing adjacency — belongs in the NetworkX / SUMO exports, not a GIS layer |
| `<mode>.turn_restrictions` | ❌ | ditto (turn logic, not geometry) |

A GIS layer is a flat feature table; the line-graph and turn restrictions are a *topology* that GIS
tools can't act on. Consumers who need routable topology use `export-graph` (NetworkX) or `sumo`.

## Layer / file structure — one layer per mode

The mode networks are **different edge sets, attributes and topology** (a `footway` is not a
`motorway`; cycling has contraflow edges driving doesn't). They do **not** align 1:1, so they are
kept as separate layers rather than merged into one mode-flagged table (a merge would be lossy and
display-only).

- **GeoPackage** — one `<name>.gpkg` with layers `edges_<mode>`, `nodes_<mode>` (per selected mode)
  and `boundary`. The user toggles modes as layers in QGIS. This mirrors the DB exactly.
- **Shapefile** — a shapefile holds a single layer of a single geometry type, so it is forced to
  one fileset per layer: `<name>_edges_<mode>.shp`, `<name>_nodes_<mode>.shp`, `<name>_boundary.shp`
  (each with its sidecar `.dbf` / `.shx` / `.prj`). This is a core reason **GeoPackage is primary**;
  shapefile also has the 10-character field-name limit and a 2 GB size cap. Use `--format shp` only
  for tools that still demand it.

## Attribute schema

Every non-geometry, non-list column of the source table is carried through, so the exact set follows
your build (H3 columns, `bridge`/`tunnel`/`layer`, etc. appear when present). The core edge
attributes are:

| Attribute | Type | Notes |
|-----------|------|-------|
| `edge_id` | int64 | **stable content-hash id** — the cross-format join key |
| `source`, `target` | int64 | endpoint node ids (match `nodes.node_id`) |
| `osm_id` | int64 | original OSM way id |
| `highway` | str | road class |
| `name` | str | road name |
| `oneway` | bool | one-way flag (never null) |
| `lanes` | int | lane count *in this edge's direction* (never null) |
| `length_m` | float | segment length, metres |
| `maxspeed_kmh` | float | normalized speed |
| `cost_s` | float | free-flow travel time, seconds |
| `is_reverse` | bool | reverse direction of a two-way edge |
| `geometry` | LineString | EPSG:4326 |

- **`refs` (the shape-point node-id list) is dropped** — it is a `BIGINT[]`, which the shapefile
  format can't hold, and it's routing-internal rather than GIS-relevant. Any other list / nested
  column is dropped the same way, keeping the GeoPackage and shapefile schemas identical.
- **CRS is written explicitly** as `EPSG:4326` (a `.prj` for shapefiles, the SRS entry for
  GeoPackage), so the layer lands in the right place with no manual "set CRS" step.
- **Shapefile field-name truncation:** names over 10 chars are shortened — `maxspeed_kmh → spd_kmh`,
  `is_reverse → is_rev`, `admin_level → adm_level`. GeoPackage keeps the full names.
- **`edge_id` in shapefiles is written as text.** `edge_id` (and the other 64-bit ids `source`,
  `target`, `osm_id`, `node_id`) is a 19-digit content hash, and a DBF numeric field cannot hold an
  int64 exactly — GDAL would store it as a float and *silently corrupt* it (a float64 keeps only
  ~15 significant digits). The exporter casts those columns to `VARCHAR` for shapefiles so the ids
  survive **exactly**; read them back as strings. **GeoPackage stores them as native Integer64** — no
  cast, no caveat. This is another reason GeoPackage is the primary format.

## How it's built (implementation)

Each layer is produced by a single DuckDB spatial `COPY`:

```sql
COPY (SELECT edge_id, …, geometry FROM driving.edges)
  TO '<layer>' (FORMAT gdal, DRIVER 'GPKG', SRS 'EPSG:4326');
```

- **GeoPackage** is multi-layer, but DuckDB's `COPY` writes one layer per file (named after the
  file) and overwrites on re-copy. So the exporter writes each layer to a temp single-layer
  `.gpkg`, then assembles them into the one target file with **`ogr2ogr … -update -append -nln
  <layer>`** — the same GDAL dependency the `duckosm admin --gpkg` export already relies on. If
  `ogr2ogr` is not on `PATH`, it falls back to one GeoPackage file per layer (`<name>_edges_<mode>.gpkg`)
  and logs a warning.
- **Shapefile** needs no assembly — each layer is a direct `COPY … DRIVER 'ESRI Shapefile'`.

The exporter is a self-contained module (`src/duckosm/gis.py`, `to_gis`) reading the same `edges` /
`nodes` tables as every other exporter — see [user_manual.md](user_manual.md) for the other exporters.

## Debugging an export (`duckosm gis-debug`)

To verify an export actually round-tripped, `duckosm gis-debug` reads the file **back through GDAL**
(the same path QGIS / ArcGIS / FME take) and writes a single self-contained HTML page — so it checks
the file on disk, not the duckOSM database:

```bash
duckosm gis-debug sodermalm.gpkg --source-db data/db/sodermalm.duckdb
duckosm gis-debug gis/ --source-db data/db/sodermalm.duckdb   # a shapefile directory
```

The page has a canvas map of every layer (edges by highway class, nodes, boundary; per-mode toggles,
pan/zoom, no external tiles) and a QA panel: per-layer feature counts, geometry type, CRS, and
`edge_id` integrity (a **float** dtype means a shapefile DBF silently rounded the 64-bit id). With
`--source-db` it also runs a **round-trip diff** — the exported `edge_id` set vs the db's, per mode —
the definitive "did every edge survive intact" check. Verdict is `PASS` / `CHECK` / `FAIL`. Needs
`geopandas` + `pyogrio` (`duckdb` only for `--source-db`); it's the module `src/duckosm/gis_debug.py`.

## `to_gis` reference

```python
to_gis(con, out, modes=None, fmt="gpkg", boundary=True, name=None,
       srs="EPSG:4326", ogr2ogr_bin=None) -> dict
```

| Param | Meaning |
|-------|---------|
| `con` | DuckDB connection to a built db (spatial extension loaded here) |
| `out` | GeoPackage: target `.gpkg` file. Shapefile: target **directory**. |
| `modes` | list of mode schemas to export (default: every mode present) |
| `fmt` | `"gpkg"` (default) or `"shp"` |
| `boundary` | also export `main.boundary` as a `boundary` layer if it exists (default True) |
| `name` | basename for layers / shapefiles (default: the db filename stem) |
| `srs` | CRS written into the output (default `EPSG:4326`) |
| `ogr2ogr_bin` | explicit `ogr2ogr` path (else auto-located; GeoPackage assembly only) |

Returns `{"fmt", "path"|"dir", "layers": {name: feature_count}, "files": [...]}`.
