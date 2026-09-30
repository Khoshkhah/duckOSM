# GMNS pretty maps — `duckosm gmns-map`

Write a **pretty, self-contained HTML map** of a GMNS DuckDB — a presentation counterpart to the
utilitarian [`gmns-viz`](gmns_viewer.md) viewer. Two styles, both on a dark canvas with pan/zoom, no
server / tiles / external assets (geometry drawn client-side; needs only DuckDB).

```bash
duckosm gmns-map monaco_gmns.duckdb                  # -> monaco_gmns_road.html (road style)
duckosm gmns-map monaco_gmns.duckdb --style lane     # every lane as a width ribbon
```

```python
from duckosm.gmns_map import write_map
write_map("monaco_gmns.duckdb", "road.html", style="road")
```

## `--style road` — one carriageway per direction

![Roads by direction](images/gmns_road_direction.png)

Each **directed link** becomes one **carriageway ribbon**, width = its lane count × lane-width,
offset to the travel side — so a **two-way road splits into two ribbons** (one each way) with a median
gap. Coloured by road class (trunk / primary / secondary / tertiary / local) and **cased** (dark
outline) like a cartographic map; smooth turn connectors overlaid in cyan.

## `--style lane` — every lane, its own width

![Every lane as a width ribbon](images/gmns_lane_width.png)

Each **lane** is drawn as a ribbon of its **real width** (metres — `width:lanes` where OSM tags it,
else the 3.25 m default) along its own geometry, coloured by use (auto / **bus** / bike), with thin
gaps between lanes acting as **markings**. Reads like an actual road surface.

## Notes

- Both styles depend on the drive-side lane offset and the smooth Bézier connectors (see
  [gmns_map_realism.md](gmns_map_realism.md)); the turn overlay needs a `meso_<mode>` schema
  (`duckosm gmns --meso`), otherwise it's omitted.
- **Fidelity:** the shapes/positions/offsets are real, but carriageway/lane **width** is driven by the
  OSM `lanes` count (tagged on ~9% of Södermalm ways, else a class default) and the default lane
  width — so widths are approximate until enriched (e.g. from NVDB) with real lane counts/widths.
- For *inspecting* the data (ids, attributes, layers, hover), use [`gmns-viz`](gmns_viewer.md); these
  maps are for *showing* it. Module: `src/duckosm/gmns_map.py` (`write_map`).
