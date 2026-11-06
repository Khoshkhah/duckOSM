# The fixes editor in duckOSM

**Status:** approved by Kaveh 2026-11-05, with choice 1 for the walls ("Option 1: duckosm builds walls"). Implemented on the branch
`claude/compassionate-zhukovsky-ad12b8`.

## Problem

The duckosm-rust page has a panel "Edit OSM data" (card 7). The panel writes a fixes file: duckOSM's `overrides`, `new_ways` and
`turn_restrictions`, and the extension keys `extra.tag_edits` and `extra.building_faces`. The engine code that applies the fixes is in
duckOSM (`rust/duckosm-core/src/fixes.rs`, `raw.rs`, `building_faces.rs`). The panel code is in duckosm-rust
(`crates/duckosm-wasm/www/app/fixes_panel.js`, `fixes.js`, `presets.json`, about 70 lines of markup and 90 lines of CSS in `index.html`).

The rule of Kaveh is that each editor lives with its owner. roadstyle owns the level editor, and the page copies it from roadstyle at a
pinned commit. duckOSM owns the fixes, so the fixes editor must live in duckOSM. Then the page takes the editor and the engine from one
duckOSM rev, and they cannot get out of step.

## Where the editor lives

A new folder `web/fixes_editor/` at the root of the repository:

| File | What it is | From duckosm-rust |
|---|---|---|
| `fixes.js` | The rules as data: read and write the fixes file, one function for each change. No DOM, no map. | `app/fixes.js`, no change |
| `fixes_panel.js` | The panel: its markup, the six tabs, the map layers, undo and redo. | `app/fixes_panel.js` and the markup of `index.html` |
| `fixes_panel.css` | The look of the panel. Every rule starts with `#fix-panel`. | the `#fix-panel` rules of `index.html` |
| `presets.json` | The fields of each tab (road, building, node, new road), and the guards between them. | `app/presets.json`, no change |
| `fixes_units.mjs` | The unit tests of `fixes.js` and of the readers of `fixes_panel.js` (Node, no browser). | `fixes_units.mjs` |
| `README.md` | The API for a host page (the section below, in short). | new |

The folder is not in the Python package. A pip install has no use for it, and only files in `src/duckosm/` ship. The CI job `test`
gets one more step: `node web/fixes_editor/fixes_units.mjs`.

The two Playwright tests `fixes_e2e.py` and `phone_fixes_e2e.py` stay in duckosm-rust. They test the page: the build, the card, the
phone sheet and the clicks on roadstyle's map. They need the page and its Wasm build, and duckOSM has neither.

## The API for a host page

The panel makes its own markup. The host page gives one element and the things that it owns:

```js
import { initFixesPanel } from "./fixes_editor/fixes_panel.js";

const panel = await initFixesPanel(document.getElementById("fix-panel"), {
	duck,          // the database: query(sql) → Arrow table, tables [{mode, table}], loadSpatial()
	map,           // the MapLibre map
	yaml,          // js-yaml (the host page loads it)
	status,        // status(text, ms): a short message for the user
	roadIdOfRow,   // the OSM way id of a road that roadstyle's map selects (event rs:select)
	areaKey,       // () → the name of the area: the browser keeps the changes of each area
	onChange,      // (summary, n): the number of changes is new
	onBuild,       // (): the user clicks "Build again"
});
```

Changes from the page as it is now:

- **The markup.** `initFixesPanel` adds the tabs and the body to the element (`insertAdjacentHTML("beforeend", …)`). The host keeps its
  own header in the element (the "Steps" button and the hide button belong to the page). The ids do not change (`fix-tab-road`,
  `fix-id`, `fix-apply`, …), so the Playwright tests of the page stay as they are.
- **`$` and `esc` leave `ctx`.** The panel has its own two helpers. They are one line each.
- **`onBuild` is new.** Now the page sets `$("fix-apply").onclick`. With the hook, the button belongs to the panel.
- **`presets.json`** loads from the folder of the module (`new URL("presets.json", import.meta.url)`), not from `app/presets.json`
  of the page.
- **The CSS.** The page has 20 rules for `:is(#fix-panel, #design-panel)`. The fixes editor takes them as `#fix-panel` rules. The page
  keeps them for `#design-panel` until the design editor moves to roadstyle (another session does that move now).

The returned object does not change: `text`, `load`, `restore`, `open`, `close`, `isOpen`, `rebuilt`, `select`, `tab`, `drawn`,
`count`.

What the panel takes from the host page, and does not own:

