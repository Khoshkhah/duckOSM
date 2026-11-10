# The ground reference

Status: the standard, accepted 2026-11-09. It applies to duckOSM, SimReady, urbanstyle, roadstyle and the projects that
read their outputs.

<p class="lead">One area has one ground. Every project computes on it, every renderer draws it, and every file says which
ground it was made on.</p>

## Why

Before this standard, each project chose its own ground. roadstyle's map terrain came from the AWS Terrain Tiles (Terrarium:
SRTM and other sources); duckOSM sampled Copernicus GLO-30 (a surface model with roofs and trees); SimReady used FABDEM
for Monaco and a 1 m LiDAR DTM for Vancouver. No file said which ground it was made on. So a road, a bridge deck or a
vehicle with its own height floated above the drawn terrain or sank into it, and no check could find the cause.

## 1. One ground reference for each area

The ground reference of an area is one elevation file. duckOSM records it, in `main.elevation_metadata`, the row of `ele`.

| Field | Rule |
|---|---|
| `product` | `DTM` (bare earth). A surface model (`DSM`, Copernicus GLO-30) is not a reference: its roofs and trees go into the road heights |
| `source`, `uri` | the file or the model, with its version (`FABDEM V1-2`, a national DTM, a LiDAR DTM) |
| `dem_crs` | its CRS |
| `vertical_datum` | known (`EGM2008`, `EGM96`, a national datum). `unknown` is not a reference |
| `resolution_m`, `license` | known |
| `sha256` | the hash of the file's bytes, the **ground id** of every product below. NULL when no whole file was read (Copernicus tiles streamed from AWS, a URL): such a source is not a reference |

The other projects do not choose a ground. They read this row (or the same values in the area's settings, for SimReady:
`areas/<area>/area.yaml` `dem:` and `sources.yaml`, which must name the same file).

If no DTM exists for an area, a DSM can stand in, but only with `product: DSM` in the row. Every product made on it then
shows "ground: surface model" (section 4). It is never silent.

## 2. One rule for heights

- Every height is in **metres above the vertical datum of the reference**, at scale 1.
- A height from another source (an OSM `ele` tag, a bridge profile, a LiDAR road surface) is first put on that datum.
- **Exaggeration exists only at display time.** It applies to everything with a height together: the terrain, the roads,
  the decks, the buildings and the vehicles.

## 3. One ground that every renderer draws: the conformed ground

| Layer | What it is | Who makes it |
|---|---|---|
| reference ground | the DTM of section 1, as a grid (`main.elevation_grid`, the raster's own pixels) | duckOSM |
| road heights | the designed (graded) height of each carriageway, from the reference ground, its structures and its rules | SimReady (`sim.elevation_profile`, method `graded`) |
| **conformed ground** | the reference ground bent to the roads: under a roadway it is the road's height, beside it it blends back to the DTM, away from roads it is the DTM | SimReady (`terrain-conform`) |

- **Every renderer draws the conformed ground**: the GLB terrain, roadstyle's map terrain, Cesium, CARLA, Unity. No
  renderer uses another elevation source (for example the AWS Terrain Tiles) for an area that has a reference.
- roadstyle's map terrain for such an area comes from raster tiles made from the conformed ground (encoded as Terrarium
  PNG, as MapLibre reads them), not from `basemaps.json` `ontop.dem`.
- A page or a file for an area with no reference says "ground: none" or "ground: AWS Terrain Tiles" where its user sees it.
- A road surface, a deck, a vehicle: its height is the road height (section 3), never a second sample of a DEM.

## 4. Every product names its ground

| Product | Where the ground id goes |
|---|---|
| duckOSM database | `main.elevation_metadata` (`sha256`, `product`, `vertical_datum`) |
| SimReady model | `sim.source`, and each `elevation_profile` row's `source_id` |
| `.xodr` | `<header>`: a `<userData code="ground">` with the ground id and the datum |
| `.glb` | the root node's `extras.ground` |
| SUMO network | a `<param key="ground" .../>` of the network |
| roadstyle page | the page option `ground` (the id, the source, the datum), shown with the attribution |
| vehicles run report | `ground` |

Two files of one area with different ground ids do not belong together. A tool that reads both stops with an error that
names the two ids.

## 5. Checks

| Check | Pass when |
|---|---|
| the reference | `product = DTM`, `vertical_datum` known, `sha256` equals the file's |
| the road on its ground | under each roadway, the road surface minus the conformed ground is at most 0.05 m |
| the map terrain | the terrain tiles of a page minus the conformed ground grid, at the grid's points: at most 0.10 m (the PNG step) |
| one ground in an area | every product of the area has the same ground id |

## Where each project stands (2026-11-10)

| Project | Now | To do |
|---|---|---|
| duckOSM | Done. `main.elevation_metadata` has `product`, `vertical_datum`, `license` and `sha256` (the ground id) in the Python command and in the Rust build (file and browser bytes); `main.elevation_grid` from the Rust build. A `--dem` file in `ele` needs its product, its vertical datum and its license (options or the config section `elevation`), or `--not-reference`. The default source with no `--dem` is still Copernicus GLO-30 (a DSM, no ground id) | nothing |
| SimReady | Monaco on FABDEM (a DTM), Vancouver on a 1 m LiDAR DTM; `terrain-conform` and the GLB on the branch `claude/gltf-adapter-design-5047c6`; the connecting roads of the `.xodr` have no elevation | the ground id in its products; the conformed ground as a grid that others read; the elevation of the connecting roads |
| roadstyle | the map terrain is always the AWS Terrain Tiles | terrain tiles from the conformed ground for an area with a reference; the page option `ground`; the vehicles take the road heights |
| urbanstyle | a "rough terrain" switch with the AWS Terrain Tiles; bridge heights from LiDAR feed SimReady | the same terrain tiles as roadstyle |

Related: [Add elevation](../guides/elevation.md), [Elevation sources](../reference/elevation-sources.md).
