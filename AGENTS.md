# AGENTS.md

Rules for AI coding agents (Codex, Cursor, Copilot, Gemini, Claude Code, …) working on this repository.
To *use* duckOSM rather than change it, read the agent skill [`skills/duckosm/SKILL.md`](skills/duckosm/SKILL.md).

## Commands

Use the project venv — `duckosm` and `pytest` are only on `PATH` when it's active.

```bash
pip install -e ".[dev,routing]"                     # dev = pytest, lxml (MATSim schema tests), networkx. It builds rust/ with maturin (needs a Rust toolchain): run it again after a change in rust/
# run pytest with an absolute PYTHONPATH (or none): a test that changes the folder starts `python -m duckosm`, and a relative path then loads another duckosm

.venv/bin/python -m pytest tests/ -q                          # full suite
.venv/bin/python -m pytest tests/test_merge_segments.py -q    # one file
.venv/bin/python -m pytest tests/test_gmns.py -k lane -q      # one test by name

.venv/bin/duckosm build --config config/sample_monaco.yaml # the published sample (seconds)
.venv/bin/duckosm build --config config/<area>.yaml        # Kaveh's area configs: local, git-ignored
.venv/bin/duckosm <command> --help                         # every exporter/tool is a click subcommand
python scripts/validate_geometry.py --db data/db/<area>.duckdb   # self-loop / endpoint / zero-length checks
jupyter nbconvert --to markdown docs/notebooks/[0-9]*.ipynb    # the pages of the notebooks (git-ignored): run it before the docs build
mkdocs build --strict                                      # docs site (pip install "mkdocs<2" "mkdocs-material<10" "mkdocs-llmstxt<0.6" nbconvert); CI deploys it to Pages
pytest --nbmake docs/notebooks/                            # run the five demo notebooks (pip install nbmake); they keep their saved outputs
```

Several tests skip themselves unless local artifacts or tools exist: `pbf/sodermalm*.osm.pbf`,
`data/db/sodermalm*.duckdb` (built parent db), `osmium`, `ogr2ogr` (GDAL), `netconvert` (SUMO).
`pbf/`, `data/db/`, `reports/` are gitignored — a skip is not a pass.

## Architecture

The library is Rust (`rust/`). The `duckosm` program is `duckosm-cli` (`rust/duckosm-cli/src/main.rs` dispatches the
commands to the crates). The wheel holds the whole program in the Python module `duckosm._rust` (maturin,
`rust/duckosm-py`): `python -m duckosm` runs it in the Python process (`src/duckosm/_main.py`).

**Build = `duckosm build`** (`build_cmd` in `rust/duckosm-cli/src/main.rs`; the steps in `rust/duckosm-build/src/lib.rs`,
the network in `rust/duckosm-core`, the DuckDB steps in `rust/duckosm-duckdb`). Step order carries real dependencies:

- **Global steps once:** read the PBF and cut it to the boundary → OSM fixes → `global_junctions` (a mode-agnostic
  junction set so every mode splits roads at the same points and shares `edge_id`s).
- **Per mode:** road filter → network (merge of the chains of one road, virtual nodes) → path connector → dismount →
  speeds → costs → functional type → turn restrictions → edge graph → H3 and tile keys.
- **After modes:** the `.duckdb` file, multimodal `mm.*`, `features.*` base-map layers (they need `raw.*`), bus routes,
  lane profile, the `edge_id_hash` macros, `main.visualization_metadata`, validation, and the drawing order
  (`visualization.edge_levels`) with the edits of the file that the build replaces.
- **Clip mode** (`source.type: duckdb`): no PBF; the rows of an attached parent build are copied (e.g.
  `sodermalm ← sweden`). Edge ids are preserved verbatim; `duckosm extract` does the same ad hoc.

`config.py` holds the YAML schema in Python (`DuckOSM(config).run()` writes it as a file and runs `duckosm build -c`);
`rust/duckosm-build/src/config.rs` reads the file and knows every key. `src/duckosm/templates/config.yaml` is the
commented reference that `duckosm init-config` writes; `docs/reference/configuration.md` has every field. Relative paths
resolve against the current folder: a pip install has no repo, so only files inside `src/duckosm/` ship.

**Exporters** (`rust/duckosm-export`: SUMO, MATSim, GMNS, OpenDRIVE, Lanelet2, railML, GIS, OSM XML, CommonRoad lanelets)
read a *finished* db. Lane-level exporters (`matsim-lanes`, `lanelet2`, `opendrive --junctions`, `lane-graph`) take a
**GMNS db** produced by `duckosm gmns`, not the core db. `railml` re-extracts rail from `raw.*`. Routing is
`rust/duckosm-route`; the pages (`edit`, `ask`, `studio`, `viz`, `route-map`) are `rust/duckosm-edit`.

**The Python SDK** (`src/duckosm/*.py`) calls `duckosm._rust` (the bindings in `rust/duckosm-py/src/`) or the `duckosm`
program, and makes Python objects of the results (DataFrames, GeoDataFrames, networkx graphs from a Rust pickle). See
"The library is Rust" below.

## GMNS lanes, movements and footpaths (2026-10-02, branch `gmns-values`)

