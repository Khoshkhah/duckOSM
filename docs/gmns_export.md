# GMNS extract → a standalone GMNS DuckDB (`duckosm gmns`)

Extract **every GMNS table that OSM can support** — including **lane detail** — into **one new,
self-contained `.duckdb` file**. The output stands alone: it doesn't reference the source build, it
carries native geometry so it's queryable and **renderable (down to individual lanes)**, and it can
be dumped to spec-standard GMNS CSVs on demand. This is the #1
[Tier‑1 export target](../../product/simulation-export-targets.md), and it keeps duckOSM's
signature: the stable `edge_id` becomes the GMNS `link_id`.

---

## What is GMNS

GMNS — the **General Modeling Network Specification** — is *"a common machine- (and human-) readable
format for sharing routable road network files,"* for multimodal static **and** dynamic
transportation models. Maintained by the **Zephyr Foundation** with Volpe / FHWA; current version
**0.97** (March 2026). It is a **directed node–link graph expressed as a set of flat tables**; only
`node` and `link` are required, and you add optional tables (lanes, movements, geometry, …) *only
where you have the data* — a "provide what you have" spec. Geometry lives as WKT on the link or in a
separate `geometry` table; a `config` table declares units + CRS.

Spec: <https://github.com/zephyr-data-specs/GMNS> · docs: <https://zephyr-data-specs.github.io/GMNS/>

---

## The GMNS format — full table catalogue (v0.97)

