Short page: [The ground reference](../concepts/ground-reference.md). This page keeps the history and the state of each project.

# The ground reference: history and state

Status: the standard, accepted 2026-11-09. It applies to duckOSM, SimReady, urbanstyle, roadstyle and the projects that
read their outputs.

## Why

Before this standard, each project chose its own ground. roadstyle's map terrain came from the AWS Terrain Tiles (Terrarium:
SRTM and other sources); duckOSM sampled Copernicus GLO-30 (a surface model with roofs and trees); SimReady used FABDEM
for Monaco and a 1 m LiDAR DTM for Vancouver. No file said which ground it was made on. So a road, a bridge deck or a
vehicle with its own height floated above the drawn terrain or sank into it, and no check could find the cause.

## Where each project stands (2026-11-10, evening; roadstyle 2026-11-11)

| Project | Now | To do |
|---|---|---|
| duckOSM | Done. `main.elevation_metadata` has `product`, `vertical_datum`, `license` and `sha256` (the ground id) in `duckosm elevation` and in the Rust build (file and browser bytes); `main.elevation_grid` from the Rust build. A `--dem` file in `ele` needs its product, its vertical datum and its license (options or the config section `elevation`), or `--not-reference`. The default source with no `--dem` is still Copernicus GLO-30 (a DSM, no ground id) | nothing |
| SimReady | Monaco on FABDEM (a DTM), Vancouver on a 1 m LiDAR DTM; `terrain-conform` and the GLB on `main` (the `gltf` adapter); the connecting roads of the `.xodr` have no elevation | the ground id in its products; the conformed ground as a grid that others read; the elevation of the connecting roads |
| roadstyle | the map terrain comes from the area's ground (the page option `ground`: a PMTiles file made from `main.elevation_grid`; roadstyle PRs 145-147), and the attribution names it; the 3D bridge decks stand on it. With no ground, the AWS Terrain Tiles, and the page says so. The vehicles take SimReady's graded profile and the Monaco vehicles page draws the conformed ground (roadstyle branch `claude/vehicles-p2`, not merged) | draw the roads that the ground does not hold (bridges, tunnels, roads side by side) at their road height, with the 3D bridge decks on the same profile |
| urbanstyle | a "rough terrain" switch with the AWS Terrain Tiles; bridge heights from LiDAR feed SimReady | the same terrain tiles as roadstyle |
