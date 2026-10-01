# GMNS output: conformance to the standard

**Status:** approved by Kaveh 2026-10-01 (U-turn code: option A, empty; spec schemas vendored). Built on
branch `gmns-spec-fixes` (447b2d5 and the commit after it); not merged.

## Problem

duckOSM's `gmns` command should write what the GMNS standard
([spec](https://github.com/zephyr-data-specs/GMNS), release v0.97, schemas in `spec/`) says, so any GMNS
tool can read it. So far it was checked by eye. This audit ran the spec's own `datapackage.json` and
schemas over the `--to-csv` output (`frictionless validate`), then checked the DuckDB tables and the
spec's category lists, which frictionless does not enforce.

## Audit (Monaco, driving)

Validated with frictionless 5.19: **link, node, geometry, lane, use_definition, use_group, curb_seg
pass.** What fails or is off:

| # | Table.column | Finding | Spec says | Fix |
|---|---|---|---|---|
| 1 | `movement.mvmt_code` | 234 U-turns are coded `NBU`, `SBU`, `EBU`, `WBU` | pattern `^[NSEW][EWB][RLT]\d?$`: no `U` | see question 1 |
| 2 | `config` | header order `long_length, short_length` | `short_length, long_length` | swap the columns (values are equal) |
| 3 | `signal_controller` (CSV) | extra columns `node_id`, `control_type` | only `controller_id` | keep them in DuckDB only (add to `_CSV_EXCLUDE`) |
| 4 | `link.ped_facility` | raw OSM `sidewalk` values (`both`, `left`, `right`, `separate`, `no`) | categories: `unknown`, `none`, `shoulder`, `sidewalk`, `offstreet_path`, `crosswalk` | map (below) |
| 5 | `link.bike_facility` | raw OSM `cycleway` (all NULL in Monaco, but wrong wherever it is tagged) | categories: `unseparated bike lane`, `buffered bike lane`, `separated bike lane`, `counter-flow bike lane`, `paved shoulder`, `shared lane`, `shared use path`, `off-road unpaved trail`, `other`, `none` | map (below) |
| 6 | `lane.allowed_uses` | named `bus` / `bike` that `use_definition` lacked | every use is defined | **done**, 447b2d5 |
| 7 | `config.currency` / `version_number` | missing column; text instead of number | optional `currency`; number | **done**, 447b2d5 |

Mappings for 4 and 5 (OSM → GMNS category; anything else → `unknown` / `other`):

- `sidewalk`: `both`, `left`, `right`, `yes` → `sidewalk`; `separate` → `offstreet_path`; `no`, `none` → `none`.
  (Which side is lost: GMNS has one value per link.)
- `cycleway`: `lane` → `unseparated bike lane`; `track` → `separated bike lane`; `share_busway`,
  `shared_lane` → `shared lane`; `opposite`, `opposite_lane` → `counter-flow bike lane`; `shoulder` →
  `paved shoulder`; `no`, `none` → `none`; other → `other`.

Already fine: all primary keys unique; every foreign key to a table we write resolves; required
fields never NULL; `movement.type` and `node.ctrl_type` values are in the spec's category lists;
lane numbering (1 = next to the centre line / leftmost) is the spec's.

Kept as documented extensions, DuckDB only (the CSV drops them): `geom` columns, `lane.turn`,
`link.bridge` / `tunnel` / `layer`, `signal_controller.node_id` / `control_type`, and the
`lane_connector` table. Version: we write `0.97`, the release tag (the `datapackage.json` inside that
release still says `0.96`).

## Proposal

1. Fix 1 to 5 in `src/duckosm/gmns.py`.
2. **A conformance test** so it cannot drift: `tests/test_gmns_spec.py` writes the tiny test network
   as CSV and validates it with frictionless against the **vendored spec schemas**
   (`tests/data/gmns_spec/`: the 10 schemas we write + `shared_categories.json` + a `README` with
   the release and the Apache-2.0 licence, as the spec's repo). It also checks every categorical value
   against `shared_categories.json`, since frictionless skips those. Skipped when `frictionless` is not
   installed (`pip install "duckosm[dev]"` adds it).
3. Docs: `docs/exports/gmns.md` gets a "Conformance" section: what is checked, how to rerun it on
   your own output (the frictionless one-liner), the documented extensions.
4. Re-run on Monaco; paste the result in the docs.

Not in scope: the optional spec tables we do not write (`zone`, `location`, `segment*`, `*_tod`, the
signal timing tables); the `meso` / `micro` outputs, which follow osm2gmns' layout and are not part of
the spec; the combined `gmns_all` schema (it reuses the per-mode builders, so it follows).

## Decided (Kaveh, 2026-10-01)

1. **U-turn `mvmt_code`: empty (A).** `type = uturn` says it. The micro / meso builders copy the NULL;
   `matsim_lanes._orient` ignores empty codes, so a U-turn no longer votes for a signal phase.
2. **Spec schemas vendored** in `tests/data/gmns_spec/` (v0.97, Apache-2.0, 76 KB).

## Audit 2: more areas and modes, and the values (2026-10-01)

Built `gmns --combined` for Monaco (driving), Södermalm, Tartu and Granville Island (driving, walking,
cycling and the combined network), then ran two kinds of check.

**Schema level** (frictionless on the CSVs, as the test does): all 14 CSV sets pass, categories too.

**Value level** (what no validator sees; a script of 25 checks per mode, run on the DuckDB files):

| Check | Result |
|---|---|
| link ends on its `from` / `to` node; no loops, no missing nodes; no orphan nodes; node coordinates in range | all pass, every area and mode |
| `lane_num` is 1..n with no gaps or duplicates; every link has lane rows; every lane has a geometry | all pass |
| movements: `node_id` is the inbound link's end and the outbound link's start; lane ranges inside the link's lanes; ib and ob ranges of equal length; no duplicates | all pass |
| `curb_seg` inside the link; `geometry` table = one row per link; speeds and capacities positive | all pass |
| **`link.length` against the geometry's length** | **0.4 to 0.8 % of links are more than 2 % off, up to 23 %** (Tartu driving 226 of 28,340) |
| **`link.lanes` against the number of lane rows** | **0.02 to 0.09 % differ** (Tartu driving 18) |
| `link.lanes` empty | 0 in driving; 3 to 4 % in walking and cycling (472 of 14,717 in Södermalm walking) |

Looked at and **expected, not defects:** links with no way on or no way in (dead ends, the area's edge:
0.2 to 0.3 %); duplicate walking links (two OSM ways drawn on the same line, in the source data);
lanes shorter than 0.5 m (the link itself is under 1 m in OSM); lanes more than 10 m from the link line
(5 to 10 lane roads: the outer lane is half the road's width away).

**Causes:**

- **`length`:** `link.length` is `edges.length_m` as it is. In the source, the same value sits on pairs
  of different geometries (46.84 m on edges of 35.1 m and 58.9 m): `graph_simplifier.py` splits a loop or
  a pair of parallel edges and gives each part `length_m / 2` instead of its own length (lines 458,
  484, 617, 625). That is upstream of GMNS and also affects routing costs.
- **`lanes`:** OSM tags disagree. `lanes:forward=2` but `turn:lanes:forward=left|through|right` (3
  entries) on Aida street in Tartu: `link.lanes` is 2, the lane rows (from `turn:lanes`) are 3.

**Fixes (Kaveh: "do all", 2026-10-01):**

1. `link.length` = the geodesic length of the link's geometry (`_build_link`). Does not touch routing.
2. `link.lanes` = the number of lane rows (an `UPDATE` after the lanes are built). Fills the empty
   `lanes` of walking and cycling links too.
3. Upstream, `graph_simplifier`: `length_m` of split loop and parallel-arc halves. **Not fixed where the
   splits are, on purpose:** the splits order edges by `length_m`, so correcting it there changes which
   edges are split and with it 8 of Monaco's 3,092 `edge_id`s (tried, reverted). Downstream matchers
   pin to `edge_id`. Instead `_fix_split_lengths` runs after the ids are final and sets `length_m` to
   the length of the edge's own geometry for the edges more than 0.1 % off (18 in Monaco; every other
   `length_m` and all `edge_id`s are unchanged: checked against the existing Monaco build).
4. The value checks live in `src/duckosm/gmns_check.py` (16 checks per mode), run by
   `duckosm gmns --check` and by `tests/test_gmns_values.py`, which also breaks each thing on purpose and
   expects the matching check to fail. `tests/test_edge_identity.py` gets the length invariant.

Result: all four areas, every mode and the combined network pass all checks.
