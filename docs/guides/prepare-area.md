# Prepare an area

A build needs the map data (a `.osm.pbf` file) and, for a city or a district, its boundary (a GeoJSON file).
[Your first network](../first_network.md) shows how to find the OSM id of an area and download its region file.

## Make the boundary file

```bash
duckosm boundary --osm-id 2220322 --pbf monaco-latest.osm.pbf
```

```text
wrote monaco.geojson: Monaco (admin level 8, OSM relation 2220322, 2.39 km²), from PBF monaco-latest.osm.pbf
```

`duckosm boundary Monaco --pbf monaco-latest.osm.pbf` finds the area by name. Case, accents and hyphens do not
matter, and English names (`name:en`) also match. When two or more borders match, the command takes the exact name with
the largest area and lists the other borders with their ids.

| Option | Does | Default |
|---|---|---|
| `NAME` | find the border by this name | |
| `--osm-id ID` | take this OSM relation | |
| `--pbf FILE` | find the border in this PBF first; then find it from Nominatim | from Nominatim only |
| `--offline` | do not use Nominatim | |
| `-o FILE` | output file | `<name>.geojson` |

Any GeoJSON polygon can be a boundary. A [config file](build.md#the-config-file) can also give the area as
`boundary.place`, `boundary.bbox` or `boundary.h3_cell`: the build then writes `<name>.boundary.geojson`.

## Cut a region file to the area

```bash
duckosm cut sweden-latest.osm.pbf --boundary sodermalm.geojson -o sodermalm.osm.pbf
```

| Command | Does | Needs |
|---|---|---|
| `duckosm cut REGION --boundary FILE -o OUT` | the smart cut of osmium: ways that cross the border stay whole, and relations (rivers, land cover) are complete | a PBF sorted by type and id (the Geofabrik files are) |
| `duckosm clip-pbf PBF BOUNDARY` | the cut of osmium, with `--strategy complete_ways` (default), `smart` or `simple`; not ported to Rust yet | [osmium](../install.md#outside-tools) and a PBF sorted by type and id |

## Several areas from one region

Use this when you need many areas of one region, for example the districts of Stockholm. Build the region once. Then
`duckosm extract` copies the part of each area from the built database. It does not read the `.osm.pbf` again, so each
area takes seconds.

```bash
# 1. Build the region once (slow)
duckosm build --pbf sweden-latest.osm.pbf -m driving

# 2. Get the border of one area
duckosm boundary --osm-id 2017432 -o sodermalm.geojson

# 3. Cut the area from the region (seconds)
duckosm extract --source sweden-latest.duckdb \
    --db sodermalm.duckdb --boundary sodermalm.geojson
```

Each road keeps its `edge_id` from the region. So data that you joined to the roads of the region, such as traffic counts,
also fits the area.

| Option of `extract` | Does |
|---|---|
| `--source FILE` | the built region (required) |
| `--db FILE` | the output database, overwritten (required) |
| `--boundary FILE` | the area as a GeoJSON file |
| `--name NAME`, `--osm-id ID` | the area from the [administrative boundaries](admin-boundaries.md) of the region |

An edge that crosses the border stays whole. `duckosm build --source-db` and `source.type: duckdb` in a config file
do the same cut.
