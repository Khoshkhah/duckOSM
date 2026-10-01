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
