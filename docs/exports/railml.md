# railML (experimental)

```bash
duckosm railml monaco.duckdb        # -> monaco.railml.xml: 13 tracks, 6 switches, 1 station
```

A railML 2.4 infrastructure file, for OpenTrack, RailSys and Viriato. duckOSM has no rail network,
so this command reads the railways from the raw OSM data kept in the database (a build from a
`.osm.pbf`; an area clipped from a bigger build has none) (`rail`, `light_rail`,
`subway`, `tram`, `narrow_gauge`, `funicular`) and builds:

- **tracks**, split at switches and where tracks meet, with their length;
- **switches** (`railway=switch`, or where three or more tracks meet) and **signals**
  (`railway=signal`) at their position on the track;
- **stations** (`railway=station` / `halt`), and **buffer stops** (`railway=buffer_stop` at a track
  end; other dead ends are open ends).

**Limits:** infrastructure only (no timetable or rolling stock). OSM rarely tags electrification,
gauge, speed or gradient, so they are left out. At a switch only two of the tracks are joined: the
third branch is left as an open end, and the switch has no connections yet. Tracks have no
geometry (only stations have coordinates), and a signal's direction isn't read. Checked for structure only (the railML schema needs registration), not yet loaded in a
rail tool.

```python
from duckosm import to_railml
to_railml("monaco.duckdb", "monaco.railml.xml")     # -> {"tracks", "switches", "signals", "ocps"}
```
