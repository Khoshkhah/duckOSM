# Documentation structure

**Status:** proposal, for sign-off before any page moves.

Based on a full read of all 32 published pages and the 8 design notes (2026-09-30).

## Problems found

1. **Pages mix jobs.** `user_manual.md` is tutorial + reference + how-tos. Feature pages are half
   user guide, half design history ("Phase 2", "shipped 2026-07-04", "v1", test plans, file lists).
2. **Topics are scattered.** Stable edge ids: 6 pages. The build pipeline: 2 (`pipeline.md` stale,
   `architecture.md` current). GMNS: 7 pages. Road filter per mode: 4. Clean-up: 3.
3. **Facts missing or wrong** (fixed as part of the rewrite, see the list at the end).
4. **No map of the exports**: which export for which tool, and which need a GMNS db first.

## Principles

- Four kinds of page, never mixed: **tutorial** (learn by doing), **guide** (do one task),
  **concept** (how and why it works), **reference** (complete, dry, generated from code where possible).
- User pages carry no project history: no dates, phases, "v1", test plans or file lists. Those stay
  in `docs/design/` (unpublished).
- One topic, one page. Other pages link to it instead of repeating it.
- Every example runs on the Monaco sample, and was run before publishing.

## Proposed menu

| Section | Page | Made from |
|---|---|---|
| **Home** | Home | `index.md` (as now) |
| **Get started** | Install | `user_manual` Installation |
| | Your first network | `user_manual` Workflow A1–B2, one query, one route, one map (Monaco) |
| **Guides** | Prepare an area | `user_manual` A1–A3, `boundary` / `clip-pbf` options, several areas from one region (`extract`) |
| | Build a network | build options, config basics, big builds (memory, batching) |
| | Fix OSM errors | `configuration` osm_overrides + `turn-restriction-overrides` note (way rules and turn rules) |
| | Query the database | `query_cookbook` (fixed), `duckosm way`, schema-visibility tips from `multi_mode` |
| | Route | `user_manual` Routing, how to get an `edge_id` / `node_id`, across modes (usage part of `multimodal`) |
| | Draw a map | `user_manual` Visualization |
| | Add elevation | usage part of `elevation` |
| | Add admin boundaries | `admin_boundaries` |
| | Check a build | validation + `validate_geometry` (from `development`) |
| **Exports** | Overview | new: which export for which tool, and the core db → GMNS db chain |
| | SUMO | `user_manual` SUMO |
| | MATSim | `matsim_export` + `matsim_lanes_signals` (network, then lanes & signals) |
| | GMNS | `gmns_export` + `gmns_meso` + `gmns_micro` + `gmns_viewer` + `gmns_map`, with the user facts of `gmns_roundout` / `gmns_map_realism` |
| | GIS | `gis_export` (with `gis-debug`) |
| | networkx | `user_manual` networkx |
| | Experimental: OpenDRIVE, Lanelet2, railML, lane-level routing | their pages, trimmed to usage + limits |
| **Concepts** | How a build works | `architecture` diagrams + step table, merged with `pipeline` |
| | Stable edge ids | the 6 scattered explanations: hash, virtual nodes, merging, cross-mode alignment (global junctions), connectors, `edge_id_map`, v1 macro |
| | What each network contains | `multi_mode` (fixed) + dismount edges + `walk_type` / `cycle_type` + speeds |
| | Network clean-up | `connectivity_repair` + component filter + clip predicate |
| | duckOSM and OSMnx | `osmnx_comparison` (trimmed) |
| **Reference** | CLI | new: every command and option (from `duckosm <cmd> --help`) |
| | Configuration | `configuration`, regenerated from `config.py` |
| | Database schema | `data_dictionary`, completed |
| | Base-map layers | `features_schema` (without the duckmap migration) |
| | Elevation sources | the DEM catalogue part of `elevation` |
| **Development** | Development | `development` |
| | Internals | per-step SQL of `architecture`, fixed |

**Leaves the site** (to `docs/design/`): `walkthrough.md` (an old dev log), `gmns_roundout.md`,
`gmns_map_realism.md`, `lane_map.md`, and the design halves of `multimodal`, `elevation`,
`merging`, `global_junction_segmentation`, `features_schema`, `lane_routing`.

## Facts to fix on the way

- Cycling includes footways as "dismount" edges by default (5 km/h, both directions,
  `dismount` column, `cycle_type='dismount'`): undocumented; `multi_mode` is now wrong.
- `turn_restrictions:` overrides: undocumented.
- Configuration: `simplify` default is true, not false; missing `merge_segments`,
  `global_junctions`, `functional_types`, `cycling_dismount`, `build_features`, `clip_strategy`,
  `connect_snap_m`, the `multimodal:` block and 4 validation keys; `connectivity_rescue` is
  implemented, not "reserved".
- Cookbook: `source_h3` doesn't exist (`from_cell`, BIGINT); tables unqualified; "service roads
  excluded" from routing is false.
- Types: `edge_id`, cells and restriction ids are BIGINT everywhere (docs say INTEGER / UBIGINT).
- `architecture` step 3 shows the old v1 hash; `multimodal` too.
- Schema: missing `access`, `dismount`, `ways`, `way_nodes`, `virtual_nodes`, `edge_id_map`,
  `boundary`, `global_junctions`; clip builds have no `raw`.
- GMNS: lane offset formula outdated; meso/micro pages call implemented features "proposed" /
  "out of scope"; `matsim_export` calls lanes/signals out of scope; lanes are XSD-validated, not DTD.
- OpenDRIVE: "no junctions / no elevation" is outdated; `--junctions` needs a GMNS db (say so).
- Dead links from `data_dictionary` / `development` into the unpublished `docs/design/`.
- `> [!TIP]` alerts don't render in MkDocs: use `!!! tip`.

## How to do it

Section by section, each reviewed before the next: Get started → Guides → Exports → Concepts →
Reference → Development. Page moves keep redirects out of scope (the site is days old).
