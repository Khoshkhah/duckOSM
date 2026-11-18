# The ground reference

<p class="lead">One area has one ground. Every project computes on it, every renderer draws it, and every file names the
ground that it was made on.</p>

This standard applies to duckOSM, SimReady, roadstyle, urbanstyle and the projects that read their outputs. Without it, a
road, a bridge deck or a vehicle floats above the drawn terrain or goes into it. History and the state of each project:
[ground reference in detail](https://github.com/Khoshkhah/duckOSM/blob/main/docs/design/ground_reference.md).

## 1. One ground reference for each area

The ground reference of an area is one elevation file. duckOSM records it in `main.elevation_metadata`, in the row of `ele`.

| Field | Rule |
|---|---|
| `product` | `DTM` (bare earth). A surface model (`DSM`, for example Copernicus GLO-30) is not a reference: its roofs and trees go into the road heights |
| `source`, `uri` | the file or the model, with its version (`FABDEM V1-2`, a national DTM, a LiDAR DTM) |
| `dem_crs` | its CRS |
| `vertical_datum` | known (`EGM2008`, `EGM96`, a national datum). `unknown` is not a reference |
| `resolution_m`, `license` | known |
| `sha256` | the hash of the file's bytes: the **ground id** of every product in section 4. NULL when the build read no whole file (Copernicus tiles from AWS, a URL). Such a source is not a reference |

The other projects do not choose a ground. They read this row. SimReady reads the same values from `areas/<area>/area.yaml`
(`dem:`) and `sources.yaml`, which must name the same file.

If no DTM exists for an area, a DSM can be the ground, with `product: DSM` in the row. Every product made on it then shows
"ground: surface model" (section 4).

## 2. One rule for heights

- Every height is in **meters above the vertical datum of the reference**, at scale 1.
- Put a height from another source (an OSM `ele` tag, a bridge profile, a LiDAR road surface) on that datum first.
- **Exaggeration is only for the display.** It applies to all heights together: the terrain, the roads, the decks, the
  buildings and the vehicles.

## 3. One ground that every renderer draws: the conformed ground

| Layer | What it is | Who makes it |
|---|---|---|
| reference ground | the DTM of section 1, as a grid (`main.elevation_grid`, the pixels of the raster) | duckOSM |
| road heights | the designed (graded) height of each carriageway, from the reference ground, its structures and its rules | SimReady (`sim.elevation_profile`, method `graded`) |
| **conformed ground** | the reference ground changed to fit the roads: under a roadway it is the road height, near the road it blends back to the DTM, away from roads it is the DTM | SimReady (`terrain-conform`) |

- **Every renderer draws the conformed ground**: the GLB terrain, roadstyle's map terrain, Cesium, CARLA, Unity. For an
  area with a reference, no renderer uses another elevation source (for example the AWS Terrain Tiles).
- For such an area, roadstyle's map terrain comes from raster tiles of the conformed ground (Terrarium PNG, as MapLibre
  reads them), not from `ontop.dem` in `basemaps.json`.
- A page or a file for an area with no reference shows "ground: none" or "ground: AWS Terrain Tiles" to its user.
- The height of a road surface, a deck or a vehicle is the road height, never a second sample of a DEM.

### Where the ground cannot hold a road

The conformed ground is a height grid: one height at each point. It cannot hold a road in three cases:

| Case | Why | Example (Monaco) |
|---|---|---|
| bridge | the road is above the ground; the ground under it stays the DEM | Boulevard du Larvotto, 10 to 16 m above the ground |
| tunnel | the road is below the ground; the ground above it stays the DEM | Boulevard du Larvotto, 5 to 13 m below the ground |
| two roads side by side at different heights | a wall is between them, and a grid has no walls: it makes a saw-tooth | the ramp to Avenue Pasteur, 6.9 m above the road beside it, 5.2 m away |

In Monaco, 60 of 1,892 ground-level carriageways (about 1.2 km) have a road at a different height less than 8 m away
(steeper than 1:2).

**The rule:** a renderer draws each road at its road height. Where the ground holds the road, the renderer puts the road on
the conformed ground. Where it does not, the renderer draws the road as a surface of its own, with a wall from its edge
to the ground. The GLB does this: each carriageway is a mesh at its graded height. A map that puts every road on its
terrain (the flat road layers of MapLibre) is wrong at these roads, also on the conformed ground.

## 4. Every product names its ground

| Product | Where the ground id goes |
|---|---|
| duckOSM database | `main.elevation_metadata` (`sha256`, `product`, `vertical_datum`) |
| SimReady model | `sim.source`, and the `source_id` of each `elevation_profile` row |
| `.xodr` | `<header>`: a `<userData code="ground">` with the ground id and the datum |
| `.glb` | `extras.ground` of the root node |
| SUMO network | a `<param key="ground" .../>` of the network |
| roadstyle page | the page option `ground` (the id, the source, the datum), shown with the attribution |
| vehicles run report | `ground` |

Two files of one area with different ground ids do not go together. A tool that reads both stops with an error that
names the two ids.

## 5. Checks

| Check | Pass when |
|---|---|
| the reference | `product = DTM`, `vertical_datum` is known, `sha256` equals the hash of the file |
| the road on its ground | under each roadway that the ground holds (section 3), the road surface minus the conformed ground is at most 0.05 m |
| the map terrain | the terrain tiles of a page minus the conformed ground, at the points of the tile pixels (the north-west corner of each pixel, where MapLibre reads it): at most 0.10 m (the PNG step) |
| one ground in an area | every product of the area has the same ground id |

Do the map terrain check at the tile pixels, not at the grid points: MapLibre interpolates the tile pixels again, so at the
grid points the difference is some meters on steep ground.

Related: [The terrain of an area](terrain.md), [Add elevation](../guides/elevation.md),
[Elevation sources](../reference/elevation-sources.md).