- **Classes:** `btn`, `btn primary`, `row`, `why`, `small`, `muted`, `grid2`.
- **CSS variables:** `--bg`, `--fg`, `--muted`, `--mut`, `--line`, `--surface`, `--accent`.
- **The class `editing-fixes` on `<body>`.** The panel sets it while it is open. The host page uses it to hide its other parts.
- **roadstyle's map:** the event `rs:select` and `window.rsDeselect`.
- **The tables.** The panel reads `driving.edges`, `driving.nodes`, `features.buildings` and `features.traffic` (duckOSM), and
  `main.building_faces`, `main.building_entrances` and `main.building_info` (the page makes them). If a table is not there, the panel
  turns off the part that needs it (the walls, the doors).

## How the page loads the editor at a duckOSM rev

The page takes the engine from duckOSM by git rev in `crates/duckosm-wasm/Cargo.toml`. `build.sh` reads that rev, fetches duckOSM at
the rev (`git fetch --depth 1`, as it fetches roadstyle at `ROADSTYLE_COMMIT`), and copies `web/fixes_editor/` to
`www/app/fixes_editor/`. The copy is not in git. One rev then gives the engine and the editor. The page has no second pin that can
get out of step.

The main session makes this change in duckosm-rust after the duckOSM PR merges. This session does not change duckosm-rust.

## The fixes file in `duckosm build`

The check: one fixes file from the page with every fix kind, on the sample of the page (`tests/data/monaco_sample.osm.pbf` of
duckosm-rust, with `monaco.geojson`). The page builds Monaco without and with the file. `duckosm build` (the Rust engine) does the same.
For each table, a digest compares the two: the row count and the md5 of the sorted rows, all columns but the geometry, on the columns that
the two tables have. The check script picks the objects of the file from the tables of the page:

- `overrides`: road `503462470` gets `lanes: 3` and `oneway: true`; building way `25722685` gets `remove: true`.
- `new_ways`: one service road from node `21911886` through a new point to node `21914340`.
- `turn_restrictions`: `no_left_turn` from way `4097656` through node `21912089` to way `35092475`.
- `extra.tag_edits`: road `503462470` gets `sidewalk` and `surface`; building way `23715051` gets `building:levels: 9` and loses
  `roof:shape`; node `21912089` gets `highway: traffic_signals`; relation `393226` loses `building`.
- `extra.building_faces`: one wall of building `23715051` gets the type `back`.

Results (43 tables that the two builds have):

| Build | Without the file | With the file |
|---|---|---|
| `duckosm build`, Rust engine, duckOSM main | equal | **6 tables different**: `driving.ways`, `walking.ways`, `cycling.ways`, `features.buildings`, `features.streets`, `features.traffic` |
| `duckosm build`, Rust engine, with fix 1 (below) | equal | equal |
| `duckosm build --engine python`, with fix 1 | equal | equal |

"Equal" is for every column of all rows, the network tables (`edges`, `nodes`, `edge_graph`, `turn_restrictions`,
`edge_id_map`, …) too. Two notes:

- `main.boundary` is equal column by column. Its row digest is different only because the two tables have the columns in a different
  order.
- Only the page has `main.building_faces`, `main.building_entrances` and `main.building_info`. Only `duckosm build` has
  `features.ocean`.

The six different tables come from one cause: `raw.*` of `duckosm build` has no tag edit. With the file, `features.traffic` has one row
fewer than on the page (the new signal), and `features.buildings` has one row more (the relation that loses `building`).

### What `duckosm build` did with each fix kind (duckOSM main, before fix 1 and fix 2)

| Fix kind | Rust engine (the default) | Python engine (`--engine python`) |
|---|---|---|
| `overrides` (`oneway`, `lanes*`, `layer`, `exclude_modes`, `turn_lanes_*`) | yes | yes (`OsmOverrides`) |
| `overrides` (`remove`, `keep_nodes`, `tags`) | yes | yes (`_raw_way_fixes`) |
| `new_ways` | yes | yes (`_raw_way_fixes`) |
| `turn_restrictions` | yes | yes (`RestrictionProcessor`) |
| `extra.tag_edits` | networks: yes. `raw.*` and the tables made from it: **no** | **no** |
| `extra.building_faces` | no table (`main.building_faces` is not made); after fix 2, with `options.building_faces` | no table |

### Fix 1: tag edits in `raw.*`

`_raw_way_fixes` reads only `overrides` and `new_ways`. So `raw.*` has no tag edit, and every table that Python makes from `raw.*` is
different from the page: `features.*` (a node that becomes a traffic signal is not in `features.traffic`, a building keeps a deleted
tag), the bus routes and the lane profile. With the Python engine, the networks also do not see the tag edits.

