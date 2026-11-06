# The fixes editor

The panel "Edit OSM data" of [duckOSM Studio](https://github.com/Khoshkhah/duckosm-studio) and of `duckosm edit`, as a JavaScript module for a host page. It writes a duckOSM fixes file:
`overrides`, `new_ways`, `turn_restrictions`, `extra.tag_edits` and `extra.building_faces`. `duckosm build --fixes` reads the file.
The user guide is [Edit OSM data](../../docs/guides/edit-osm-data.md). The design is [docs/design/fixes_editor.md](../../docs/design/fixes_editor.md).

| File | What it is |
|---|---|
| `fixes.js` | The rules as data: read and write the fixes file. No DOM, no map. |
| `fixes_panel.js` | The panel in roadstyle's editor frame: its markup, the six tabs, the highlight sets and the map layers, undo and redo. |
| `fixes_panel.css` | The look of the panel. Every rule starts with `#fix-panel`, the root of the frame. |
| `presets.json` | The fields of each tab, and the guards between them. |
| `fixes_units.mjs` | The unit tests: `node web/fixes_editor/fixes_units.mjs` (the CI job `test` runs them). |

## Use it in a page

The host page is roadstyle's standard dashboard (roadstyle `e1b0617a` or later): the editor uses roadstyle's editor frame `rs.frame` and the
highlight sets (`rsHighlight(rows, {set, color})`). Copy the folder at one duckOSM rev, link `fixes_panel.css`, load
[js-yaml](https://github.com/nodeca/js-yaml) and call `initFixesPanel`:

```js
import { initFixesPanel } from "./fixes_editor/fixes_panel.js";

const panel = await initFixesPanel(document.getElementById("sb"), {   // the dashboard's panel: the editor's frame goes into it
	duck,          // the database: query(sql) gives an Arrow table, tables is [{mode, table}], loadSpatial()
	map,           // the MapLibre map
	dashboard,     // roadstyle's dashboard (window.rs): the editor uses rs.frame. Default: window.rs
	yaml,          // js-yaml
	status,        // status(text, ms): a short message for the user
	roadIdOfRow,   // the OSM way id of a road that roadstyle's map selects (event rs:select)
	roadRows,      // (osmId) gives the rows of the map's roads of that OSM way: the editor lights them with the highlight sets
	areaKey,       // () gives the name of the area: the browser keeps the changes of each area
	onChange,      // (summary, n): the number of changes is new
	onBuild,       // (): the user clicks "Build again"; the host page builds with panel.text()
	onBack,        // optional: (): the user clicked the back button, and the editor is closed. The host shows its own panel
	backLabel,     // optional: the text of the back button ("← Steps"). Default: "Back"
	buildOff,      // optional: () gives why the host cannot build now ("" when it can); Build again is then off, with this reason under it
	changesNote,   // optional: more HTML for the help text of the Changes tab
});
```

The panel returns `text()` (the fixes file, or `null` when there is no change), `load(text)`, `restore()`, `open()`, `close()`,
`isOpen()`, `rebuilt()` (call it after a build), `select()`, `tab()`, `drawn()`, `count()` and `frame` (the editor's frame).

### The frame

`open()` opens the frame in the dashboard's panel: the title line ("← back" and "Edit OSM data"), the six tabs in the head row, the
form of the tab in the body (the one area that scrolls) and the buttons and the state of the tab in the bar at the bottom edge. The bar
changes with the tab, and an empty bar does not show. On a phone the sheet opens to its full height. `close()` removes the frame and
shows the host's panel again. The back button closes the editor, then calls `onBack`.

The frame's root has the id `fix-panel`, and every rule of `fixes_panel.css` starts with it. The host can add to the frame: to
`panel.frame.top` (the title line, for example the name of the file) and to `panel.frame.bar` (for example a status line).
The panel sets the class `editing-fixes` on `<body>` while it is open. A control of the host that hides the panel must call `close()`
first. Then nothing of the editor stays selected on the map while the panel is hidden.

### The roads on the map

The editor lights the roads of the map with roadstyle's highlight sets, in the colour on the road's own fill and at the road's width
at every zoom. The set names are in one place, `ROAD_SETS` of `fixes_panel.js`. A road in two sets has the colour of the later set:

| Set | Colour | The roads |
|---|---|---|
| `fix-edits` | amber | the roads in the changes |
| `fix-gone` | red | the roads that the changes delete |
| `fix-turn-out` | green | the roads that leave the junction of the Turn tab |
| `fix-turn-hover` | dark gray | the road under the pointer (or the picked road) in the list of the Turn tab |
| `fix-sel` | blue | the picked road |

The editor's own layers show the buildings, the nodes, the junctions, the walls, the doors and a new road (also while you draw it).
A road that the map does not have (`roadRows` gives no rows) is on the editor's own layers too.

**A host that has not moved yet.** With no `roadRows`, the editor draws the roads on its own line layers (`fix-sel`, `fix-turn`,
`fix-edits`), as before. With no `rs.frame` in the dashboard, `initFixesPanel` stops with an error: the host must update roadstyle.

### What the host page gives

- The classes `btn`, `btn primary`, `row`, `why`, `small`, `muted` and `grid2`, and the CSS variables `--bg`, `--fg`, `--muted`, `--mut`,
  `--line`, `--surface` and `--accent`.
- roadstyle's map: the event `rs:select`, `window.rsDeselect` and `window.rsHighlight`.
- `duck.query`: an Arrow table, as DuckDB-Wasm gives it. A JSON column (`ST_AsGeoJSON`) is text in it.
- The tables `driving.edges`, `driving.nodes`, `features.buildings` and `features.traffic`. With `main.building_faces` (option
  `building_faces`), the Building tab shows the walls. With `main.building_entrances`, it shows the doors. If a table is not there,
  the panel turns off the part that needs it.

## An object that is in the changes

When the person picks an object that has records in the changes, the panel shows a box over the form, and the form waits:
**This road is in your changes:** lanes 2, name "B". **Edit it** opens the form with the values of the records. **Delete it** removes
the records, and the form opens with the OSM data. Undo brings them back. The box is for each kind: a road, a building (with the
rules of its walls), a node, the turn rules of a road at a junction, and a new road that opens from the Changes tab.

A file can have two or more records for one object. The build applies them in file order, so for the same key the later record wins.
The panel reads the records joined in the same way (`fixes.js`: `findRule`, `findTagEdit`, `findFace`, `findTurn`, `findNewWay`), and
a write puts one record in the place of all of them. The box then says how many there are, and Edit joins them into one. The Changes
tab marks such an object with "2 rules".
