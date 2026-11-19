# The ground reference

One area has one ground. Each height of the area is measured on it, each map and 3D model draws it, and each file names
it. Without one ground, a road or a bridge floats above the drawn terrain or goes into it.

## The reference: one elevation file

The ground of an area is one bare-earth elevation file (a DTM). duckOSM records it in the table
`main.elevation_metadata`:

| Field | Must be |
|---|---|
| `product` | `DTM` (bare earth). A surface model (DSM, for example Copernicus) is not a reference: its roofs and trees go into the road heights |
| `source` | the file or the model, with its version, for example `FABDEM V1-2` |
| `vertical_datum` | known, for example `EGM2008` |
| `sha256` | the hash of the file: the **ground id**. A source with no whole file (tiles from a server) has none, and is not a reference |

When no DTM exists for an area, a DSM can be the ground, with `product: DSM`. Each product made on it then says
"ground: surface model".

## One rule for heights

- Each height is in metres above the vertical datum of the reference. A height from another source (an OSM `ele` tag,
  a bridge profile) goes on that datum first.
- Exaggeration is only for the display, and it applies to all heights together.

## The ground that the maps draw

| Layer | What it is | Made by |
|---|---|---|
| reference ground | the DTM, as a grid of heights | duckOSM |
| road heights | the designed height of each carriageway | SimReady |
| **conformed ground** | the reference ground changed to fit the roads: under a road it is the road height, away from roads it is the DTM | SimReady |

Each renderer (roadstyle, the 3D model, CARLA) draws the conformed ground, and no other elevation source.

### Where the ground cannot hold a road

A height grid cannot hold a bridge, a tunnel, or two roads side by side at different heights. There, the renderer draws
the road as a surface of its own, at its road height.

## Each file names its ground

| Product | Where the ground id goes |
|---|---|
| duckOSM database | `main.elevation_metadata` |
| OpenDRIVE `.xodr` | `<userData code="ground">` in the header |
| SUMO network | `<param key="ground">` |
| 3D model `.glb` | `extras.ground` of the root node |
| roadstyle map | the page option `ground` |

Two files of one area with different ground ids do not go together: a tool that reads both stops with an error.

## Checks

| Check | Passes when |
|---|---|
| the reference | `product = DTM`, `vertical_datum` is known, and `sha256` equals the hash of the file |
| the road on its ground | where the ground holds the road, the road minus the conformed ground is at most 0.05 m |
| the map terrain | the terrain tiles of a map minus the conformed ground is at most 0.10 m |
| one ground in an area | each product of the area has the same ground id |

Related: [The terrain of an area](terrain.md), [Add elevation](../guides/elevation.md),
[Elevation sources](../reference/elevation-sources.md). The history and the state of each project:
[ground reference in detail](https://github.com/Khoshkhah/duckOSM/blob/main/docs/design/ground_reference.md).
