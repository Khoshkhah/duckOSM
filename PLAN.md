# duckOSM pipeline update — PLAN

**Status:** spec (2026-06-17). Goal: make duckOSM a self-contained network-build tool by
absorbing the orchestration that currently lives in `osm-traffic-enrichment/pipeline_network.py`,
fixing the boundary-clip gap, adding a **clip-from-existing-db** source mode, a connected-component
clean-up, connectivity-aware road filtering, structured logging + progress, build-time validation,
a build report, and roadstyle visualization.

---

## 1. Motivation

Three concrete problems drove this:

1. **`boundary_path` doesn't clip.** `importer._load_boundary` only loads a metadata table; the
   actual clip lives in the wrapper (`osmium extract --polygon`). Building duckOSM directly with
   `--boundary` yields the whole source PBF (all of Sweden), and an externally-clipped PBF leaves
   **boundary-crossing stub components** (observed: 3621 edges / **306** weak components for
   Södermalm vs production's 4471 / 4). Nothing — neither project — cleans those up.

2. **The road filter dropped real connectors.** The old per-way, tag-only driving filter dropped a
   street's *sole connector* when OSM tagged it `pedestrian` (Lottens Gata → stranded a residential
   block) or `motor_vehicle=no` (Katarinavägen → stranded the Slussbrogatan chain). Fixed by a
   class-trusting + connectivity-aware filter (see §4).

3. **The build smarts live in a wrapper.** `pipeline_network.py` does boundary fetch, PBF download,
   osmium clip, the duckOSM call, timezone, H3, logging and a summary — but duckOSM itself can't do
   a clean area build alone. We move that capability into duckOSM.

---

## 2. Target architecture

### 2.1 Two source modes
- **`source.type: pbf`** — build a network from OSM: (optional) download a country PBF, **clip it
  in-pipeline with osmium**, load, RoadFilter, graph build, simplify, speeds/costs/restrictions,
  components, H3.
- **`source.type: duckdb`** — **derive an area by clipping a parent build** (e.g. `sodermalm ←
  sweden`): spatially select the parent's edges inside the boundary, filter `edge_graph` to the
  survivors, copy the relevant nodes, component-clean. **Edge_ids are preserved** from the parent
  (same geometry + source/target → same hash), so a sub-area is a stable *view* of its parent — no
  re-hash, no cross-project re-key.

### 2.2 Pipeline stages (new order)
```
[pbf mode]    fetch boundary → download PBF → osmium clip → load → RoadFilter →
              GraphBuilder → GraphSimplifier → speeds → costs → restrictions →
              EdgeGraphBuilder → ComponentFilter → H3 → validate → report → viz
[duckdb mode] fetch boundary → DuckdbClipper(parent, boundary) → EdgeGraphBuilder(re-key-free) →
              ComponentFilter → (H3) → validate → report → viz
```
New stages in **bold-equivalent**: in-pipeline **osmium clip**, **DuckdbClipper**, **ComponentFilter**,
**validate**, **report**, **viz**.

### 2.3 Config schema
The canonical schema is `config/template.yaml` (full, commented). Top-level keys:
`name · output_path · source{type,…} · boundary{path|place|bbox|h3_cell,buffer_m} ·
clip{predicate,keep_largest_component,min_component_edges,connectivity_rescue,strongly_connected} ·
modes · options{…} · validation{…} · report{} · viz{} · cache_dir`.
No `refresh` key — the network db always rebuilds; only immutable inputs (downloaded PBF, fetched
boundary) are cached under `cache_dir`. Existing flat keys (`pbf_path`, `boundary_path`, `h3_cell`)
remain accepted as shorthand for `source.type: pbf` for back-compat.

---

## 3. Workstreams

### A · Build core
- **A1 — in-pipeline osmium clip** (pbf mode). New importer step `_clip_pbf()` before `_load_pbf`;
  lift `filter_pbf` (osmium `extract --polygon`, `complete_ways`). Idempotent on `cache_dir`.
  Closes the `boundary_path`-doesn't-clip gap.
- **A2 — `DuckdbClipper` processor** (duckdb mode). `ATTACH parent; SELECT edges WHERE
  <predicate>(geometry, boundary)`; filter `edge_graph` to surviving `from_edge`/`to_edge`; copy
  nodes; keep `edge_id`. New module `src/duckosm/processors/duckdb_clipper.py`.
- **A3 — `ComponentFilter` processor**, after `EdgeGraphBuilder`, before H3. Weakly-connected
  components → keep largest, drop/flag smaller than `min_component_edges`; optional
  `connectivity_rescue` (promote a `maybe` connector iff it reconnects a drive component);
  `strongly_connected` toggle. New module `src/duckosm/processors/component_filter.py`.
