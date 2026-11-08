# `duckosm edit`: edit the OSM data on your computer

**Status:** done 2026-11-05, with Kaveh's OK ("do it, finalize it, merge it").

## Problem

roadstyle's level editor works in two ways: on its own, with `roadstyle levels edit out/area` (a small Rust server,
`rust/roadstyle-serve`, on 127.0.0.1, that serves a host page of roadstyle's dashboard and saves `edits.csv`), and as step 8 of duckOSM
Studio. duckOSM's fixes editor (`web/fixes_editor/`, "Edit OSM data") had only the second. To edit the fixes of a database on your
computer, you had to open Studio, build the area again in the browser, and download the file.

## The command

```bash
duckosm edit <area>.duckdb [--fixes osm_fixes.yaml] [--port 8790]
duckosm-rust edit <area>.duckdb [--fixes osm_fixes.yaml] [--port 8790]
```

The two commands run the same Rust code (`rust/duckosm-edit`). `duckosm-rust edit` is a subcommand of duckOSM's Rust program.
`duckosm edit` calls `duckosm._rust.edit()`. There is no Python web server. The command prints the address of the page
(`http://127.0.0.1:8790/`) and runs until Ctrl+C.

- **The fixes file:** `--fixes`, else `osm_fixes.yaml` beside the database. If it does not exist, the page starts with no rules,
  and the first change makes the file.
- **The page:** the network of the database on roadstyle's map, roadstyle's standard dashboard panel, and the editor in the panel.

## The server

`rust/duckosm-edit/src/lib.rs`. It has the safety rules of roadstyle-serve:

- It binds 127.0.0.1 only.
- Each request must name the server in `Host` (`127.0.0.1:<port>` or `localhost:<port>`). This stops DNS rebinding.
- Each write (`PUT /api/fixes`, `POST /api/query`, `POST /api/build`) must come from the server's own page (`Origin`), and must have a
  `Content-Length`. A body is at most 16 MB. The request line and the headers are at most 64 KB (`httparse`).
- The page writes one file only: the fixes file. Its path comes from the command line, not from the page. The server copies the old
  file to `<fixes>.bak` before its first write.

| Route | Does |
|---|---|
| `GET /` | the page |
| `GET /fixes_editor/<file>` | the editor's files, in the program (`include_str!` of `web/fixes_editor/`) |
| `GET /api/info` | the area's name, the paths, the tables, and why Build again is off |
| `GET`, `PUT /api/fixes` | read and write the fixes file |
| `POST /api/query` | the rows of one query, as JSON |
| `POST /api/build` | Build again |

### Why not roadstyle-serve as a library

roadstyle-serve has one public function, `serve(area, page, port)`. It serves the level editor's files and an area folder, and it writes
only the files of its list `WRITABLE` (`edits.csv`, `levels.csv`, ...). Its request code (`answer`, `inside`, `reply`) is private. A fixes
file, a query route and a build route need other routes. So `duckosm-edit` has its own small server with the same rules and the same
tests. A change in roadstyle to make its server generic is a change in Kaveh's roadstyle session, which is busy with the editor frame.