The change: `_raw_way_fixes` applies `extra.tag_edits` to `raw.ways`, `raw.nodes` and `raw.relations`, after the `tags` of the
`overrides` rules and before `new_ways`. This is the order of `Raw::apply_fixes` (`raw.rs`): set the tags, then delete the keys. One SQL
`UPDATE` for each edit (7 lines in `importer.py`; the check above ran with it). Then the two engines see the same tags, and the Python
engine supports tag edits too.

A test in `tests/test_osm_overrides.py`: on Monaco, a node edit adds `highway=traffic_signals` and the node is in `features.traffic`, a way
edit deletes a tag, and the walking edges of a road with a new `sidewalk` tag get `walk_type = sidewalk` (the same road as
`rust/duckosm-core/tests/tag_edits.rs`).

### Fix 2: the walls, `options.building_faces` (choice 1 of Kaveh)

`main.building_faces` (the walls of the buildings, each with a type) was made only by duckosm-build, when the page asks for it. `duckosm
build` did not make the table, so a wall rule of the page had nothing to change. The choices were (1) a new option, or (2) wall rules only on
the page. Kaveh chose 1.

- **The option** `options.building_faces` (default off). `duckosm._rust.build()` has the keyword `building_faces`. With the option on,
  `rust_engine.build_parquet` also gives the module the boundary, because the walls need `raw.*` of the area. The module then cuts the cut
  PBF again: the cut of a cut gives the same ways. `_rust_build` loads `building_faces_main.parquet` into `main.building_faces`. With the
  option off, nothing changes.
- **The component filter.** The walls face the street edges of the driving and walking networks. Without more, the Rust step makes the
  walls before Python's `ComponentFilter`, so 99 walls of Monaco faced a street piece that the filter removed later. The page filters in
  Rust. With the option on, the Rust step also keeps only the largest component (`clip.keep_largest_component`), as on the page.
- **Errors.** `config.validate()` stops the build when the option is on and the engine is `python`, the source is a `duckdb` build, or
  there is no boundary.

`main.building_entrances` and `main.building_info` are not fix kinds: the page makes them from `features.buildings` and `raw.*` for the
panel. They stay in the page.

## The box of a picked object (2026-11-05)

Kaveh's design. When the person picks an object that has records in the changes, the panel shows a box over the form, and the form
waits: **This road is in your changes:** lanes 2, name "B". **Edit it** opens the form with the values of the records. **Delete it**
removes all the records of the object, and the form opens with its OSM data. Undo brings them back.

| Kind | The records |
|---|---|
| Road | `overrides` by `osm_id`, and the tag edit of the way |
| Building | the tag edit of the way or relation (and `overrides` for a delete), and `extra.building_faces` of its walls |
| Node | the tag edit of the node |
| Turn | `turn_restrictions` of the picked road at the picked junction (by `from_way`, `via_node`, `to_way`) |
| New road | `new_ways` by `id`, when it opens from the Changes tab |

The walls are in the box of their building: a pick of a wall in the list does not stop the form a second time. The box keeps the
person's answer while the same object stays picked (a build again, Load of the same ID).

**Two or more records of one object.** Before this change, `parse()` kept every record, `findRule` took the first, and `upsertRule`
replaced the first. An edit could then go under a later record, because the build applies the records in file order. Now `fixes.js`
reads the records joined as the build applies them, and each write puts one record in the place of the first one:

- `overrides`: each key in file order (the later wins); `remove` and `exclude_modes` of every record; `tags` key by key.
- Tag edits: for each record, set the tags, then delete the keys. `was` keeps the value before the first change.
- Walls: the build keeps the last record of a wall (`building_faces.rs`, a map by wall), all of it: a later record with no `door` has no door.
- Turns: the form has one rule for each turn: the later record. The build applies every record; two different rules for one turn
  have no meaning, so the join takes the later one.
- New roads: the build stops when two new roads have one ID. The join takes each key in file order.

