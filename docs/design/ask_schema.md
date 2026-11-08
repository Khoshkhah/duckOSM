# What Ask knows about the tables: one module for both pages

Status: built (2026-11-08). Kaveh signed off the design before the code. Kaveh asked: "use duckosm and roadstyle like a library, so if they change, your result changes easily; especially for Ask, editing OSM data,
editing level, editing design". This is step 1, for Ask. The editing of OSM data is already one folder (`web/fixes_editor/`). The questions of Ask are already one file
(`web/ask/questions.js`, PR 40).

## The problem

Two pages give roadstyle's Ask and data dock the same knowledge about the duckOSM tables, and each page writes it by itself:

| Knowledge | `duckosm ask` (`rust/duckosm-edit/page/ask.js`) | duckOSM Studio (`www/index.html`, `www/app/duck.js`) |
|---|---|---|
| A note for each table (the Tables list of the dock) | `ABOUT`, 10 tables | `ABOUT`, 5 tables; and `TABLES` in `duck.js`, 25 tables, for the download list |
| The text for the agent (`about:`) | 15 lines: networks, `edge_id`, `main.building_faces`, `extra.edits`, geometry, H3, colours, road classes | 3 lines: networks, `edge_id`, `osm_id` |
| Which roads a result means (`roadsBy`) | `SELECT DISTINCT edge_id FROM <main> WHERE col IN (...)` | the same, for `driving.edges` |
| The first SQL line of the dock | classes, edges and km of `<main>` | the same, for `driving.edges` |
| The columns of the main table, for the agent | one query on `information_schema` | the same query |

When a table changes in duckOSM (a new column, a new table), someone must change both pages. Studio is behind already: its agent does not know `building_faces`,
the road classes or the H3 cells.

## The decision

One module, `web/ask/schema.js`, next to `web/ask/questions.js`. It has no DOM, no `fetch` and no DuckDB. It takes the tables and a `query` function as arguments, so the same
code runs on the server of `duckosm ask` (through `POST /api/sql`) and on DuckDB-wasm in Studio. The page and Studio differ only where the data access differs.

```js
export const TABLE_NOTES            // { edges: "...", building_faces: "...", ... }: the notes of all the tables, one list
export const mainEdges(names)       // "driving.edges" (else the first "<mode>.edges", else null), from the names of the tables
export const roadsBy(main)          // returns (sql, column) => SQL for dataDock({ roadsBy })
export const startSql(main)         // the first SQL line of the data dock
export async function describeTables(tables, main, query)   // adds `about` to each table and the columns to the main one: the result of dataDock({ schema })
export async function roadClasses(query, names)             // "driving.edges: residential (919), service (781), ..." (the real values of highway)
export function aboutText({ tables, classes, shapes })      // the text for askPanel({ about })
```

- `tables` is a list of table names (`"driving.edges"`, `"main.building_faces"`, `"extra.edits"`).
- `aboutText` tells the agent about what exists. It always has the networks, `edges`, `edge_id` and `osm_id`. It has the paragraph of a table only when the table is in the list: the walls
  for `main.building_faces`, the changes for `extra.edits`, the H3 cells for `extra.boundary_h3` and the nodes. With `classes`, it gives the real road classes, so the agent does
  not guess them. With `shapes: true` (a page that draws the geometry of a result, as `duckosm ask` does), it has the paragraphs about geometry columns and colours.
- `TABLE_NOTES` joins the three lists above. The notes are short and plain. Studio's `TABLES` in `duck.js` can read it too.

## What changes in `duckosm ask`

`page/ask.js` imports `./ask/schema.js` (the same route as `questions.js`: the server has the files of `web/ask/` in the program and gives them at `/ask/<file>`) and removes its own `ABOUT`, `aboutText`,
`roadsBy`, the start SQL, the class lookup and the columns query. What stays in `ask.js` is what only this page has: the server `query`, the table `extra.edits`, the H3 cells, the
shapes on the map, the card of a row and the tool `set_shapes`. `web/ask/*` is in the sdist already.

## What Studio does after this is merged (not part of this change)

It deletes its `ABOUT`, its short `about:` and its `roadsBy`, imports `./duckosm/ask/schema.js` (`build.sh` copies `web/ask/`) and gives `duck.tables` as the list of tables.
Its agent then knows the walls and the road classes.

## Tests

- A unit test with Node on `schema.js`: the notes cover every table that a Monaco build has; `aboutText` names a table only when it is in the list; `mainEdges` and `roadsBy`
  give the right text for one, two and no network; `describeTables` adds the columns of the main table. `tests/test_ask_schema.py` runs it with the table names of a Monaco build (it skips without `node`).
- The existing tests of the page (`tests/test_ask_page.py`) keep passing: the real classes are in the text of the agent (`window.askPage.about`).
- A Rust test for the route `/ask/schema.js`, as for `questions.js`.

## How the page gives the tables

The page makes `extra.edits` (always) and `extra.boundary_h3` (only when h3-js loads) after the text for the agent is made, so it adds these two names to the list that it gives to `aboutText`.

## Not in this change

- Roadstyle's Ask puts the `about` note of a table into the prompt only for the tables of `extra.`. The notes of the other tables show in the Tables list of the dock only. So the agent learns
  about the other tables through `aboutText`. A change in roadstyle (the notes of all tables in the prompt) would make `aboutText` shorter. That is for roadstyle, not here.
- The steps 2 to 4 of Kaveh's plan (editing levels, editing design) are other tasks.
