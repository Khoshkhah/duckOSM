# duckOSM Studio

duckOSM Studio is the dashboard of duckOSM. Choose an area on the map, build its road network, fix the OSM data, and
export the result.

[Open duckOSM Studio](https://khoshkhah.github.io/duckosm-studio/){ .md-button .md-button--primary }
[The manual](https://khoshkhah.github.io/duckosm-studio/manual/){ .md-button }

![The dashboard: the steps in the panel, and the area on the map](../images/studio.png)

## The steps

| Step | Does |
|---|---|
| Choose an area | search a place, draw a shape, open a GeoJSON file, or click H3 cells |
| OSM data | a Geofabrik extract that covers the area, or your own `.osm.pbf` file |
| Build | the driving, walking and cycling networks, with the levels and the base-map layers |
| Ground | the height of the ground (a DEM) of the area |
| Edit OSM data | change a road, a building or a turn rule, then build again |
| Edit levels | say which road is on top at a bridge or a tunnel |
| Edit design | the theme, the colours and the widths of the map |
| Export | `.duckdb`, PMTiles, GeoPackage, GeoJSON, SUMO, MATSim and more |

## Two versions

| | Online | On your computer |
|---|---|---|
| Start | open [the page](https://khoshkhah.github.io/duckosm-studio/) | `duckosm-rust studio areas`, then open `http://127.0.0.1:8792/` |
| The build runs | in the browser (WebAssembly) | in the native program |
| The areas are | in the browser's storage | in folders on your disk |
| Largest area | about the size of Kentucky (4 GB of the tab) | a country, by tiles |

Both versions use the same steps and give the same tables. See [Offline and online builds](../concepts/offline-online.md).

More about the version on your computer (its options, your areas, the limits):
[the details on GitHub](https://github.com/Khoshkhah/duckOSM/blob/main/docs/rust/studio_local.md).
