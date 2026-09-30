# railML (experimental)

```bash
duckosm railml monaco.duckdb        # -> monaco.railml.xml: 13 tracks, 6 switches, 1 station
```

A railML 2.4 infrastructure file, for OpenTrack, RailSys and Viriato. duckOSM has no rail network,
so this command reads the railways from the raw OSM data kept in the database (`rail`, `light_rail`,
`subway`, `tram`, `narrow_gauge`, `funicular`) and builds:

- **tracks**, split at switches and where tracks meet, connected end to end;
- **switches** (`railway=switch`) and **signals** (`railway=signal`) at their position on the track;
- **stations** (`railway=station` / `halt`) and **buffer stops**.

**Limits:** infrastructure only (no timetable or rolling stock). OSM rarely tags electrification,
gauge, speed or gradient, so they are left out. Where three or more track ends meet, the switch is
approximate. Checked for structure only (the railML schema needs registration), not yet loaded in a
rail tool.

```python
from duckosm import to_railml
to_railml("monaco.duckdb", "monaco.railml.xml")     # -> {"tracks", "switches", "signals", "ocps"}
```
