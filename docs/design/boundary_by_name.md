# Area tools: find a boundary, cut a PBF

**Status:** implemented 2026-09-30 (`src/duckosm/area.py`, `duckosm boundary`, `duckosm clip-pbf`).

## Goal

Two commands that prepare an area before a build. `duckosm build` itself does not change.

```bash
duckosm boundary "Monaco" --pbf monaco-latest.osm.pbf      # -> monaco.geojson
duckosm clip-pbf monaco-latest.osm.pbf monaco.geojson       # -> monaco.osm.pbf
duckosm build --pbf monaco.osm.pbf --boundary monaco.geojson
```

## `duckosm boundary NAME` — find an area's boundary

Writes the boundary of the named area as a GeoJSON polygon. Where it looks, first hit wins:

1. **In a PBF (offline)**, with `--pbf`: the official borders OSM stores as boundary relations
   (countries, cities, districts). Reuses `admin.extract_boundaries` (GDAL's `ogr2ogr`) and the
   name matcher `extract --name` already uses: accents ignored, `name:en` also matched, an exact
   name before a partial one, then the largest area.
2. **Nominatim (online)**, when there is no `--pbf`, no `ogr2ogr`, or no match: one request to
   OpenStreetMap's search service (the idea of `scripts/download_boundary.py`, without osmnx).
   Only polygon results count.
3. Nothing found → an error that says what was searched.

Options: `--osm-id` picks one exact relation when a name is ambiguous (the Monaco PBF has two
"Monaco" borders: the country, level 2, 79.8 km² incl. sea, and the municipality, level 8,
2.39 km²); `-o` sets the output file (default `<name>.geojson` in the current folder);
`--offline` never goes online. It prints what it found: name, admin level, OSM id, area, source.

## `duckosm clip-pbf PBF BOUNDARY` — cut a PBF to an area

Writes a smaller PBF containing only the area, with osmium, using the same strategy the build's
own pre-clip uses (`complete_ways`: roads crossing the border are kept whole, so no junction is
lost). Output default: `<boundary name>.osm.pbf`. The build's pre-clip and this command share one
function.

## Clean-up

`scripts/download_boundary.py` and `scripts/filter_pbf.py` become these commands and are deleted.
The Monaco sample gets a boundary made with `duckosm boundary`, so its build runs the fragment
clean-up (today its walking network is 107 pieces).
