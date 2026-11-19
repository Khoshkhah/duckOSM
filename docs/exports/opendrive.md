# OpenDRIVE (experimental)

`duckosm opendrive` writes an ASAM OpenDRIVE 1.7 `.xodr` file, for driving simulators (CARLA, esmini) and
microsimulators (PTV Vissim, Aimsun). Each OpenDRIVE road id is the duckOSM `edge_id`.

| Reference and format | |
|---|---|
| Format | [ASAM OpenDRIVE 1.7](https://www.asam.net/standards/detail/opendrive/) (`.xodr`) |
| Reference tool | none: the file is not yet loaded in CARLA or [esmini](https://github.com/esmini/esmini) |
| Check | none yet against the ASAM schema. The tests check the XML structure and the order of its elements. [How each output is checked](../concepts/checks.md) |

```bash
duckosm opendrive monaco.duckdb                       # -> monaco.xodr: roads only
duckosm gmns monaco.duckdb                            # for the junctions: a GMNS database first
duckosm opendrive monaco_gmns.duckdb --junctions      # -> monaco_gmns.xodr: roads, connecting roads, junctions
```

Monaco test build: 2,844 roads; with `--junctions`, 2,862 roads, 1,888 connecting roads and 493 junctions.

## What it makes

| OpenDRIVE item | From duckOSM |
|---|---|
| road | one directed edge; `road id` = `edge_id` |
| reference line | the edge line, in meters (the UTM zone of the data, or `--crs`) |
| lanes | the driving lanes, each 3.25 m wide |
| elevation profile | the heights of [`duckosm elevation`](../guides/elevation.md); not with `--junctions` |
| connecting road (`--junctions`) | one legal turn at an intersection, along a smooth curve |
| junction (`--junctions`) | the connecting roads of one intersection |

Without `--junctions`, the roads have no links to each other: you can look at them, but a vehicle cannot drive from
road to road. With `--junctions`, the command reads a [GMNS database](gmns.md). Each legal turn becomes a connecting
road, so vehicles can drive through. Where a road continues, the two roads link directly.

## Options

| Option | Does | Default |
|---|---|---|
| `-m`, `--mode` | the mode | `driving` |
| `--crs` | the projected CRS of the reference line | the UTM zone of the data |
| `--junctions` | add the junctions and the connecting roads (needs a GMNS database) | off |
| `-o`, `--out` | the output file | `<name>.xodr` |

```python
from duckosm import to_opendrive
to_opendrive("monaco.duckdb", "monaco.xodr")                              # -> {"roads", "crs", ...}
to_opendrive("monaco_gmns.duckdb", "monaco.xodr", junctions=True)
```

Design: [opendrive_export](https://github.com/Khoshkhah/duckOSM/blob/main/docs/design/opendrive_export.md).

## Limits

- Each lane is 3.25 m wide: OSM `width:lanes` is not used.
- A connecting road has one lane. It links to the rightmost lane of the roads it joins.
- A connecting road starts and ends near the road ends: usually about 2 m off, sometimes tens of meters. A strict
  simulator can report this.
- No road type, speed limit, road markings, signals or banking.
