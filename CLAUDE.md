# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

Use the project venv — `duckosm` and `pytest` are only on `PATH` when it's active.

```bash
pip install -e ".[dev,routing]"                     # dev = pytest, lxml (MATSim schema tests), networkx

.venv/bin/pytest tests/ -q                          # full suite
.venv/bin/pytest tests/test_merge_segments.py -q    # one file
.venv/bin/pytest tests/test_gmns.py -k lane -q      # one test by name

.venv/bin/duckosm build --config config/sample_monaco.yaml # the published sample (seconds)
.venv/bin/duckosm build --config config/<area>.yaml        # Kaveh's area configs: local, git-ignored
.venv/bin/duckosm <command> --help                         # every exporter/tool is a click subcommand
python scripts/validate_geometry.py --db data/db/<area>.duckdb   # self-loop / endpoint / zero-length checks
mkdocs build --strict                                      # docs site (pip install "mkdocs<2" "mkdocs-material<10"); CI deploys it to Pages
```

Several tests skip themselves unless local artifacts or tools exist: `pbf/sodermalm*.osm.pbf`,
`data/db/sodermalm*.duckdb` (built parent db), `osmium`, `ogr2ogr` (GDAL), `netconvert` (SUMO).
`pbf/`, `data/db/`, `reports/` are gitignored — a skip is not a pass.

## Architecture

**Build = `DuckOSM(config).run()`** (`src/duckosm/importer.py`). `run()` assembles an ordered step
list from the config and executes it; read it first — step order carries real dependencies:

- **Global steps once:** connect → (osmium pre-clip of the PBF to the boundary, cached in `pbf/`) →
  `ST_READOSM` into `raw.nodes/ways/relations` → boundary → `global_junctions` (a mode-agnostic
  junction set so every mode splits roads at the same points and shares `edge_id`s).
- **Per mode** (`CREATE SCHEMA <mode>; USE <mode>`, so unqualified table names are per-mode):
  RoadFilter → OSM overrides → GraphBuilder → GraphSimplifier (incl. `merge_segments`) →
  path connector → dismount → speeds → costs → functional type → turn restrictions (driving only)
  → EdgeGraph → ComponentFilter → H3 → indexes → validate.
- **After modes:** multimodal `mm.*`, `features.*` base-map layers (needs `raw.*`, so before
  cleanup), persist `edge_id_hash` macros, cleanup, `main.visualization_metadata`, report, viz.
- **Clip mode** (`source.type: duckdb`): no PBF — `DuckdbClipper` copies rows out of an attached
  parent build (e.g. `sodermalm ← sweden`), then ComponentFilter/validate. Edge ids are preserved
  verbatim; `duckosm extract` does the same ad hoc.

Each stage is a `BaseProcessor` subclass in `src/duckosm/processors/` that runs SQL on the shared
connection — logic lives in SQL, not Python loops. `config.py` holds the YAML schema
(`src/duckosm/templates/config.yaml` is the commented reference `duckosm init-config` writes;
`docs/configuration.md` every field). Relative paths resolve against the current folder: a pip
install has no repo, so only files inside `src/duckosm/` ship.

**Exporters** (`sumo.py`, `matsim*.py`, `gmns*.py`, `opendrive.py`, `lanelet2.py`, `railml.py`,
`gis.py`, `routing.py`) read a *finished* db and are wired as subcommands in `cli.py`. Lane-level
exporters (`matsim-lanes`, `lanelet2`, `opendrive --junctions`, `lane-graph`) take a **GMNS db**
produced by `duckosm gmns`, not the core db. `railml` re-extracts rail from `raw.*`.

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

## Conventions

- Heavy/optional deps (geopandas, networkx, rasterio, roadstyle, sumolib) are imported lazily
  inside functions; the core install is duckdb/shapely/h3/click/rich/pyyaml only.
- Maps go through **roadstyle** (`duckosm viz`, `scripts/roadstyle_map.py`), not hand-rolled
  folium/matplotlib.
- Design docs for non-trivial changes live in `docs/design/`; update the matching `docs/*.md`
  (`data_dictionary.md` for schema changes) alongside code.
