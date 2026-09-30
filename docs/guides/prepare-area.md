# Prepare an area

Before a build you need two things: the map data (a `.osm.pbf` file) and, for a city or district,
its boundary. [Your first network](../first_network.md) walks through this once for Monaco; this page
has the details.

## Get the map data

[Geofabrik](https://download.geofabrik.de) offers free `.osm.pbf` extracts of every continent,
country and many regions, updated daily. Download the **smallest region that contains your area**:

```bash
curl -LO https://download.geofabrik.de/europe/monaco-latest.osm.pbf     # 0.7 MB
curl -LO https://download.geofabrik.de/europe/sweden-latest.osm.pbf     # 835 MB
```

Any `.osm.pbf` works, up to the [whole planet](https://planet.openstreetmap.org).

## Find the area's id

Search for the area on [openstreetmap.org](https://www.openstreetmap.org) and click the result whose
border is drawn on the map. The page address ends in `relation/<id>`: that number is the **OSM id**.
Seeing the outline is the surest way to pick the right area: there is a Södermalm in Stockholm and
one in Sundsvall, and a coastal country's border can reach far out to sea. Monaco's country border
([relation/1124039](https://www.openstreetmap.org/relation/1124039)) includes its territorial waters;
its land area is [relation/2220322](https://www.openstreetmap.org/relation/2220322).

## Make the boundary file

```bash
duckosm boundary --osm-id 2220322 --pbf monaco-latest.osm.pbf       # -> monaco.geojson
```

```text
wrote monaco.geojson: Monaco (admin level 8, OSM relation 2220322, 2.39 km²), from PBF monaco-latest.osm.pbf
```

With `--pbf`, the border is read from the PBF itself (offline; needs [GDAL](../install.md#tools-outside-python)).
Without it, or if GDAL is missing, it's fetched from OpenStreetMap online.

**By name instead of id.** `duckosm boundary Monaco --pbf monaco-latest.osm.pbf` searches by name:
case, accents and hyphens don't matter, and English names (`name:en`) match too. If the name matches
more than one border, it takes the exact name with the largest area and lists the others with their
ids, so you can rerun with the right `--osm-id`. For "Monaco" it takes the country (with its sea) and
lists the land area:

```text
also matched (pick one with --osm-id):
  Monaco (admin level 8, OSM relation 2220322, 2.39 km²)
  Monaco-Ville (admin level 10, OSM relation 2220207, 0.2 km²)
```

| Option | Does |
|---|---|
| `--osm-id ID` | take this exact OSM relation |
| `--pbf FILE` | read the border from this PBF (offline) |
| `--offline` | never go online |
| `-o FILE` | output file (default `<name>.geojson`) |

Any GeoJSON polygon you already have works as a boundary too.

## Several areas from one region

To build many areas of one country, build the country once and cut the areas out of it. Cutting
takes seconds, re-reads no OSM, and every `edge_id` stays the same as in the parent, so data keyed on
the parent's ids works on the areas as they are:

```bash
duckosm build --pbf sweden-latest.osm.pbf -m driving          # slow, once -> sweden-latest.duckdb
duckosm boundary --osm-id 2017432 -o sodermalm.geojson        # Södermalm, Stockholm
duckosm extract --source sweden-latest.duckdb --db sodermalm.duckdb --boundary sodermalm.geojson
```

`extract` keeps edges that cross the border whole. It can also find the area by `--name` or `--osm-id`
if the parent has [administrative boundaries](admin-boundaries.md). The same cut can run as a build
from a config file with `source.type: duckdb`; see [Configuration](../configuration.md).

## Cut a smaller PBF

To get just the area's map data as a PBF, for example for another tool:

```bash
duckosm clip-pbf sweden-latest.osm.pbf sodermalm.geojson      # -> sodermalm.osm.pbf
```

It needs [osmium](../install.md#tools-outside-python). By default, ways that cross the border are kept
whole (`--strategy complete_ways`); `--strategy smart` also keeps rivers and other areas whole, which
the [base-map layers](../features_schema.md) need.
