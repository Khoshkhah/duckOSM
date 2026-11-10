# Add elevation

`duckosm elevation` adds the ground height to every node of a finished build, in place. Needs
`pip install "duckosm[elevation]"`.

```bash
duckosm elevation monaco.duckdb
```

```text
  source=auto → copernicus (bbox 7.41, 43.73, 7.44, 43.75)
  [cycling] 4151 nodes sampled (0 nodata/fill) -> ele
  [driving] 1719 nodes sampled (0 nodata/fill) -> ele
  [walking] 4116 nodes sampled (0 nodata/fill) -> ele
elevation added from Copernicus GLO-30 — 9986 nodes (0 nodata/fill) across 3 mode(s) -> ele
```

It adds:

- `nodes.ele`: the height at each node, in metres;
- `edges.z_from`, `edges.z_to`: the height at each edge's two ends;
- a row in `main.elevation_metadata` saying where the heights came from (source, resolution,
  vertical datum, licence), and the SHA-256 of the elevation file, the ground id.

The [MATSim](../exports/matsim.md), [OpenDRIVE](../exports/opendrive.md), [GIS](../exports/gis.md) and
[GMNS](../exports/gmns_tables.md) exports then include the heights (GMNS: `node.z_coord`, `link.grade`).

## Where the heights come from

With no options it picks a free global elevation model for your area and streams just the part it
needs: **Copernicus GLO-30** (30 m, worldwide, no account). Inside Europe it uses **EU-DTM** instead
when you set an [OpenTopography](https://opentopography.org) key in `OPENTOPOGRAPHY_API_KEY`. For
better accuracy, point it at your own file, e.g. a national 1 m model:

```bash
duckosm elevation monaco.duckdb --dem my_dtm.tif --product DTM --vertical-datum EGM2008 --license CC-BY-4.0
```

A `--dem` file in `ele` is the area's ground reference. Its product (`DTM` or `DSM`), its vertical datum and its license
are necessary: without them the command stops. For a file that is not the reference (a test, a surface model), add `--not-reference`. A file with
`--suffix` is never the reference.

| Option | Does | Default |
|---|---|---|
| `--dem FILE` | use this elevation file (GeoTIFF, VRT, … any format GDAL reads) | |
| `--source` | `auto`, `copernicus`, or `eudtm` (needs the key) | `auto` |
| `-m MODE` | only this mode; repeat for several | every mode |
| `--nodata-fill` | height written where the model has no value | `0` |
| `--suffix dsm` | store as a second surface (`ele_dsm`, `z_from_dsm`, …) instead of replacing | |
| `--product` | `DTM` (bare earth) or `DSM` (surface): the product of the `--dem` file | |
| `--vertical-datum` | the vertical datum of the `--dem` file, for example `EGM2008` | |
| `--license` | the license of the `--dem` file | |
| `--not-reference` | the `--dem` file is not the area's ground reference | |

The Rust build (`duckosm-rust --dem FILE`) has the same rule, with the options `--dem-product`, `--dem-vertical-datum`,
`--dem-license` and `--dem-not-reference`. To set the DEM one time for each area, put it in the config file, in the section
[`elevation`](../reference/configuration.md#elevation) (`dem`, `product`, `vertical_datum`, `license`); `--dem` wins over it. The browser build gives them as `Params.dem_product`,
`dem_vertical_datum`, `dem_license` and `dem_not_reference`.

### The ground id

`main.elevation_metadata.sha256` is the SHA-256 of the elevation file's bytes. It is the same value that `sha256sum`
gives, so you can check a database against its file:

```bash
sha256sum my_dtm.tif
duckdb monaco.duckdb -c "SELECT sha256 FROM main.elevation_metadata WHERE ele_column = 'ele'"
```

The value is NULL when the step read no whole file: Copernicus GLO-30 is streamed in parts, and a `--dem` URL is read in
parts. To get a ground id, give the reference as a local file.

Where to get elevation data, and how the sources compare: [Elevation sources](../reference/elevation-sources.md).

To find the sources that cover an area, best first, ask the catalog:

```python
from duckosm.dem_sources import sources_for_bbox

for s in sources_for_bbox((7.41, 43.73, 7.44, 43.75)):      # Monaco: west, south, east, north
    print(s["name"], s["resolution_text"], s["product"], s["license"])
```

The order is a national 1 m DTM, EU-DTM, FABDEM, Copernicus GLO-30 and then the others. Each entry has the `product`, `vertical_datum` and `license`
that `--product`, `--vertical-datum` and `--license` need. In duckOSM Studio the step "Ground" does the same for the area on the map.

For an area that SimReady, urbanstyle or roadstyle also use, the heights must follow [The ground reference](../concepts/ground-reference.md):
one bare-earth model (DTM) with a known vertical datum, recorded once and used by every project.

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
| Boulevard du Jardin Exotique | 117.0 | 97.0 | 123.0 | -16.5 |
| Boulevard du Jardin Exotique | 97.0 | 117.0 | 123.0 | 16.5 |
| Avenue de La Quarantaine | 32.0 | 8.0 | 144.0 | -16.1 |

With Copernicus, the tunnels of Monaco would top this list: Tunnel Albert II "falls" from 336 m to
32 m, the height of the hill above it.
