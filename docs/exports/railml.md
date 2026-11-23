# railML (experimental)

`duckosm railml` writes the railways of an area as a railML 2.4 infrastructure file, with your OSM fixes, for rail
planning tools such as OpenTrack, RailSys and Viriato. It reads the rail ways and nodes from `raw.*` of the Level 2 db, not from the GMNS db.

| Reference and format | |
|---|---|
| Format | [railML 2.4](https://www.railml.org/en/documentation-railml2), infrastructure |
| Reference tool | none: no test loads the file in a rail tool yet |
| Check | the tests check the XML structure and its references. A check against the railML schema needs a registration. [How each output is checked](../concepts/checks.md) |

```bash
duckosm railml monaco.duckdb        # -> monaco.railml.xml
```

The command reads the railways from the raw OSM tables (`raw.*`) of a build from an `.osm.pbf`. A database cut from a
larger one has no raw tables.

## What goes in

| railML item | From OSM |
|---|---|
| track | a railway (`rail`, `light_rail`, `subway`, `tram`, `narrow_gauge`, `funicular`), cut at switches, with its length |
| switch | `railway=switch`, or a node where three or more tracks meet |
| signal | `railway=signal`, at its position on the track |
| station | `railway=station` or `halt`, with its coordinates |
| buffer stop | `railway=buffer_stop` at the end of a track |

## The file

| Element | Attributes | Holds |
|---|---|---|
| `infrastructure` | `id` | all of the area |
| `ocp` (operation control point) | `id` (`ocp_<node id>`), `name` | a station or a halt |
| `ocp/geoCoord` | `coord` (longitude latitude), `epsgCode` 4326 | the position of the station |
| `track` | `id`, `type` (the OSM `railway` value) | one track |
| `trackBegin`, `trackEnd` | `id`, `pos` (metres from the track start) | the two ends of a track |
| `connection` | `id`, `ref` (the connection at the other track) | two tracks that join |
| `bufferStop`, `openEnd` | `id` | a track end with a buffer stop, or with nothing |
| `switch` | `id` (`sw_<node id>`), `pos`, `name` | a switch on the track |
| `signal` | `id` (`sig_<node id>`), `pos`, `dir` | a signal on the track |

The `<node id>` is the OSM node id, so you can find each item in OSM.

## Speed and memory

Mac mini (Apple M6, 12 cores, 16 GB).

| Area (input DuckDB) | Time | Memory | Output |
|---|---|---|---|
| Monaco (38 MB `.duckdb`) | 0.03 s | 18.6 MB | 0.1 MB |
| Kentucky (~1.2 GB `.duckdb`) | 0.22 s | 394 MB | 4.8 MB |
| France (~35 GB `.duckdb`) | not run | (needs raw tables on disk) | |

## Limits

| Not in the file | Why |
|---|---|
| timetable, rolling stock | railML infrastructure only |
| electrification, gauge, speed, gradient | OSM seldom has these tags |
| the third branch of a switch | only two tracks are joined; the third is an open end |
| the shape of a track | only stations have coordinates |
| the direction of a signal | not read yet |
