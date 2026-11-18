# Add administrative boundaries

`duckosm admin` adds all administrative areas of a region (country, counties, municipalities, districts) to a built
database, as the table `main.admin_boundaries`.

```bash
duckosm admin --pbf monaco-latest.osm.pbf --db monaco.duckdb
```

| Option | Does |
|---|---|
| `--pbf FILE` | the PBF with the borders (required) |
| `--db FILE` | the database (required) |
| `--gpkg FILE` | keep the borders in this GeoPackage, and use it again in the next run |

Use the table to find the district of a road, or to cut an area by name with
[`extract --name`](prepare-area.md#2-cut-a-built-region). For the boundary file of one area, use
[`duckosm boundary`](prepare-area.md#make-the-boundary-file).

## The table

| Column | Type | Holds |
|---|---|---|
| `osm_id` | BIGINT | the OSM id of the boundary relation |
| `name` | VARCHAR | the name in the local language; can be NULL |
| `name_en` | VARCHAR | the English name (`name:en`); NULL when OSM has none |
| `admin_level` | INTEGER | the OSM admin level; NULL when the relation has none |
| `parent_osm_id` | BIGINT | the smallest boundary that contains this one; NULL for the country |
| `geometry` | GEOMETRY | a MultiPolygon in EPSG:4326 |

`admin_level` 2 is the country, and a higher number is a smaller area. Each country uses the numbers in its own way:
4 is a county in Sweden and a state in the USA. The table of each country is in the
[OSM wiki](https://wiki.openstreetmap.org/wiki/Tag:boundary%3Dadministrative).

| Sweden | `admin_level` |
|---|---|
| Country (Sverige) | 2 |
| County (län), 21 | 4 |
| Sami reindeer-herding district (sameby) | 6 |
| Municipality (kommun), 290 | 7 |
| District, borough, neighborhood (distrikt, stadsdel); not complete in OSM | 8 to 11 |

The parent is found from the geometry: the boundary with a lower `admin_level` that contains a point inside the child.
A border artifact can have no parent. On Sweden, 1,456 of 1,472 boundaries have a parent.

## Example queries

```sql
-- each district with its parent
SELECT c.name, p.name AS parent
FROM admin_boundaries c LEFT JOIN admin_boundaries p ON c.parent_osm_id = p.osm_id
WHERE c.admin_level = 10;

-- the district that contains a point (lon, lat)
SELECT name FROM admin_boundaries
WHERE admin_level = 10 AND ST_Contains(geometry, ST_Point(7.4270, 43.7397));

-- driving edges in each district
SELECT d.name AS district, count(*) AS edges
FROM driving.edges e
JOIN admin_boundaries d ON d.admin_level = 10 AND ST_Contains(d.geometry, ST_PointOnSurface(e.geometry))
GROUP BY 1 ORDER BY edges DESC;
```