- **A4 — connectivity-aware RoadFilter** (done): road classes trusted (incl. `_link`,
  `living_street`, `service`); `highway=pedestrian` rescued iff `motorcar/motor_vehicle ∈
  yes/designated/permissive` or `access ∈ delivery/destination/agricultural`. Next: emit a
  `drive_status`/`drop_reason` so `maybe` connectors survive for A3's rescue.
- **A5 — config + CLI**: `source`/`boundary`/`clip`/`validation` blocks in `config.py`; CLI
  `duckosm build <name|--config> [--place|--boundary] [--country|--pbf] [--source duckdb
  --source-db …] [--modes]`.

### B · Logging + progress
- Lift `osm-traffic-enrichment/pipeline_utils.py` (`StepTimer`, `setup_logging`, `write_summary`)
  into `src/duckosm/logging.py`: leveled logging to console **and** `logs/<name>_<ts>.log`, per-step
  timing.
- Extend the existing Rich progress to **every** stage (download → clip → filter → graph →
  components → validate) as one consistent tree with per-stage counts/ETA, plus an end-of-run
  summary line.

### C · Testing
- `tests/` (pytest): **unit** (RoadFilter buckets, `DuckdbClipper` predicates, `ComponentFilter`
  on a fixture graph) + **integration** (full build of a bundled mini-PBF, and a duckdb-clip of a
  bundled mini-db) → assert invariants.
- **Build-time validation hook** driven by `validation:`; fail the build. Invariants: 1 dominant
  component · no stranded named through-street · `edge_id` stable (vs fixture / vs parent) ·
  `edge_graph` shares-a-node · no node pair < snap tolerance.

### D · Reporting
- `reports/<name>_<ts>.{md,html}` per build: stats (nodes/edges/edge_graph per mode), clip
  provenance (source, boundary, predicate, parent), the **filter audit** (kept/`maybe`/dropped by
  reason; rescued connectors named), **component summary** (largest + each dropped stub with
  location/road), **validation results**. New `src/duckosm/report.py`.

### E · Visualization (roadstyle)
- `duckosm viz` + `src/duckosm/viz.py` using `import roadstyle as rs` (the package at
  `/home/kaveh/projects/roadstyle`, as in `traffic_tube/.../11_roadstyle_map.py`):
  - **network map** — edges by highway class (roadstyle casing+fill, CARTO basemap, legend);
  - **QA map** — kept network vs dropped stubs vs rescued connectors, per-class layer toggles +
    hover detail (the roadstyle successor to the throwaway `duckosm_errors.html`).
  - interactive HTML + static PNG into `reports/`.

### F · Docs + migration
- Rewrite `README.md`; update `docs/configuration.md`; add `docs/pipeline.md` (stages, source
  modes, the sweden→sodermalm example) and `docs/testing.md`. Document RoadFilter logic,
  ComponentFilter, validation, report, viz.
- `osm-traffic-enrichment/pipeline_network.py` network half collapses to one `duckosm build
  --config` call; only traffic-enrichment-specific steps stay in the wrapper.

---

## 4. RoadFilter (current state, ref)

Driving keep rule now: `highway ∈ {motorway…residential, *_link, unclassified, service,
living_street, road}` **OR** `highway=pedestrian AND (motorcar/motor_vehicle ∈
{yes,designated,permissive} OR access ∈ {delivery,destination,agricultural})`. Road classes are
trusted regardless of `access`/`motor_vehicle` restrictions (lenient: keeps a possibly-restricted
road over disconnecting the network; the regime/sim down-weights it by low flow). The component
clean-up + connectivity-rescue (A3) handle the residual stranded connectors that tags can't.

---

## 5. Execution order
1. **A** (source modes + clip + ComponentFilter) — the heart; makes duckOSM standalone.
2. **B** (logging + progress) — observability around the build.
3. **C** (testing + validation) — lock correctness in.
4. **D + E** (report + roadstyle viz) — the "show the result" layer.
5. **F** (docs/README + CLI + wrapper migration).

## 6. Open decisions
- `clip.predicate` default (`intersects` + largest-component vs `within`).
- `strongly_connected` keep vs flag-only for one-way traps.
- `connectivity_rescue` length cap + whether to auto-rescue `mv=no` road connectors or require an
  allowlist.
- Whether `source.type: duckdb` re-runs `simplify` (default: no — parent is already simplified).
