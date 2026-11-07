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

Roads and buildings are clickable. A road shows its card beside the table. The card is for roads only, so a click on a building (roadstyle sends
`rs:select` with `overlay: "buildings"`) runs a query in the data dock: the row of the building with its tags, the number of its walls and of the walls
that were edited, and its changes from `extra.edits`. There is no popup for any item: the page turns every overlay popup off and holds the road popup.

**Asked of roadstyle (2026-11-07, for PR 69):** the data dock's card should also draw the facts of an overlay item, as it does for a road. It skips an
overlay click now, and a host has no way to put content in the card. Kaveh wants the facts of a clicked item in that card. When roadstyle has it, the
page moves the pin and the building has its facts in the card too. The row in the table stays: it has the walls and the changes, which the card does not.

## What the host gives

roadstyle knows no database. The host page (`rust/duckosm-edit/page/ask.js`) gives the data dock three things:

- `query(sql)`: the server runs the statement on the database, read only (`POST /api/sql`). The answer has the columns with their types, and
  the rows as lists. A 64-bit integer (an `edge_id`) goes as a text and becomes a `BigInt` in the page, because a JSON number has 53 bits.
- `schema()`: the tables of the file, each with a line about what it holds, and the columns of the main table.
- `roadsBy(sql, column)`: how to find the roads of a result that has `osm_id` and no `edge_id`.

## Shapes on the map

roadstyle's Ask lights roads only. Asked "show all nodes" or "show all buildings", it answered "I can only show roads". So the page draws the
geometry of a result itself, with one GeoJSON source and three layers (fill, line, circle) in roadstyle's select purple:

- A result with a column of the type `GEOMETRY` is drawn. The page runs the same SQL again, for that column only, with `ST_AsGeoJSON` and no limit.
  The limit of 1000 is for the rows that the table shows, not for the selection: the table has the first 1000 rows and the map has all of them, so
  "show all buildings" draws every building. There is no cap: a result of millions of shapes is slow in the browser.
- A new table with no geometry clears the shapes, as the light of roadstyle follows the table.
- The camera moves to the shapes after an answer of Ask, not after the person's own SQL.
- The `about` text tells the agent that it can show nodes, buildings and walls, with which columns, and to add no `LIMIT` for "all".

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
