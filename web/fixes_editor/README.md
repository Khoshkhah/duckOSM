# The fixes editor

The panel "Edit OSM data" of the duckosm-rust page, as a JavaScript module for a host page. It writes a duckOSM fixes file:
`overrides`, `new_ways`, `turn_restrictions`, `extra.tag_edits` and `extra.building_faces`. `duckosm build --fixes` reads the file.
The user guide is [Edit OSM data](../../docs/guides/edit-osm-data.md). The design is [docs/design/fixes_editor.md](../../docs/design/fixes_editor.md).

| File | What it is |
|---|---|
| `fixes.js` | The rules as data: read and write the fixes file. No DOM, no map. |
| `fixes_panel.js` | The panel: its markup, the six tabs, the map layers, undo and redo. |
| `fixes_panel.css` | The look of the panel. Every rule starts with `#fix-panel`. |
| `presets.json` | The fields of each tab, and the guards between them. |
| `fixes_units.mjs` | The unit tests: `node web/fixes_editor/fixes_units.mjs` (the CI job `test` runs them). |

## Use it in a page

Copy the folder at one duckOSM rev, link `fixes_panel.css`, load [js-yaml](https://github.com/nodeca/js-yaml) and call `initFixesPanel`:

```js
import { initFixesPanel } from "./fixes_editor/fixes_panel.js";

const panel = await initFixesPanel(document.getElementById("fix-panel"), {
	duck,          // the database: query(sql) gives an Arrow table, tables is [{mode, table}], loadSpatial()
	map,           // the MapLibre map
	yaml,          // js-yaml
	status,        // status(text, ms): a short message for the user
	roadIdOfRow,   // the OSM way id of a road that roadstyle's map selects (event rs:select)
	areaKey,       // () gives the name of the area: the browser keeps the changes of each area
	onChange,      // (summary, n): the number of changes is new
	onBuild,       // (): the user clicks "Build again"; the host page builds with panel.text()
	changesNote,   // optional: more HTML for the help text of the Changes tab
});
```

The element must have the id `fix-panel`. The panel adds its tabs and its body to the element, after the header of the host page.
The panel returns `text()` (the fixes file, or `null` when there is no change), `load(text)`, `restore()`, `open()`, `close()`,
`isOpen()`, `rebuilt()` (call it after a build), `select()`, `tab()`, `drawn()` and `count()`.

The host page gives:

- The classes `btn`, `btn primary`, `row`, `why`, `small`, `muted` and `grid2`, and the CSS variables `--bg`, `--fg`, `--muted`, `--mut`,
  `--line`, `--surface` and `--accent`.
- The header of the panel, and the show and hide of the panel: the panel sets the class `editing-fixes` on `<body>` while it is open.
- roadstyle's map: the event `rs:select` and `window.rsDeselect`.
- The tables `driving.edges`, `driving.nodes`, `features.buildings` and `features.traffic`. With `main.building_faces` (option
  `building_faces`), the Building tab shows the walls. With `main.building_entrances`, it shows the doors. If a table is not there,
  the panel turns off the part that needs it.