All 24 GMNS tables, whether the spec **requires** them, and whether **this extractor** can fill them
from OSM (✅ full · ◐ partial, OSM has some of it · ✕ needs data OSM doesn't carry):

| Table | Spec | OSM | Notes |
|-------|------|-----|-------|
| `config` | optional | ✅ | units, CRS, version |
| **`node`** | **required** | ✅ | vertices; `ctrl_type='signal'` from `highway=traffic_signals` |
| **`link`** | **required** | ✅ | directed segments |
| `geometry` | optional | ✅ | link shapes (also inlined as WKT / native geom) |
| `lane` | optional | ✅ | **per-lane rows from OSM lane tags** — see below |
| `movement` | optional | ✅ | turns, from the `edge_graph` line graph |
| `use_definition` / `use_group` | optional | ✅ | the modes present (auto/walk/bike) |
| `signal_controller` | optional | ◐ | signal *locations* (from `highway=traffic_signals`; no phases/timing in OSM) |
| `curb_seg` | optional | ◐ | from `parking:left`/`parking:right` (or `parking:lane:*`) where tagged |
| `segment` / `segment_lane` (+ `_tod`) | optional | ✕ | mid-link overrides — edges already split at junctions |
| `signal_coordination` / `_detector` / `_phase_mvmt` / `_timing_phase` / `_timing_plan` | optional | ✕ | signal **timing** — not in OSM |
| `zone` | optional | ✕ | demand zones (TAZ) — not network data |
| `location` | optional | ✕ | point features along a link |
| `link_tod` / `movement_tod` / `lane_tod` / `segment*_tod` / `time_set_definitions` | optional | ✕ | time-of-day dynamics — the network is static |

**So "everything from OSM" ≠ the whole spec** — the ✕ rows need signal timing, travel demand, or
time-of-day data an OSM network extract simply doesn't contain, and GMNS's own contract is to omit
what you don't have. What the extractor produces is the complete **network + lanes + connectivity**
layer: `config, node, link, geometry, lane, movement, use_definition, use_group` (+ optional
`signal_controller`, `curb_seg`).

---

## Output: one standalone `.duckdb` file

```bash
duckosm gmns data/db/sodermalm.duckdb -o sodermalm_gmns.duckdb
```

- **One file, a schema per mode** — `gmns_driving`, `gmns_walking`, `gmns_cycling` — because
  `edge_id` collides across modes (same physical edge → same hash), so each mode is its own GMNS
  network (which is what DTALite / Path4GMNS consume). Lane detail is populated for the vehicle
  (`driving`) mode.
- **Native geometry, not just WKT.** `node.geom` (POINT) and `link.geom` (LINESTRING) are stored as
  DuckDB `GEOMETRY` (EPSG:4326) so the file is directly queryable (`ST_*`) and renderable. The
  spec's textual `geometry` (WKT) is kept alongside for CSV fidelity.
- **Self-contained.** No attach to the source db; everything needed to route, analyse, or draw the
  network is in the one file.
- **Dump to spec CSVs anytime:** `duckosm gmns … --to-csv gmns/` writes `node.csv`, `link.csv`,
  `lane.csv`, `movement.csv`, … per mode (the canonical GMNS interchange), with `ST_AsText` geometry.

---

## Column reference (tables the extractor writes)

Columns are the **GMNS spec columns**; *Source* is how OSM/duckOSM fills each. Spec columns with no
source are listed so the full shape is visible.

**`config`** (one row): `dataset_name`, `long_length=meter`, `short_length=meter`, `speed=kmh`,
`crs=EPSG:4326`, `geometry_field_format=wkt`, `version_number=0.97`, `id_type=integer`.

**`node`** — `node_id`✅(`nodes.node_id`) · `x_coord`✅(lon) · `y_coord`✅(lat) ·
`ctrl_type`(`'signal'` if the node is an OSM `traffic_signals`) · *`name,z_coord,node_type,zone_id,
parent_node_id` blank*. Plus non-spec `geom` (POINT) for rendering.

**`link`** — `link_id`✅(**`edge_id`**) · `from_node_id`✅(`source`) · `to_node_id`✅(`target`) ·
`directed`✅(`true`) · `geometry`(WKT) · `dir_flag`(1) · `length`(`length_m`, m) ·
`free_speed`(`maxspeed_kmh`, km/h) · `lanes`(count) · `capacity`(class default, pce/hr/lane) ·
`facility_type`(**`highway`** — GMNS's field, not `link_type`) · `name` · `allowed_uses`(mode) ·
`bike_facility`/`ped_facility`(from `highway`/`cycleway`/`sidewalk`) · *`grade,toll,parking,
jurisdiction,row_width,geometry_id,parent_link_id` blank*. Plus non-spec `geom` (LINESTRING).

**`lane`** — one row per lane of a directed link: `lane_id`✅ · `link_id`✅(`edge_id`) ·
`lane_num`✅(1…N, GMNS left-to-right) · `allowed_uses`(per-lane, from `*:lanes` tags, else the mode) ·
`width`(from `width:lanes`) · `r_barrier`/`l_barrier`(from `change:lanes`). Optional non-spec
`geom`: the lane centerline offset from the link for lane-level rendering (see Visualization).

**`movement`** — one row per legal turn (`edge_graph`): `mvmt_id`✅ · `node_id`✅(junction) ·
`ib_link_id`✅(`from_edge`) · `ob_link_id`✅(`to_edge`) · `allowed_uses`(mode) · `type`(left/right/
thru/uturn from the bearing change at the junction) · `mvmt_code`(direction+turn, `NBL`/`EBT`… from
the inbound compass bearing) · `start_ib_lane`/`end_ib_lane`(the inbound lanes feeding the turn, from
`turn:lanes`) · `geometry`(a short turn-path connector) · `ctrl_type`(signal if the node is
signalized) · *rest blank*. The **immediate reversal** (turning back onto the same physical segment — `edge_graph`
carries it for routing completeness) is dropped; genuine intersection U-turns (a different `osm_id`)
stay and are typed `uturn`.

**`use_definition`** — `use`, `persons_per_vehicle`, `pce` (defaults: auto 1.0/1.0, etc.).
**`use_group`** — `use_group`, `uses`, `description`.

---

## Lane details from OSM (the new extraction)

The lane rows are parsed from the raw OSM way tags (`raw.ways.tags`, a `MAP`) joined to each edge by
`osm_id`, honouring direction (`is_reverse`). OSM lane conventions:

| OSM tag | → GMNS | Handling |
|---------|--------|----------|
| `lanes`, `lanes:forward`, `lanes:backward` | lane **count** per direction | forward edge gets `:forward` (or the ½-split), reverse gets `:backward`; else the class default already on `edges.lanes` |
| `turn:lanes[:forward/backward]` | per-lane **turns** → `movement` lane ranges + `type` | split on `\|`, e.g. `through\|through\|right` → 3 lanes, lane 3 feeds the right turn |
| `width:lanes` | `lane.width` | split on `\|`, per lane |
| `bicycle:lanes`, `psv:lanes`, `busway` | `lane.allowed_uses` | e.g. `bicycle:lanes=no\|no\|designated` → lane 3 allows `bike` |
| `change:lanes` | `lane.r_barrier`/`l_barrier` | `no`/`not_left`/`not_right` between lanes |

Coverage is partial (measured: `lanes` ~9–15 %, `turn:lanes` ~1–2 % of ways) — every link still gets
`lane` rows from its lane **count**, enriched wherever OSM tagged the detail. **This needs the OSM
tags** (`raw.ways`), present in PBF-mode builds; a duckdb-clip build would read the parent's `raw`
(or take `--source-pbf`).

---

## Visualization with lane detail

Because the output carries native `GEOMETRY`, it renders with no extra step:

- **Network level** — `link.geom` / `node.geom` draw with roadstyle / duckmap or the
  self-contained viewer, exactly like a duckOSM db.
- **Lane level** — each `lane` row optionally carries a `geom`: the link centerline **offset** by
  `(lane_num − center) × width` (right-hand-drive aware), so a renderer draws parallel lanes with
  their turn arrows and use colours (bus/bike/general). Offsets are computed once at export
  (shapely `offset_curve`), so the viewer just draws them — a true lane-level map from one file.

*(The lane `geom` is a non-spec convenience column; the spec `lane` columns remain pure, and
`--to-csv` drops `geom` so the CSVs stay standard.)*

---

## Why the mapping is clean

- **Directed already** — `<mode>.edges` are one row per direction → **1 edge = 1 GMNS link**.
- **Turns already resolved** — `edge_graph` is the turn-restriction-respecting line graph → it *is*
  the `movement` table.
- **Stable ids ride along** — `link_id = edge_id`; lanes and movements reference it, so measured
  data keyed to `edge_id` drapes onto links, lanes, and turns.

## Units & conventions (declared in `config`)

EPSG:4326 lon/lat · length & width in metres · `free_speed` km/h · ids 64-bit integers, exact.

## Usage

```bash
duckosm gmns data/db/sodermalm.duckdb                       # -> sodermalm_pbf_gmns.duckdb (all modes)
duckosm gmns data/db/sodermalm.duckdb -m driving            # driving schema only
duckosm gmns data/db/sodermalm.duckdb -o out.duckdb --to-csv gmns/   # also dump spec CSVs
```

## Comparison with osm2gmns

[osm2gmns](https://github.com/jiawlu/OSM2GMNS) (Lu & Zhou, ASU) is the established OSM→GMNS tool. Both
were run on the **same Södermalm PBF** (2026-07-04); this is an empirical diff, not a spec sheet.

| | **duckOSM `gmns`** | **osm2gmns 1.0.1** (current pip) | **osm2gmns 0.7.6** (full) |
|---|---|---|---|
| Output | 10 tables in **one DuckDB** (+ CSV) | 2 CSVs (node, link) | 7 CSVs: macro node/link/**movement** + **meso** + **micro** |
| Rows (driving/auto) | 2,876 links · 1,482 nodes | 3,542 · 2,215 | macro 3,540 · **meso 7,100** · **micro 35,352** |
| `link_id` / `node_id` | **stable content hash** (`edge_id`) | sequential ints (OSM id in `osm_way_id`) | sequential ints (from 0) |
| Lane detail | **per-lane rows** with `turn`/`allowed_uses`(bus/bike)/`width` from OSM tags + offset geom | lane **count** only | **meso/micro lane cells** — routable, with lane-change connectors, geometric offset |
| Movements | 4,503, from the **turn-restriction-respecting** `edge_graph` | none | 4,558, **geometric**, with lane ranges + direction codes + geometry |
| Intersection consolidation | no (preserves OSM topology) | **yes** | **yes** |
| Multi-resolution meso/micro | no | no | **yes** |
| Signals / curb tables | `signal_controller` + `curb_seg` | node `ctrl_type` only | node `ctrl_type` only |
| Multimodal | 3 modes in one file, shared `edge_id` | one network type per run | one network type per run |

**Read-out — they solve different problems:**

- **osm2gmns is the deeper *modelling* tool.** Its 0.7.x line builds **multi-resolution** networks
  (meso = lane-level, micro = cell-based with lane-change connectors — 35k micro links here) that are
  microsimulation-ready, consolidates complex intersections, fills capacity defaults, generates
  movements with lane ranges + direction codes, and integrates with DTALite / grid2demand. If the
  goal is a ready-to-simulate model, it leads. (The current pip release **1.0.1** is a leaner C++
  rewrite that emits only node+link.)
- **duckOSM is the richer *network extract*.** It keeps a **stable, joinable `link_id`** (osm2gmns
  renumbers every run), carries **per-lane OSM semantics** (bus/bike lane, turn arrow, width — from
  `turn:lanes`/`bicycle:lanes`/`psv:lanes`/`width:lanes`, which osm2gmns' geometric lanes don't),
  derives movements from a **turn-restriction-honoring** routing graph, adds signals + curb, keeps all
  modes in **one queryable DuckDB** sharing `edge_id`, and plugs into the duckOSM spatial pipeline.

The two are complementary: osm2gmns' geometric **lane topology** vs duckOSM's **lane semantics +
stable ids**. A natural pairing is duckOSM for the stable-ID, measured-data-joined network and
osm2gmns for the micro simulation build.

## Implementation

Module `src/duckosm/gmns.py` (`to_gmns(source_db, out_path, …)`): opens a **new writable** target
`.duckdb` and `ATTACH`es the source read-only, then per mode writes the `gmns_<mode>` schema —
`config`/`use_*` as fixed tables; `node`/`link`/`geometry`/`movement`/`signal_controller` as
`CREATE TABLE AS SELECT` from `nodes`/`edges`/`edge_graph`; and `lane`/`curb_seg` from a raw-tag
parser (with shapely lane-offset geometry). `--to-csv` `COPY`s each table out (dropping the non-spec
`geom`/`turn` columns). Reads the same tables as the other exporters plus `raw.ways`/`raw.nodes` for
lane / signal / curb detail; needs the DuckDB spatial extension (+ shapely for lane offsets). See the
[export roadmap](../../product/simulation-export-targets.md).
