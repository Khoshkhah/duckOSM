# OpenDRIVE (experimental)

`duckosm opendrive` writes an ASAM OpenDRIVE 1.7 `.xodr` file, with your OSM fixes, for driving simulators (CARLA,
esmini) and microsimulators (PTV Vissim, Aimsun). Each OpenDRIVE road id is the duckOSM `edge_id`.

| Reference and format | |
|---|---|
| Format | [ASAM OpenDRIVE 1.7](https://www.asam.net/standards/detail/opendrive/) (`.xodr`) |
| Reference tool | none: no test loads the file in CARLA or [esmini](https://github.com/esmini/esmini) yet |
| Check | the tests check the XML structure and the order of its elements. [How each output is checked](../concepts/checks.md) |

```bash
duckosm opendrive monaco.duckdb                       # roads only
duckosm gmns monaco.duckdb                            # for the junctions: a GMNS database first
duckosm opendrive monaco_gmns.duckdb --junctions      # roads, connecting roads and junctions
```

Without `--junctions`, the roads do not link: you can look at them, but a vehicle cannot drive from road to road. With
`--junctions`, each legal turn becomes a connecting road, so vehicles can drive through the junctions.

## What goes in

| OpenDRIVE item | From duckOSM |
|---|---|
| `road` | one direction of an edge. `id` is the `edge_id` |
| reference line | the edge line, in metres, in the UTM zone of the data (or `--crs`) |
| `lane` | the driving lanes, each 3.25 m wide |
| elevation profile | the heights of [`duckosm elevation`](../guides/elevation.md). Not with `--junctions` |
| connecting road | one legal turn through a junction, as a smooth curve (`--junctions`) |
| `junction` | the connecting roads of one junction (`--junctions`) |

## Speed and memory

Mac mini (Apple M6, 12 cores, 16 GB). With `--junctions`.

| Area (input) | Time | Memory | Output |
|---|---|---|---|
| Monaco (0.7 MB) | 0.06 s | 50.5 MB | 6.0 MB |
| Kentucky (148 MB) | not run: `duckosm gmns` stops on Kentucky | | |
| France (5.1 GB) | not run | | |

## Limits

| Limit | Why |
|---|---|
| Each lane is 3.25 m wide | OSM `width:lanes` is not read yet |
| A connecting road has one lane, joined to the rightmost lane | not made yet |
| A connecting road ends about 2 m from its road, sometimes more. A strict simulator can report it | not fixed yet |
| No road type, speed limit, markings, signals or banking | not made yet |
