# Add elevation

`duckosm elevation` adds the ground height to every node of a built database, in place.

```bash
duckosm elevation monaco.duckdb
```

| It adds | Holds |
|---|---|
| `<mode>.nodes.ele` | the height of the node, in meters |
| `<mode>.edges.z_from`, `z_to`, `z_vertices` | the heights of the edge ends and of each vertex |
| a row in `main.elevation_metadata` | the source, its product, vertical datum and license, and the ground id |

The [MATSim](../exports/matsim.md), [OpenDRIVE](../exports/opendrive.md), [GIS](../exports/gis.md) and
[GMNS](../exports/gmns_tables.md) exports then include the heights. All options: [Command line](../reference/cli.md#elevation).

## Where the heights come from

| Source | Model | Reference ground | License |
|---|---|---|---|
| `--source copernicus` | Copernicus GLO-30: 30 m, the whole world, a surface model (DSM) | no: it includes buildings and trees | Copernicus open, with attribution |
| `--source eudtm` | EU-DTM: 30 m, Europe, bare earth (DTM); needs a free key in `OPENTOPOGRAPHY_API_KEY` | yes | the terms of OpenTopography |
| `--source auto` (default) | `eudtm` when the area is in Europe and the key is set, else `copernicus` | | |
| `--dem FILE` | your own file, for example a national 1 m DTM | yes, with `--product DTM` | the license that you give |

The other models and their licenses: [Elevation sources](../reference/elevation-sources.md).

## Use your own file

Each area has one ground: one bare-earth model (DTM) with a known vertical datum ([The ground reference](../concepts/ground-reference.md)).

```bash
duckosm elevation monaco.duckdb --dem my_dtm.tif --product DTM --vertical-datum EGM2008 --license CC-BY-4.0
```

`--dem` takes a GeoTIFF in EPSG:4326 with 32-bit float values, as a file or an HTTP address. To convert another format,
use GDAL ([File formats](../reference/elevation-sources.md#file-formats)). The file is the ground reference of the area, so
`--product`, `--vertical-datum` and `--license` are necessary. For a test file or a surface model, give `--not-reference`.

`main.elevation_metadata.sha256` is the ground id: the SHA-256 of the file, the same value as `sha256sum my_dtm.tif`.
It is NULL when the command reads the file in parts (Copernicus, or a `--dem` address).

| Where | How |
|---|---|
| the program | `duckosm elevation DB` |
| duckOSM Studio (browser) | the step **Ground height (DEM)**: it lists the sources for the area and links to their download pages; you drop the file |
| `duckosm studio` (your computer) | the same step; it also downloads the tiles itself |

## What the heights mean

The height is the ground at the position of the node, not the road. A tunnel gets the height of the ground above it,
and a bridge the height of the ground below it. A surface model (Copernicus) also includes buildings and trees, so
short streets in a city can look steep. Thus, leave out tunnels and bridges when you calculate grades:

```sql
SELECT name, round(100 * (z_to - z_from) / length_m, 1) AS grade_pct
FROM driving.edges
WHERE length_m > 100 AND tunnel IS NULL AND bridge IS NULL
ORDER BY abs(z_to - z_from) / length_m DESC LIMIT 5;
```

France, driving, `--source copernicus`, on a Mac mini (Apple M6, 12 cores, 16 GB): 12.3 million nodes in 6 min 53 s with the
download, 7.3 GB peak memory.
