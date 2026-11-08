# Tests on hosts that have the shape of Studio

## Problem

The editor (`web/fixes_editor/`) and Ask (`web/ask/`) are libraries for hosts (`AGENTS.md`, "Hosts of the editor and Ask"). A host should only move its version. The tests ran on
one host, `duckosm edit`. Studio differs from it in three ways, and each difference gave a bug that the tests did not see:

| Difference | Bug that came from it |
|---|---|
| The overlays of the map are vector-tile layers on a shared source (`rs-roads`), not GeoJSON sources with the name of the overlay | The hover and select state of a building was never set (roadstyle PR 100 and 102) |
| The rows of `window.RS_SIDE.props` are arrays, not objects | The editor could not find the OSM way of a row (`roadKeys`, duckOSM PR 43) |
| A saved area has the `features.*` tables without the `tags` column | "This ID is not in this area" (duckOSM PR 48) |

## What the tests do now

`tests/test_editor_host_shapes.py` and `tests/test_edit_page.py` run the real editor page of `duckosm edit` and change one thing of the host at a time:

- **Array rows.** The page is changed in the browser before the editor starts: the rows of `RS_SIDE.props` are arrays, `roadKeys` is given, `rsGetProps` is removed, and the host gives no
  `roadIdOfRow` and no `roadRows`. A click on a road must pick the right OSM way and light its rows.
- **No `tags` column.** A copy of the Monaco test database has every `features.*` table without `tags` (`restored_url` in `tests/editor_fixtures.py`). A picked building and node must show, with a note.

## Not covered, and who covers it

- **Vector-tile overlays on the shared source `rs-roads`.** A page of this shape needs roadstyle's vector-tile build. roadstyle has the browser test for it
  (`test_overlay_vector_state_browser.py`). duckOSM gets the page of `duckosm edit`, which has GeoJSON overlays. If roadstyle can make a vector-tile page in a test, a third test goes here.
- **DuckDB-wasm.** Studio reads the data in the browser. The editor needs only the interface `duck.query(sql)`, `duck.tables` and `duck.loadSpatial()` (`web/fixes_editor/README.md`),
  and `duckosm edit` gives the same interface over `POST /api/query`. A real DuckDB-wasm in the test is not planned.

## The rule

A change of the host contract (an option of `initFixesPanel`, the tables or the columns that the editor reads) gets a test here, on the shape that it concerns.
