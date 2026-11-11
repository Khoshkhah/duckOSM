# `duckosm studio`: make the data of an area on your computer

**Status:** done 2026-11-10. Kaveh approved the design ("Yes, build it").

## Problem

duckOSM Studio (the browser app) has the steps that make the data of an area: the area, the OSM data, the settings, the ground and the build.
The build runs in the browser, so the area is small and the result is in the browser. duckOSM had no page of its own for these steps. The Ground step
was a part (`web/ground_step/`) and had no page.

A part belongs to its owner in two ways: as a page of its own on your computer (`duckosm edit`, `duckosm studio`) and as a part that Studio loads at a pin.

## The command

```bash
duckosm studio [OUT_DIR] [--port 8792] [--geofabrik-index URL_OR_FILE] [--no-open]
duckosm-rust studio [OUT_DIR] [same options]
```

The two commands run the same Rust server (`rust/duckosm-edit/src/studio.rs`). `duckosm studio` calls `duckosm._rust.studio()`; there is no Python web
server. The server shares its request rules with `duckosm edit` (`read_request` in `lib.rs`: 127.0.0.1 only, this server's name in `Host`, a write from this
server's page, a body of at most 16 MB).

## The steps

| Step | Part | The server |
|---|---|---|
| 1. Area | `web/build_steps/area_step.js`: Nominatim search, a box on the map, a GeoJSON file | none |
| 2. OSM data | `data_step.js`: the Geofabrik extracts that cover the area, or the path of a `.osm.pbf` | `GET /api/geofabrik` (the list, read once), `POST /api/file` |
| 3. Settings | `settings_step.js`: the networks and the options, as the text of a config | none |
| 4. Ground | `web/ground_step/` (the part of PR 69) | `PUT /api/dem/<name>` (the DEM file) |
| 5. Build | `build_step.js` | `POST /api/build`, `GET /api/job` |
| 6. Result | `result_step.js`: the link to `/edit/` | `/edit/...` |

The Result is the page of `duckosm edit`, not a copy. The server makes the `Site` of `duckosm edit` for the finished database and sends each request
under `/edit/` to it. The page uses relative addresses, so it works under the prefix.

## Decisions

0. **The page is roadstyle's dashboard.** The server makes the page with roadstyle's page writer (`page::make_studio`: the template `dashboard`, an empty road table, `tiles: false`), as it does for `duckosm edit`, so the panel,
   the toolbox, the theme and the phone sheet are roadstyle's, unchanged. `page/studio.js` adds each step as a card with `rs.sections.add({ step, fold })`, as duckOSM Studio does, and `page/studio.css` has only the
   rules that Studio's own page has for its step cards. The roadstyle pin of `duckosm-edit` moves with the page (to `34e21f30`). Gaps of roadstyle that the page works around, for a roadstyle change:
   a page with no roads shows the banner "0 road features" (the script removes it); a `label.rs-opt` in the panel is upper case (`#sb label`; the part CSS resets it); the first view of a page with no roads is 0, 0 (the script jumps to Europe).

1. **The place keys come from the server, the settings from the page.** The config of a build is the keys of the place (`name`, `output_path`, `source`, `boundary`,
   `elevation`) and then the text of the Settings step. The page does not know a path on the disk. The config is written to `config.yaml` in the folder of the area, and it
   is the record of the build (`build_config` of `main.visualization_metadata`), so **Build again** of the edit page works.
2. **One folder for each area**, `OUT_DIR/<name>/`. The fixes file is beside the database, so two areas have two fixes files.
3. **Rust is the product, Python is the cross-check** (Kaveh, 2026-11-10: everything is written in Rust; the Python version stays for cross-validation). The build is a `Host`. The native program (`NativeHost`) runs `duckosm-build` with DuckDB. The Python module has no DuckDB, so its host (`duckosm/studio.py`)
   runs `duckosm build` (`duckosm.edit.build_area`): the Rust engine and the SQL steps, with the base-map layers. The two hosts give the server a line for each step
   (`set_progress` of `duckosm-build`, and `DuckOSM.on_step`), and the page shows them.
4. **The DEM in the Python host.** The Python build does not read the `elevation` section, so `build_area` runs `duckosm.elevation.to_elevation` after the build and
   writes `main.elevation_grid` from the Rust engine (`_rust.elevation_grid`), the same rows as the Rust build. A DEM needs `duckosm[elevation]` (rasterio). The same
   function is used by Build again of `duckosm edit`, so the DEM stays after a new build.
5. **The download is in the server.** A browser cannot read download.geofabrik.de (no CORS header). The server downloads the extract into the folder of the area
   with `ureq`, and the job has the bytes. A file that is in the folder is not downloaded again.
6. **A local PBF is a path.** The page is on the same computer as the server, but a browser does not give the path of a file. The person types it, and the server
   checks it. A host in a browser (Studio) has no path, so it does not use `data_step.js`.
7. **Offline-friendly.** MapLibre and roadstyle's modules come from the program (`/vendor/`, `/roadstyle/`). Only the map tiles, Nominatim, Geofabrik and the
   js-yaml of the edit page use the internet.

## The areas on the disk

Studio keeps the areas in the browser (OPFS), so this page keeps them where `duckosm` keeps them: on the disk. The card **Your areas** (`rs.sections.add`, as in Studio) lists the folders of `OUT_DIR` that have
`<name>/<name>.duckdb` (`GET /api/areas`, newest first, with the config text for the networks). **Open** (`POST /api/open`) makes the page of the result for that area at `/edit/`. **Load**
(`GET /api/area/<name>`) gives the boundary and the config, and the page puts them into the Area and Settings steps (`parseSettings` of `core.js` reads back what `settingsYaml` wrote). There is no Delete:
the server does not delete files.

## Gap: the Rust build is smaller than the Python build

The native build does not make `features.*` (the base-map layers), the H3 columns (`h3_cell`, `from_cell`, `to_cell`, `lca_res`), `features.ocean`, and the optional checks, report, maps and multimodal graph. The page of
Studio (browser) makes `features.*` by running roadstyle's SQL (`features_sql`) on DuckDB. The native program can run the same SQL on its bundled DuckDB over `raw.*`; H3 can use a Rust H3 crate. Until this is done,
`duckosm studio` (Python) gives the full result. This is the next task after this page.

## Limits

One build at a time. A DEM file is at most 16 MB. The build cannot be stopped from the page.

## Tests

`web/build_steps/steps_units.mjs` (Node), the tests of `studio.rs` (the routes, the rules, a build with a fake host), and `tests/test_studio_page.py`
(Chromium, Monaco: a search, a Geofabrik extract from a local mirror, no DEM; a boundary file, a local PBF, a DEM; a box on a phone of 390 × 844).
