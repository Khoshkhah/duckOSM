# `duckosm route-map`: an interactive route planner in one page

**Status:** implemented 2026-09-30 (`src/duckosm/route_map.py`, `duckosm route-map`); in the browser
the page gives the same answers as `route()` / `route_multimodal()` (checked on Monaco: 8 of 8 trips).
Tests (`tests/test_route_map.py`) and the Route guide (a driving map) done 2026-09-30. **Next:** it
moves into mapstyle as `ms.render_route_planner` (`../mapstyle/docs/PLAN.md`), where walking and
cycling get their own styles; until then duckOSM's docs show driving only.

## Goal

One command that turns a build into a page where you **drop a start and an end on the map and see
the route**, with no server: routing runs in the browser. It borrows the design of the private
`route-viewer` planner (markers, mode check boxes, per-mode legs) but needs nothing outside duckOSM,
so every user has it and the docs can embed a live one.

```bash
duckosm route-map monaco.duckdb                  # -> reports/monaco_route_map.html
```

## The page

- A roadstyle map, roads in neutral grey so the route stands out.
- **Click the map to drop the start, click again for the end;** drag either marker to move it. Each
  marker snaps to the nearest road.
- **A mode menu:** Drive, Walk, Cycle (the modes the db has), and **Walk + drive** when the db has
  the `mm` tables; fastest / shortest for one mode.
- The route is drawn as its own line at the top of its level (over the roads it crosses, under a
  bridge above it), from roadstyle's road source; a panel shows time, length and
  the streets in order, and for a trip across modes, each leg in its mode's colour (walking green,
  driving red, cycling blue).

## Same answers as the Python API

The page embeds compact JSON graphs (node ids remapped to small integers: some ids pass 2**53) and
runs Dijkstra in JavaScript:

- **One mode ticked:** over that mode's `edge_graph` (edge to edge, legal turns only), weight
  `cost_s` or `length_m`: exactly `route()`.
- **Several modes ticked** (Walk must be one of them): over `(node, mode, phase)` states from
  `mm.edges` and `mm.transfers`, `walk* vehicle* walk*`, start and end on foot: exactly
  `route_multimodal(..., allowed_modes=...)`. This needs the `mm` tables (`duckosm multimodal`);
  without them the page offers one mode at a time. Like `route_multimodal`, it goes junction to
  junction and does not see turn restrictions; the page says so.

Tests check the embedded graphs against the db; a browser check (playwright, run by hand) compares
routes in the page with `route()` / `route_multimodal()`.

## Options

| Option | Does | Default |
|---|---|---|
| `-m MODE` | modes to include; repeat for several | every mode in the db |
| `--basemap` | as `duckosm viz` | `osm` (keyless) |
| `-o FILE` | output file | `reports/<name>_route_map.html` |

## Limits

- Everything is in the page: fine for a city (Monaco, all modes: ~20 k edges, a few MB), heavy for a
  country; above ~100 k edges the command warns and suggests clipping an area first.
- Later: turn-by-turn directions (route-viewer gets them from the private route-guidance).
- **Not truly multimodal yet.** `route_multimodal` is walk + one vehicle, with the vehicle available
  at any junction. Walk + cycle is left out of the menu until there are bike-share stations. Real
  multimodal needs where you can change: bike-share stations, parking, bus stops, and bus
  timetables (GTFS).

## Where it lives

`src/duckosm/route_map.py` + a CLI command. Needs `pip install "duckosm[viz]"` (roadstyle). Docs: the
Route guide embeds the Monaco page; the CLI reference lists the command.