The box says how many records there are ("This road has 2 rules in your changes. Edit joins them into one: for the same key, the later
rule wins, as in the build."), and the Changes tab marks the object ("2 rules"). `fixes_units.mjs` checks the joins and the delete of all.

**The hide button.** The panel has no hide button. The back button of the frame closes it. A host control that hides the panel must
call `close()` first (README.md).

## The editor frame and the highlight sets (2026-11-05)

Kaveh's decisions. The editor had its own shell: a 3 × 2 grid of grey tabs, a body that scrolls, and a white bar that sticks to the
bottom of each tab. The roads had the editor's own line layers (`fix-sel-*`, `fix-turn-*`, `fix-edits-*`) with a fixed width. On the map
they were wider or thinner than the road, and they had dark blobs at the joins. roadstyle now has both parts (roadstyle PR #49,
`e1b0617a`): the editor frame `rs.frame` and the highlight sets.

- **The frame.** `initFixesPanel(panel, ctx)` takes the host's dashboard panel (`#sb`) and makes `rs.frame({panel, title: "Edit OSM
  data", back, id: "fix", tabs})`. The ids of the tab buttons do not change (`fix-tab-road`, ...). The six tabs are in the frame's head
  row. The panes are in the frame's body. The bars of the four kinds of tab (`fix-actions`, `fix-turn-bar`, `fix-new-bar`,
  `fix-build-bar`) are in the frame's one bar, and `paintBars()` shows the bar of the tab that shows. The bar also waits while the box
  "This road is in your changes" asks. A CSS rule hides the frame's bar when all of its children are hidden. The frame's root has the
  id `fix-panel`, so the CSS of the editor did not change its scope. The editor finds its elements in the frame (`$`), because the
  frame is out of the page while it is closed.
- **open, close, back.** `open()`, `close()`, `isOpen()` and `tab()` follow the frame. The back button closes the editor and calls
  `ctx.onBack`. The class `editing-fixes` on `<body>` stays.
- **The highlight sets.** A new host hook, `roadRows(osmId)`, gives the map's rows of an OSM way. With it, the roads go into five sets
  (`ROAD_SETS`, in one place): `fix-edits` amber, `fix-gone` red, `fix-turn-out` green, `fix-turn-hover` dark gray, `fix-sel` blue. A
  road in two sets has the colour of the later set in roadstyle. A change of one set sets the later sets again, so the order of
  `ROAD_SETS` is always the order in roadstyle. `roadsLit()` takes the lines of the roads that the map has out of the features of a
  layer. Then the layer keeps the buildings, the nodes, the junctions and a new road. Without `roadRows`, the editor uses its old line
  layers.
- **duckosm edit.** The page gives `dashboard`, `roadRows` (an index of `RS_SIDE.props` by `osm_id`, made one time) and
  `backLabel: "Map"`. The name of the fixes file is in the frame's title line, and the status line is in the frame's bar. A card of the
  dashboard's panel, "Edit OSM data", opens the editor again after the back button. Its note is the summary of the changes.

Two faults that the move showed:

- **The server's JSON columns.** The server of `duckosm edit` gives the rows as JSON (DuckDB's `to_json`). A JSON column, for example
  `ST_AsGeoJSON(geometry)`, came as an object, but the editor parses text, as in DuckDB-Wasm's Arrow table. Thus no object of the editor
  showed on the map of `duckosm edit`: the selected road, the changes, the walls. `duck.query` of the page now gives such a column as
  text.
- **roadstyle's Layers box on a desktop.** On a docked dashboard with no toolbar, roadstyle puts the Layers box of the base map in
  the stack at the top left (`overlays.js`, `_tlHost`). The stack is over the panel and covers the back button. A click on the button
  hits the box. This is a fault of roadstyle. The page does not change the template.

## What the Python engine (`--engine python`) does not support

- `extra.building_faces`: the Python engine has no `building_faces` step. With `options.building_faces`, the build stops with an error.
- Everything else of the fixes file acts in the Python engine after fix 1.

## What was done

- `web/fixes_editor/`: the five files and `README.md`. One change more than the API above: `ctx.changesNote`, an optional HTML text
  for the help of the Changes tab. The page had a sentence about its step 4 there; the editor has a neutral sentence, and the page
  can add its own sentence.
- The CI job `test` runs `node web/fixes_editor/fixes_units.mjs`.
- Fix 1 (`_raw_way_fixes`), with a unit test in `tests/test_osm_overrides.py`.
- Fix 2 (`options.building_faces`), with the test of the error in `tests/test_osm_overrides.py`.
- `tests/test_page_fixes.py`: the check of this note as a test. `tests/data/page_fixes/osm_fixes.yaml` is the fixes file.
  `page_tables.json` has the digests of the page's tables (`tests/page_tables.py`), from `scripts/page_tables.py` (the page at
  duckosm-rust `062cda8`). The test builds Monaco with the file and `options.building_faces`. All 44 tables that the two builds have
  are equal in all columns. The page has one column more in `main.boundary` (`geofabrik`); there the test compares each column.
- The guide `docs/guides/edit-osm-data.md`, and `main.building_faces` in `docs/reference/database.md`.
- A check of the moved panel in the page: a copy of the duckosm-rust page (outside the repository) with the new API, and the page's own
  `fixes_e2e.py` on it.
