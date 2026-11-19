# railML (experimental)

`duckosm railml` writes a railML 2.4 infrastructure file, for rail tools such as OpenTrack, RailSys and Viriato.

| Reference and format | |
|---|---|
| Format | [railML 2.4](https://www.railml.org/en/documentation-railml2), infrastructure ([XML tree](https://wiki2.railml.org/wiki/Dev:XMLtree/2.4)) |
| Reference tool | none: the file is not yet loaded in a rail tool |
| Check | none yet against the railML schema, because it needs a registration. The tests check the XML structure and its references. [How each output is checked](../concepts/checks.md) |

```bash
duckosm railml monaco.duckdb        # -> monaco.railml.xml
```

Monaco test build: 13 tracks, 6 switches, 1 station.

## What it makes

duckOSM has no rail network. This command reads the railways from the raw OSM data in the database (`raw.*`). A build from
an `.osm.pbf` keeps it; an area clipped from a bigger build has none. Railway types: `rail`, `light_rail`, `subway`,
`tram`, `narrow_gauge`, `funicular`.

| railML item | From OSM |
|---|---|
| track | a railway, cut at switches and where tracks meet, with its length |
| switch | `railway=switch`, or a node where three or more tracks meet |
| signal | `railway=signal`, at its position on the track |
| station (OCP) | `railway=station` or `halt`, with its coordinates |
| buffer stop | `railway=buffer_stop` at a track end; other dead ends are open ends |

## Options

| Option | Does | Default |
|---|---|---|
| `-o`, `--out` | the output file | `<name>.railml.xml` |

```python
from duckosm import to_railml
to_railml("monaco.duckdb", "monaco.railml.xml")     # -> {"tracks", "switches", "signals", "ocps"}
```

Design: [railml_export](https://github.com/Khoshkhah/duckOSM/blob/main/docs/design/railml_export.md).

## Speed and memory

Mac mini (Apple M6, 12 cores, 16 GB). Time: the wall-clock time. Memory: the peak memory footprint (`/usr/bin/time -l`). 1 MB = 10⁶ bytes.

| Area | Time | Memory | Output size |
|---|---|---|---|
| Monaco (0.7 MB) | 0.03 s | 18.6 MB | 0.1 MB |
| Kentucky (148 MB) | measuring |  |  |
| France (5.1 GB) | measuring |  |  |

The command: `duckosm railml area.duckdb`. The input is the build with `--features`: railML reads `raw.*`.

## Limits

- Infrastructure only: no timetable and no rolling stock.
- No electrification, gauge, speed or gradient: OSM seldom has these tags.
- At a switch, only two tracks are joined. The third branch is an open end, and the switch has no connections.
- Tracks have no geometry. Only stations have coordinates.
- The direction of a signal is not read.