What the page takes from roadstyle as a library is the page itself: `roadstyle::page_options::page_from` (the dashboard template, its
panel, its toolbox, its phone sheet) and `roadstyle::duckosm` (`roadstyle_io::duckosm`, the SQL that reads a duckOSM file: the roads of
every mode with their stored levels, and the base map's `features.*`). This is the Rust form of roadstyle's Python
`duckosm.page(db, dashboard=True)`. roadstyle is a Git dependency at a fixed `rev`, as `roadstyle-levels`.

## The editor's SQL: a query route with the database of the server

The editor needs SQL on the tables (`duck.query`, `duck.tables`, `duck.loadSpatial`). There were two ways:

1. duckdb-wasm in the page, which reads the tables that the server gives as Parquet files, as Studio does.
2. A query route on the server, with the database that is open in the server.

The choice is 2. The server has the database open already: it reads the roads and the layers for the page. The page then needs no
duckdb-wasm (about 30 MB) and no copy of the tables. Studio's `ctx` contract does not change: `duck.query(sql)` gives an object with
`toArray()`, and each row has `toJSON()`. The server sends each row as DuckDB's `to_json`. `duck.tables` comes from
`information_schema.tables`, and `loadSpatial()` does nothing (the server loaded the spatial extension).

The query route must not open a way to the files of the computer. After the server makes the page, it locks the database (`LOCK` in
`lib.rs`): `SET enable_external_access = false` (no `COPY`, `read_csv`, `ATTACH` or `INSTALL`) and `SET lock_configuration = true` (no
`SET` after it). The file is open read only. Each query runs inside `SELECT to_json(duckosm_row) FROM (<query>) duckosm_row`, so only a query runs.

### Two databases: native and Python

The Python module `duckosm._rust` has no DuckDB inside (`pyproject.toml`: `no-default-features`). The Python package brings its own
`duckdb`. So the server takes the database as a trait, `Db` (`open`, `close`, `execute`, `texts`, `build`):

- `duckosm-rust edit`: `native::NativeDb`, the bundled DuckDB 1.5.6 of `duckosm-duckdb`.
- `duckosm edit`: `src/duckosm/edit.py`, Python's `duckdb`. The Rust binding calls its methods.

The server, the page and the safety rules are the same Rust code for the two.

## Build again

The page needs the area and the input that the database came from. Before this change, the database did not record them. Now the
build writes two columns in `main.visualization_metadata`:

| Column | Has |
|---|---|
| `build_config` | the config of the build as YAML: `duckosm build` writes all of its `Config` (with the options of the command line); `duckosm-rust --config` writes the text of the config file |
| `build_dir` | the folder in which the build ran, for the relative paths of the config |

Build again then runs the build again, with the fixes file as `osm_overrides`, into the same database file:

- `duckosm edit`: `duckosm build` (`DuckOSM(config).run()`), the same steps that made the database.
- `duckosm-rust edit`: the native build of `duckosm-build`, from the config text. A step that the native build does not have shows as
  "skipped" in the result. Only the `.duckdb` file is written again (no GeoParquet files).

The server closes the database before the build and opens it again after. Then the page loads again, and the server gives the page of
the new network. The rules come back from the fixes file.

If the database has no record (a database from an older duckOSM, from `duckosm-rust` without `--config`, or from Studio), or the native
config reader cannot read the record, Build again is off, and the reason is under the button (the new optional hook `ctx.buildOff` of
the editor).

## The page

`rust/duckosm-edit/page/edit.js` and `edit.css` go into roadstyle's page as `page_script` and `page_style`. The script:

- gives the editor roadstyle's editor frame in the dashboard's panel (`rs.frame` with `panel: #sb` and no back button), with the name
  of the fixes file in its title line and the line of state under the tabs. While the editor is open, the frame hides the panel's
  body, as in Studio.
- calls `initFixesPanel` with the `ctx` of Studio: `frame`, `duck`, `map` (`window.map`), `yaml` (js-yaml 4.1.0, the version of Studio),
  `status`, `areaKey` (the editor reads the roads of the map itself: `window.RS_SIDE.props`), `onChange`, `onBuild`, `changesNote` and `buildOff`.
- loads the fixes file. A file that is not valid YAML is not changed: the page says so and saves nothing.
- saves the rules with each change (`onChange`, `PUT /api/fixes`), only when the text changes. The panel's **Download** button stays.
- closes the editor when the template's hide button hides the panel, and opens it again with the show button.

The page writes the file from the editor's rules (`fixes.js`, `dump`). So the comments of a file that you wrote by hand do not stay
after the first change. The `.bak` file keeps the old file.

## Tests

- `rust/duckosm-edit/src/lib.rs`: the server's safety (another `Host`, another `Origin`, a write to another path, a body or headers
  too large, a chunked body), the save (the first save makes the file, the `.bak` of an old file), a query and a build.
- `tests/test_edit_page.py` (Playwright, headless Chromium, Monaco only): `duckosm edit` on a Monaco build, pick a road, add a change,
  see `osm_fixes.yaml` on disk, pick the road again (the box "This road is in your changes"), and Build again.
