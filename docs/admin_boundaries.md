# Administrative boundaries

The `admin_boundaries` table holds OpenStreetMap administrative areas (countries,
counties, municipalities, districts, …) for the imported region, with a derived
parent link so the hierarchy can be traversed.

It is **not** built by the core import pipeline. Add it to an existing duckOSM
database with:

```bash
python scripts/add_admin_boundaries.py --pbf <file.osm.pbf> --db <file.duckdb>
```

The script extracts `boundary='administrative'` multipolygons from the PBF with
`ogr2ogr` (GDAL assembles the boundary relations' member ways into polygons —
something the bundled DuckDB spatial extension can't do, as it lacks
`ST_Polygonize`), loads them, and computes `parent_osm_id`.

## What `admin_level` means

`admin_level` is a tag on OSM `boundary=administrative` relations: an integer
(roughly 1–11) giving a feature's position in the administrative hierarchy.
**Lower number = larger / higher-level unit**; `2` is the country, and each higher
number is a progressively smaller subdivision.

The **exact meaning of each number is country-specific** — `admin_level=4` is a
*county* in Sweden but a *state* in the USA — because administrative structures
differ between countries. The authoritative mapping is the per-country table in the
OSM wiki: <https://wiki.openstreetmap.org/wiki/Tag:boundary%3Dadministrative>.

### Sweden

Sweden's formal hierarchy is shallow — **Country → 21 Län (counties) → 290 Kommuner
(municipalities)**. Below the municipality there is no general administrative
subdivision; the levels below `7` are a mix of the statistical *distrikt* (created
2016, based on the old parishes/*socknar*) and city-internal *stadsdelar*.

| `admin_level` | Meaning | Swedish term | Notes |
|--:|---|---|---|
| 2 | Country | Land / Riket | "Sverige" |
| 4 | County | Län | 21 of them |
| 6 | Sami reindeer-herding district | Sameby | Special; sits inside a län, not a normal tier |
| 7 | Municipality | Kommun | 290 official |
| 8 | District (former parish) | Distrikt / socken | Partial OSM coverage |
| 9 | District / city borough | Distrikt / Stadsdelsområde | Partial coverage |
| 10 | Neighborhood / locality | Stadsdel / delområde | Partial coverage |
| 11 | Minor locality | Köping / småort | Rare |

Caveats:

- **Levels 1, 3 and 5 are not used** in Sweden.
- **Coverage below kommun (8–11) is incomplete.** Sweden has ~2,500 official
  *distrikt*, but only a fraction are mapped in OSM, so do not treat levels 8–11 as
  exhaustive.
- A few boundaries carry `boundary=administrative` with **no `admin_level`** tag;
  these land with `admin_level = NULL`.

## Table schema

`admin_boundaries`

| Column | Type | Description |
|--------|------|-------------|
| `osm_id` | BIGINT | OSM id of the boundary relation |
| `name` | VARCHAR | Boundary name (may be NULL) |
| `admin_level` | INTEGER | OSM admin level (see above); NULL if untagged |
| `parent_osm_id` | BIGINT | `osm_id` of the immediate enclosing boundary (see below); NULL for the root and a few edge cases |
| `geometry` | GEOMETRY | MultiPolygon, EPSG:4326 (same CRS as the edge geometries) |

## The `parent_osm_id` hierarchy

OSM does not store an explicit parent link, so `parent_osm_id` is derived
**spatially**: a guaranteed-interior point of each boundary (`ST_PointOnSurface`)
is tested against the geometry of candidate parents (those with a lower
`admin_level`), bounding-box pre-filtered for speed. The **immediate parent** is the
containing boundary with the *highest* `admin_level` below the child's (smallest by
area to break ties).

`parent_osm_id` is NULL for:

- the country root (`admin_level = 2`), and
- a small number of boundaries whose interior point isn't covered by any parent —
  typically coastline/border artifacts (e.g. villages on the Sweden–Finland border)
  or unnamed fragments.

On the full Sweden extract this links **1,456 of 1,472** boundaries, e.g.
21 counties → Sweden, 289 kommuner → county, districts → kommun.

## Example queries

```sql
-- All counties (län)
SELECT name FROM admin_boundaries WHERE admin_level = 4 ORDER BY name;

-- Each boundary with its immediate parent
SELECT c.name, c.admin_level, p.name AS parent, p.admin_level AS parent_level
FROM admin_boundaries c
LEFT JOIN admin_boundaries p ON c.parent_osm_id = p.osm_id;

-- Full ancestry chain of one area (kommun -> county -> country)
WITH RECURSIVE up AS (
    SELECT osm_id, name, admin_level, parent_osm_id
    FROM admin_boundaries WHERE name = 'Lidingö kommun'
    UNION ALL
    SELECT b.osm_id, b.name, b.admin_level, b.parent_osm_id
    FROM admin_boundaries b JOIN up ON b.osm_id = up.parent_osm_id
)
SELECT admin_level, name FROM up ORDER BY admin_level DESC;

-- Which county contains a point (lon, lat)?
SELECT name FROM admin_boundaries
WHERE admin_level = 4 AND ST_Contains(geometry, ST_Point(18.07, 59.33));

-- Tag every driving edge with the kommun it lies in (interior point match)
SELECT e.edge_id, k.name AS kommun
FROM driving.edges e
JOIN admin_boundaries k
  ON k.admin_level = 7
 AND ST_Contains(k.geometry, ST_PointOnSurface(e.geometry));
```
