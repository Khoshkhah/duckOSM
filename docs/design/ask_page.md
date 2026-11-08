# `duckosm ask`: ask questions about the OSM data and the edits

Status: built (2026-11-07). Kaveh asked for a tool that opens the full duckOSM map, with all buildings and roads clickable, and that uses
the Ask and SQL panels, so that you can ask any question about the edits and the OSM data. Kaveh then said: it is a small update,
because roadstyle has the parts already and duckOSM Studio uses them.

## What it is

```bash
duckosm ask monaco.duckdb        # then open http://127.0.0.1:8791/
```

The page is the same roadstyle dashboard as `duckosm edit`. It has two more parts, both from roadstyle, with no copy in duckOSM:

| Part | What it does |
|---|---|
| The data dock | One SQL line over a wide table under the map. The roads of the result light on the map. A click on a row selects its road. |
| Ask | A question in words. The agent writes SQL, runs it in the data dock, and answers in a few sentences. It can colour roads, focus on roads and draw a chart. It uses the person's own key of a free AI service, which stays in the browser. |

Roads and buildings are clickable: roadstyle selects the clicked item (a solid purple look). A road shows its card beside the table. The card is for
roads only, so a click on a building shows no facts yet. A first version ran a query for the building and put its row in the table; Kaveh wanted the
facts in the card, and the query replaced the table, so a table with no geometry cleared the shapes of the map. A click runs no query now. There is no popup for any item: the page turns every overlay popup off and holds the road popup.

**Asked of roadstyle (2026-11-07, for PR 69):** the data dock's card should also draw the facts of an overlay item, as it does for a road. It skips an
overlay click now, and a host has no way to put content in the card. Kaveh wants the facts of a clicked item in that card. When roadstyle has it, the
page moves the pin and the building has its facts in the card too. The walls and the changes of a building are not in the card: ask for them, or write a SQL line.

## What the host gives

roadstyle knows no database. The host page (`rust/duckosm-edit/page/ask.js`) gives the data dock three things:

- `query(sql)`: the server runs the statement on the database, read only (`POST /api/sql`). The answer has the columns with their types, and
  the rows as lists. A 64-bit integer (an `edge_id`) goes as a text and becomes a `BigInt` in the page, because a JSON number has 53 bits.
- `schema()`: the tables of the file, each with a line about what it holds, and the columns of the main table.
- `roadsBy(sql, column)`: how to find the roads of a result that has `osm_id` and no `edge_id`.

## What Ask knows about the tables

The notes of the tables, the text for the agent (`about`), `roadsBy`, the first SQL line and the columns of the main table are in `web/ask/schema.js`, not in `ask.js`. The page and
duckOSM Studio import the same file. The design is in [ask_schema.md](ask_schema.md).

## The example questions

The examples of Ask are in one file, `web/ask/questions.js` (`export const QUESTIONS`, groups of `{ title, items }`). The page imports it from `/ask/questions.js`,
where the server has it in the program, and passes it as `examples`. roadstyle's six general groups show first, so no title of the file is one of them.
Another host page can copy the file. `tests/test_ask_questions.py` checks the groups, and the guide `docs/guides/ask-questions.md` lists the same questions.

## Shapes on the map

roadstyle's Ask lights roads only. Asked "show all nodes" or "show all buildings", it answered "I can only show roads". So the page draws the
geometry of a result itself, with one GeoJSON source and three layers (fill, line, circle) in roadstyle's select purple:

- A result with a column of the type `GEOMETRY` is drawn. The page runs the same SQL again, for that column only, with `ST_AsGeoJSON` and no limit.
  The limit of 1000 is for the rows that the table shows, not for the selection: the table has the first 1000 rows and the map has all of them, so
  "show all buildings" draws every building. There is no cap: a result of millions of shapes is slow in the browser.
- The colour of a shape is its column `color` (a CSS colour; one that CSS does not know is left out), else its `face_type` in the colours of the editor's walls
  (`FACE_COLORS`), else roadstyle's purple. Asked "show the walls with a colour for each face_type", Ask answered that it had no tool for the colours of shapes
  (Kaveh, 2026-11-07); now it needs none, and the `about` text tells it about `face_type` and `color`.
