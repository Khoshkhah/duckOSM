# Add elevation

`duckosm elevation` adds the ground height to every node of a finished build, in place. Needs
`pip install "duckosm[elevation]"`.

```bash
duckosm elevation monaco.duckdb
```

```text
  source=auto → copernicus (bbox 7.41, 43.73, 7.44, 43.75)
  [cycling] 3566 nodes sampled (0 nodata/fill) -> ele
  [driving] 1160 nodes sampled (0 nodata/fill) -> ele
  [walking] 3491 nodes sampled (0 nodata/fill) -> ele
elevation added from Copernicus GLO-30 — 8217 nodes (0 nodata/fill) across 3 mode(s) -> ele
```

It adds:

- `nodes.ele`: the height at each node, in metres;
- `edges.z_from`, `edges.z_to`: the height at each edge's two ends;
- a row in `main.elevation_metadata` saying where the heights came from (source, resolution,
  vertical datum, licence).

The [MATSim](../exports/matsim.md), [OpenDRIVE](../exports/opendrive.md) and [GIS](../exports/gis.md)
exports then include the heights.

## Where the heights come from

With no options it picks a free global elevation model for your area and streams just the part it
needs: **Copernicus GLO-30** (30 m, worldwide, no account). Inside Europe it uses **EU-DTM** instead
when you set an [OpenTopography](https://opentopography.org) key in `OPENTOPOGRAPHY_API_KEY`. For
better accuracy, point it at your own file, e.g. a national 1 m model:

```bash
duckosm elevation monaco.duckdb --dem my_dtm.tif          # any GDAL raster
```

| Option | Does | Default |
|---|---|---|
| `--dem FILE` | use this elevation file (GeoTIFF, VRT, … any format GDAL reads) | |
| `--source` | `auto`, `copernicus`, or `eudtm` (needs the key) | `auto` |
| `-m MODE` | only this mode; repeat for several | every mode |
| `--nodata-fill` | height written where the model has no value | `0` |
| `--suffix dsm` | store as a second surface (`ele_dsm`, `z_from_dsm`, …) instead of replacing | |

Where to get elevation data, and how the sources compare: [Elevation sources](../elevation.md).

## What the heights mean

The height is the **surface at the node's position**, not the road itself:

- A **tunnel** gets the height of the ground above it, and a **bridge** the ground below.
- **Copernicus is a surface model**: it includes buildings and trees, at 30 m resolution. In a
  dense city, short streets between buildings can look steep. A bare-earth model (EU-DTM, or a
  national one) gives better road grades.

So leave tunnels and bridges out when you look at gradients:

```sql
SELECT name, round(z_from) AS z_from, round(z_to) AS z_to, round(length_m) AS length_m,
       round(100 * (z_to - z_from) / length_m, 1) AS grade_pct
FROM driving.edges
WHERE length_m > 100 AND name IS NOT NULL AND tunnel IS NULL AND bridge IS NULL
ORDER BY abs(z_to - z_from) / length_m DESC LIMIT 5;
```

| name | z_from | z_to | length_m | grade_pct |
|---|---:|---:|---:|---:|
| Boulevard de Belgique | 86.0 | 111.0 | 110.0 | 22.5 |
| Boulevard de Belgique | 111.0 | 86.0 | 110.0 | -22.5 |
| Rue Louis Auréglia | 58.0 | 36.0 | 164.0 | -12.9 |
| Boulevard du Larvotto | 36.0 | 51.0 | 116.0 | 12.8 |
| Avenue de Monte-Carlo | 36.0 | 50.0 | 106.0 | 12.7 |

With Copernicus, the tunnels of Monaco would top this list: Tunnel Albert II "falls" from 336 m to
32 m, the height of the hill above it.
