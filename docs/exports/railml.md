# railML (experimental)

!!! warning "Experimental"
    The file is checked for structure only. It is not yet loaded in a rail tool.

`duckosm railml` writes a railML 2.4 infrastructure file, for rail tools such as OpenTrack, RailSys and Viriato.

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

## How it is checked

The tests check the XML structure. They do not use the railML schema, because it needs a registration.
See [How each output is checked](../concepts/checks.md).

## Limits

- Infrastructure only: no timetable and no rolling stock.
- No electrification, gauge, speed or gradient: OSM seldom has these tags.
- At a switch, only two tracks are joined. The third branch is an open end, and the switch has no connections.
- Tracks have no geometry. Only stations have coordinates.
- The direction of a signal is not read.
