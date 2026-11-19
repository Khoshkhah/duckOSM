# The terrain of an area

duckOSM keeps the ground of an area and the heights of its roads in the database of the area, from one elevation file
(a DEM). The maps of duckOSM and roadstyle draw this terrain. The rules: [The ground reference](ground-reference.md).

![Monaco: the ground heights from FABDEM, with the area boundary](../images/terrain-monaco.png)

*Monaco from FABDEM (bare earth, about 30 m). The black line is the area boundary.*

## What the database holds

| Table or column | Holds |
|---|---|
| [`main.elevation_grid`](../reference/database.md#elevation_grid) | the terrain: one row for each pixel of the DEM over the area (`lon`, `lat`, `z`) |
| `nodes.ele` | the ground height at each node |
| `edges.z_from`, `edges.z_to`, `edges.z_vertices` | the height at the two ends and at each point of each edge |
| [`main.elevation_metadata`](../reference/database.md#elevation_metadata) | the source, the product (`DTM` or `DSM`), the vertical datum, the license, and the ground id |

## Give the DEM

Give the DEM one time for each area, in the build:

```bash
duckosm monaco.osm.pbf --boundary monaco.geojson -o monaco \
  --dem fabdem-monaco.tif --dem-product DTM --dem-vertical-datum EGM2008 --dem-license CC-BY-NC-SA-4.0
```

Or in the config file, [`elevation`](../reference/configuration.md#elevation): `dem`, `product`, `vertical_datum`,
`license`. Without the product, the vertical datum and the license, the build stops.

| Choose | Why |
|---|---|
| a bare-earth model (DTM), for example FABDEM or a national LiDAR model | a surface model (DSM, for example Copernicus) has roofs and trees in its heights |
| a known vertical datum, for example EGM2008 | each height is measured on it |
| a GeoTIFF in EPSG:4326 | the format that the build reads |

Where to get a DEM: [Elevation sources](../reference/elevation-sources.md). To add heights to a built database:
[Add elevation](../guides/elevation.md).

## What the terrain does not show

| Not shown | Why |
|---|---|
| tunnels, bridges, and two roads side by side at different heights | their heights come from the road, not from the grid ([where the ground cannot hold a road](ground-reference.md#where-the-ground-cannot-hold-a-road)) |
| a step or a curb | a 30 m DEM has no such detail |

When an area has no DEM, the maps use the AWS Terrain Tiles, and they say "Ground: AWS Terrain Tiles".
