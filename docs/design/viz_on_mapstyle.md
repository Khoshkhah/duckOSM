# duckOSM's maps on mapstyle

**Status:** implemented 2026-10-01, as approved by Kaveh: `duckosm viz` writes both an all-modes page and one page per
mode; duckOSM's page code is removed (routing stays in duckOSM); the background is blank (the
database's own layers).

## Problem

duckOSM draws its own maps: `duckosm viz` (roads only, one page per mode, over a web background) and
`duckosm route-map` (a driving-only route planner), in `viz.py` and `route_map.py` (669 lines, most of
them page JavaScript). mapstyle, the companion map library, now does both better, from the same
database: a full base map drawn from `features.*`, every mode on one map, and a planner that also
walks, cycles and combines walking and driving. Its planner began as a copy of `route_map.py`, so
today the same code lives in two places and every fix is needed twice.

mapstyle 0.2.0 (PyPI) closed the gaps found in the parity check: it draws `private_edges` (private
roads grey, bus lanes blue, a row each in the Roads box) and its planner has a Roads box. Checked on
Monaco: the same route as `route-map` (2 min 38 s, 1.9 km, the same directions), Walk + drive works,
no page errors.

## Routing stays in duckOSM (Kaveh, 2026-10-01)

duckOSM owns routing, as Python tools; mapstyle only draws the page. The tools today: `route` /
`Router` (edge to edge over legal turns), `route_multimodal` (node to node across modes),
`route_points` / `route_multimodal_points` (between two points: off-road access within a radius,
exact partial edges). **New:** `directions(con, route)` returns the turn-by-turn steps ("Turn left
onto …", "At the roundabout, take the 2nd exit onto …"). Its rules exist today only as page
JavaScript (in `route_map.py`, and copied into mapstyle's planner); they move into Python here, so
duckOSM is the reference.

The planner page still routes in the browser (a static page can't call Python), so its page code is
routing code too and lives in duckOSM (`templates/planner.html`, copied from mapstyle's demo planner,
which mapstyle keeps only as a demo; Kaveh, 2026-10-01: "routing is not part of its job"). duckOSM's
`route_map.py` asks mapstyle for the map (`render_map`, `load_roads`) and adds its planner on top; the
page finds the map's roads by `edge_id`. A test keeps page and Python equal: on Monaco, in each mode,
the page's directions for its route equal `directions()` for the same edges (tests/test_maps.py).

## Change

| Today | With mapstyle |
|---|---|
| `duckosm viz DB` → one roads-only page per mode, `reports/<name>_<mode>_network.html` | one page with every mode over the full base map, `reports/<name>_map.html` (decision 1); `-m walking` brings that mode to the front |
| `duckosm route-map DB` → driving planner, `reports/<name>_route_map.html` | duckOSM's planner page (Drive / Walk / Cycle, and Walk + drive when the db has `mm.*`) over mapstyle's map, same file name |
| build option `viz.enabled` → `render_network` per mode | the same call as `duckosm viz` |
| `viz` extra: `roadstyle>=0.11.0` | `mapstyle>=0.2.0` (it brings roadstyle) |

Both commands draw through `mapstyle.render_map(db, mode=…, planner=…, theme=…)`; their
options map onto it (`-m` → `mode`, `--basemap` → roadstyle's `basemap`, `-o` / `--out-dir`). The
roadstyle page code in `viz.py` and the old planner page in `route_map.py` are replaced: `viz.py` keeps
only what the build and the CLI call, `route_map.py` builds the planner's data and adds its page. duckOSM gets map features and
fixes by upgrading mapstyle.

**What changes for a user:**

- The map shows the city, not just roads: water, land, buildings, the sea, from the database (no web
  background needed; one can still be chosen).
- Pages are about twice as large (Monaco: map 3.4 MB instead of 1.7 MB, planner 5.1 MB instead of
  1.8 MB): fine for a city, and both commands already warn above 100,000 edges.
- On a page with every mode, a road that cars may not use but walkers may (a bus lane, most private
  paths) is drawn as an ordinary road for walking; the Private roads / Bus lanes rows cover roads no
  shown mode may use. A single-mode page (`-m driving`) keeps them all marked.

## Docs and tests

- Draw a map and Route: the commands' new output, the mapstyle section becomes the main text, the
  three docs maps redrawn with mapstyle (the docs CARTO key for any CARTO background), Development
  and the CLI reference updated, the agent skill too.
- Tests: `test_route_map.py` (which tests duckOSM's own page) is replaced by a test that both
  commands write a page through mapstyle on the Monaco sample (skipped without the `viz` extra, as
  now). mapstyle's own tests cover the page.

## Decisions for Kaveh

1. **`duckosm viz` output.** Recommended: one page with every mode (`<name>_map.html`), mapstyle's
   way; `-m` brings a mode to the front. Alternative: keep one page per mode, each made by mapstyle
   with that mode in front (the same file names as today).
2. **Remove duckOSM's own map code.** Recommended: yes, `route_map.py` and the page code in `viz.py`
   go. Alternative: keep them as a fallback when mapstyle isn't installed (two copies again).
3. **Default background.** Recommended: mapstyle's (the database's own layers, no web tiles).
   Alternative: keep CARTO Voyager under the map as today.