- A row of the table (roadstyle's data dock: the event `row`, `data.card(...)` and the event `cardclose`, PR 81): a row with a geometry column is drawn in the select look
  (a source `ask-pick`, solid purple), the map zooms to it, and the card has the title (its `name`, else the table and its id), the table as the chip, all the values
  except the geometry, and its id with Copy id. `cardclose` (the close button, a new selection of roadstyle, a new result) removes the drawing. The map switch (On or Off) is for the table only: Off, the shapes of the result go, and the selected item stays on the map, with its card
  (Kaveh, 2026-11-07). A road row (the result has `edge_id`) is selected by roadstyle with its own card, and the page draws nothing. Kaveh, 2026-11-07: "each selecting from table must show that item on map also shows
  its info in card info in sql panel".
- A result with the column `edge_id` is lit by roadstyle on the road's own shape, so the page draws no shape for it. The geometry column of an edges table is the centre
  line of the road, and a thin line over the road shows the road twice (Kaveh, 2026-11-07: "it shows roads from table in sql in this shape").
- A new table with no geometry clears the shapes, as the light of roadstyle follows the table.
- The shapes follow the map switch of the data dock (On or Off, roadstyle PR 73): the event `light` {on}, and `data.light()`, are true while the switch is on and the
  dock is open. Off, or with the dock closed, the layers are hidden, and the map has its own colours only.
- The camera moves to the shapes after an answer of Ask, not after the person's own SQL.
- The `about` text tells the agent that it can show nodes, buildings and walls, with which columns, and to add no `LIMIT` for "all".

## H3 cells

Asked "show smaller h3 cell on map that contain whole boundary area", Ask answered that it had no information about H3 cells (Kaveh, 2026-11-07). The database has
H3 only in the `h3_cell` column of the nodes (resolution 8), `main.boundary_cells` is an optional build step that is not in every database, and DuckDB here has no H3
functions, so a question about the H3 cells of the boundary had no data. Now:

- The page loads h3-js (jsdelivr, a pinned version) and makes the table `extra.boundary_h3` (resolution 0 to 11, `h3_cell` as a hex text, `whole_boundary`): the cells of the
  vertices of `main.boundary`, and from resolution 5 the cells inside it. `whole_boundary` is true when all the vertices are in one cell. A cell is convex, so that cell holds the
  whole boundary; the smallest cell that holds it is the largest resolution with the flag. Without h3-js (no network) the table is not made.
- A result with no geometry column and a column `h3_cell`, `h3` or `h3_id` is drawn as hexagons, one for each distinct cell (BIGINT or hex text).
- The `about` text and an example question tell Ask about both.

## The opacity of the shapes, and the H3 row

Two requests of Kaveh (2026-11-07), sent by the Ask sessions:

- "h3-cell selection from table must show the cell on the map". A click on a row that has an H3 column (`h3_cell`, `h3`, `h3_id`; a hex text or a BIGINT) and no geometry draws that
  one cell in the select look (the source `ask-pick`), moves the map to it, and fills the card (the title is its name, else `H3 cell <hex>`, and all its values). A row with a
  geometry column (the nodes) is selected by its geometry as before.
- "increase transparency of h3 cells on map". Ask answered that it could not: its tool `set_map` changes roadstyle's layers, and the shapes of a result are the page's own layer.
  The page gives Ask its own tool through the `tools` option of roadstyle: `set_shapes {opacity, reset}`. It sets the fill, line and point opacity of the shapes (`applyOpacity`), a new
  result and a new base map keep it, and `reset: true` brings the default look back (the fill 0.4, the outline and the points 1). The selected item is not changed.

## The edits as a table

Ask must answer questions about the edits too. The page reads the fixes file, as the editor does, and gives the server its changes as
the table `extra.edits` (`POST /api/table`). `extra` is a small database file in the temp folder, attached next to the database, so the file of the database stays read only. (DuckDB does
not attach an in-memory database next to a read-only one.)

| Column | Value |
|---|---|
| `kind` | `object`, `wall`, `turn` or `new_road` |
| `type` | `way`, `node`, `relation` or `building` |
| `osm_id` | the OSM id of the object (the building, for a wall; the first way, for a turn) |
| `what` | the change in words, as the Changes tab shows it |
| `changed` | the time of the latest record ([the time of each change](fixes_editor_time_merge.md)); NULL for a record with no time |
| `note` | the note of the record |

A change that the build has applied is in the tables of the database too (`main.building_faces` has `computed_type` and `edited`). So a question
can compare the file with the database.

## The server

`duckosm ask` is the server of `duckosm edit` with `Mode::Ask`. These parts are new:

- `GET /roadstyle/<file>`: roadstyle's dashboard modules `data/data_dock.js` and `ask/ask_panel.js`, with the small modules that they import
  (`frame.js`, `selectbox.js`, `panelscroll.js`, `data/guard.js`, `ask/providers.js`). None of them keeps state: they use `window.rs`. Only `.js`
  files of the dashboard, read only.
- `POST /api/sql` and `POST /api/table`, with the rules of `/api/query`: the page's own `Origin`, a `Content-Length`, at most 16 MB.
  `/api/table` takes names of letters, digits and `_`, types from a short list, and writes the values as SQL literals on the server, so the body is data.
- `POST /api/build` is refused. `duckosm ask` does not change the data.

## Not in this update

- `duckosm-rust ask` (the native program). The Python command `duckosm ask` has it.
- A table for the OSM tags of a changed object before the change (`was`). The text in `what` has it.
- Ask on a changed database that has not been built again. Ask sees the file (`extra.edits`) and the built database, not a build of both.
