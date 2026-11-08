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

## Object kinds: which column of a result row is a node or a building (built, 2026-11-08; Kaveh signed off the note)

Kaveh: a node picked in the SQL table does not show on the map, and its facts are not in the card. The data dock of roadstyle maps a row to a road only (`edge_id`). The dock's part is
roadstyle's (a design for the kinds `node` and `building`). The knowledge of the tables is duckOSM's, and it goes in `web/ask/schema.js` as data and SQL text, with no code in the hosts.
Checked on Monaco:

| Kind | Id column of a result | Geometry and facts come from | Note |
|---|---|---|---|
| road (exists) | `edge_id`; `osm_id` of an edge | `<mode>.edges` | `roadsBy` |
| node | `node_id`, `source`, `target`, `via_node` | `driving.nodes` (`node_id`, `geom`, `h3_cell`), else the first `<mode>.nodes` | a negative `node_id` is a virtual node and has a `geom` |
| node (point of the base map) | `osm_id` with `osm_type = 'node'` | `features.traffic` (`kind`, `name`, `tags`, `geom`) | 410 of 575 are also graph nodes. The rest are not in `nodes`, so this table has its own `geom` |
| building | `building_id` (walls); `osm_id` with `osm_type` of `way` or `relation` | `features.buildings` (`osm_id`, `osm_type`, `kind`, `name`, `tags`, `geom`) | `building_faces.building_id = features.buildings.osm_id` for all 10909 walls |

`osm_id` alone is not sure (a road, a building and a point have their own ids), so the SQL of a weak id column keeps `osm_type` and the dock takes the kind that gives rows. On Monaco no id is in two kinds.
`tags` is a column of the base-map tables only in a file that was saved after roadstyle PR 112; an older saved area has none. So the SQL leaves `tags` out when the host says `hasTags: false`.

Form (to fit the dock's hook when its design is written): `objectKinds({ names, hasTags })` returns `{ node: { idColumns, geometryBy }, building: { idColumns, geometryBy } }`. `geometryBy(sql, column)` returns
one SELECT, like `roadsBy`: the columns `id`, `geom`, then the facts of the card (`h3_cell`, `kind`, `name`, `tags`), for the ids that `SELECT column FROM (sql)` gives. The hosts pass it to the dock unchanged.

Host line, when the dock has the hook (the name of the option is roadstyle's): `kinds: objectKinds({ names, hasTags: await hasTags(query) })` from `web/ask/schema.js`. The hosts (`duckosm ask`, Studio)
write nothing else. Until then `objectKinds` is not called by any host.

## The base-map tables for Ask (built, 2026-11-08; Kaveh signed off the note)

Fault (Studio, model claude-haiku-5-5): "how many is traffic signals on the map?" got "I can't give a count yet". Studio's `schema()` drops every `features.*` table (`t.mode !== "features"`), so the
agent never sees `features.traffic`, and the text for the agent does not name it. The knowledge is duckOSM's. On Monaco: `features.traffic` has `kind` = `traffic_signals` (22) or `crossing` (553) and no
other kind, so there are no stop signs in it. Every `features.*` table has `osm_id`, `osm_type`, `kind` (the value of the main OSM tag: a `pois` row has `kind` = `bench`, `restaurant`, ...), `name`, `geom` and,
in a file saved after roadstyle PR 112, `tags` (a MAP). `kind` is always there, `tags` is not, so a hint uses `kind` first.

1. `askTables(tables)` takes the host's list (`{ mode, table, rows }` or `{ name, rows }`) and returns `[{ name: "schema.table", rows }]` with every table, the `features.*` tables included. It drops only `raw.*`.
   Studio calls it in `schema()` in place of its filter.
2. `aboutText` gets one line for each `features.*` table that is in the list: its columns and what its `kind` holds, from the table notes. With `hasTags: false` it says that there is no `tags` column and gives no hint that uses it.
3. Hints in `aboutText`, as SQL the agent can copy: the count of traffic signals (`SELECT count(*) FROM features.traffic WHERE kind = 'traffic_signals'`), the buildings by number of levels
   (`try_cast(tags['building:levels'] AS INTEGER)`, only with tags), and the points of interest by kind (`GROUP BY kind`). The hints are in the text only when their table is.
4. Test on a Monaco database: the SQL of every hint runs and gives the numbers that a second, independent query gives (22 signals, 179 buildings with more than 5 levels). No model is run.

Size: Monaco has 575 traffic points, 1262 buildings, 3496 streets and 1645 points of interest. A county has millions. Ask sends only the names and the row counts of the tables, never the rows, so size
costs nothing in the prompt. The text tells the agent to count or group in SQL and not to list `features.streets` or `features.buildings` whole. `tags` is a MAP, and a result with a MAP column is shown as JSON by the dock, so
the text says to select `tags['key']` and not `tags`.

Host line: Studio calls `describeTables(askTables(duck.tables), main, query)` in `schema()` in place of its `t.mode !== "features"` filter, and gives `aboutText({ tables, classes, hasTags: await hasTags(query) })`. `duckosm ask` already lists every table and now gives `hasTags`.