Data holes are fixed here, in `gmns.py`, not in a reader (lanestyle only draws). Design notes in `docs/design/gmns_lane_connectors.md`, `gmns_crossings.md`, `gmns_walking_frame.md`:
lane width by road class where OSM has none (`_LANE_W_BY_CLASS`, written to `lane.width`); `_inherit_lanes` (an untagged one-way edge takes the lane count of the road it plainly
continues); `_continuation_movements` (a car lane the road has no room for ahead merges, a bike lane continues; skipped where `turn:lanes` marks the lanes); connector width fitted to its curve
(`_fit_width`); `link_along` + `along_*` (the roads a footpath runs along: cars-only roads of its own level, matched by how long the road runs along it; a `footway=crossing` only when long and parallel
or continuing a matched footpath); `--walk-frame` (moves footpaths to the kerb) is opt-in and off. Matching and crossings compare the exact level (`_level`: layer, else bridge 1 / tunnel -1).

## Invariants that are easy to break

- **`edge_id` is a stable content hash**, not a sequence:
  `(hash(osm_id, source, target) >> 1)::BIGINT` (`edge_id.py`). Downstream projects
  (sensor-matching, SonoFlow, fetching-sweden-data…) join on it, and every exporter uses it as the
  target format's id. Don't change the formula, the arg order, or the `duckdb<2` pin (DuckDB's
  `hash()` isn't stable across majors). Legacy `edge_id_hash_v1` (with `is_reverse`) ships too.
- Direction is encoded by `source → target`; self-loops and parallel/antiparallel arcs are split
  with **virtual nodes (`node_id < 0`)**, content-hashed so they're stable too.
- `edges.lanes` is an INTEGER **per direction**; `oneway` is topological (decides whether the
  reverse twin exists), which is why OSM overrides must run before GraphBuilder.
- `highway=service` stays in `edges` and `edge_graph` — there is no separate service table.
- `osm_overrides/osm_overrides.yaml` holds Kaveh's fixes for OSM errors (way rules + synthetic
  `turn_restrictions:`, keyed by OSM ids, a no-op where the ways aren't present). It is applied only
  when named: config `osm_overrides:` or `build --fixes` (no default since 2026-09-30; Kaveh's local
  area configs name it). Only enable a rule once verified; provenance goes in `known_osm_issues.md`.

## The library is Rust (Kaveh, 2026-11-23)

- **Python is only a thin layer.** Each SDK function calls `duckosm._rust`. The only exception: a function that must return a
  Python object (a networkx graph). No Python engine code: no build steps, exporters or routers in Python.
- **Port and delete in the same PR.** When a part moves to Rust, its Python version goes. No fallbacks to Python code.
- **No computing in Python.** A Python function may call Rust, check its arguments, read tables for the user, and make Python
  objects of the results (a DataFrame, a networkx graph, an AequilibraE project, a CommonRoad scenario, a matplotlib picture).
  It may not compute results: no SQL that joins, aggregates or writes, no geometry (offsets, buffers, unions), no graph
  search. That work goes into a Rust function and a binding.
- **`tests/test_no_python_engine.py` guards this.** Each module of `src/duckosm` must be in its list of thin modules; the list
  of modules still to port (`LEFT`) is empty since 2026-11-23 and stays empty.
- Do not ask Kaveh about Python leftovers, and do not report them as open questions. Port them.

## Conventions

- Heavy/optional deps (geopandas, networkx, roadstyle) are imported lazily inside functions.
- Maps go through **roadstyle** (`duckosm viz`, `scripts/roadstyle_map.py`), not hand-rolled
  folium/matplotlib.
- Design docs for non-trivial changes live in `docs/design/`; update the matching `docs/*.md`
  (`docs/reference/database.md` for schema changes) alongside code.

## Hosts of the editor and Ask (Studio and others)

A host is a page that runs duckOSM's fixes editor (`web/fixes_editor/`, `initFixesPanel`) or Ask (`web/ask/`) with its own data and map: `duckosm edit`, `duckosm ask` and duckOSM Studio.
The aim: when the editor or Ask changes in duckOSM or roadstyle, a host only moves the version (the git revision of the crates `duckosm-core`, `duckosm-editor` and `roadstyle`).
It does not change its code. These rules keep this true (Kaveh, 2026-11-08):

1. **Keep the host contract compatible.** The options of `initFixesPanel` and the files of `web/ask/` are the contract. A new option has a default that keeps an old host working.
   A change that breaks a host needs a `### Host changes` entry in `CHANGELOG.md` and the text of the migration, in the same PR.
2. **Say what a host must do, in each PR.** The PR text ends with one line: `Host change: none, only move the version`, or the exact change that the host must make.
3. **Test the contract on a host with the shape of Studio, not only on `duckosm edit`.** Studio has vector-tile overlays on a shared source (`rs-roads`, not a source with the name of the overlay),
   array rows in `RS_SIDE.props` (`roadKeys`) and a DuckDB-wasm style data access (`ctx.duck`). The hover bug and the road index work both came from this gap.
   The test host is planned in `docs/design/test_host.md`. Until it exists, check a change of the contract against these three by hand.
