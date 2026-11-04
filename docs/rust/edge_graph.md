# Design: the turn restrictions and the edge graph in Rust

Status: approved by Kaveh and done on 2026-10-22 (version 0.2.0). This is the part "Later, 2nd" of
section 11 of [design.md](design.md).

**Result.** Every row of every table is equal to duckOSM, compared with all columns:

| Area | `edge_graph` (driving, walking, cycling) | `turn_restrictions` | `turn_permission` | `turn_path_restrictions` |
|---|---|---|---|---|
| Monaco, no component filter | 7,348, 36,850, 32,750 | 43 | 0 | 0 |
| Monaco, cut, with the component filter | 6,567, 34,764, 31,013 | 38 | 0 | 0 |
| Tartu (12 conditional rules, 8 with exceptions) | 74,467, 195,228, 188,839 | 81 | 19 | 5 |

`cargo test` runs the two Monaco variants (`crates/duckosm-core/tests/graph.rs`, golden file
`tests/data/monaco_graph_summary.tsv` from `tools/make_golden_graph.py`). One fix was necessary during the
test: a half of a split edge can have one node in `refs`, and then its end has no way (in the SQL,
`refs[2]` is NULL). Its Python sources are
`processors/restrictions.py` (233 lines) and `processors/edge_graph.py` (210 lines) of duckOSM.

## Why this part is next

`options.build_graph` is on by default in duckOSM, so nearly every real config needs `--allow-skipped`
with the Rust CLI today. With this part, a config with the defaults of duckOSM runs in Rust except for
H3, the base-map layers, the drawing order and the bus routes. In duckOSM with `options.engine: rust`,
the two steps move from Python to Rust too, and the Rust output then includes the graph that routing
reads.

## What duckOSM does today

**Turn restrictions** (`RestrictionProcessor`, the driving mode only):

1. From `raw.relations` with `type=restriction`, one rule for each rule key of the relation: `restriction`
   (binds all vehicles, always), `restriction:conditional` (the type before `@`, the condition in the
   parentheses after it), `restriction:<vehicle>` (`applies_to` is the vehicle). The `except` tag gives
   `except_vehicles`. A key with two colons is not a rule.
2. The members: `from_way` (role `from`, a way), `via_node` (role `via`, a node), `to_way` (role `to`,
   a way). With more than one member of a role, the largest id wins (`MAX`).
3. The synthetic rules of the OSM fixes file (`turn_restrictions:` with `from_way`, `via_node`, `to_way`,
   `restriction`, default `no_u_turn`), with the ids -1, -2, ... in the order of the file. A type that is
   not in the allow-list is skipped with a warning.
4. The map to edges goes through `way_nodes` of the mode: the way at the source end of an edge is a way of
   the mode that has `refs[1]` and `refs[2]` as neighbors (two consecutive nodes), and the way at the target
   end has the last two nodes. A rule gives a row `turn_restrictions(restriction_id, restriction_type,
   except_vehicles, applies_to, condition, via_node, from_edge_id, to_edge_id)` for each from edge that
   ends at `via_node` with `from_way` at its target end, and each to edge that starts there with
   `to_way` at its source end (`DISTINCT`).
5. A rule with a via way (one or more) becomes rows of `turn_path_restrictions(..., from_edge, via_edges,
   to_edge)`: a depth-first walk from each from edge along the edges of the via ways, at most 6 edges,
   no node two times, onto an edge of a to way.

**Edge graph** (`EdgeGraphBuilder`, every mode):

1. `edge_graph(from_edge, to_edge, via_edge, cost, uses)`: every pair of edges with `e1.target = e2.source`
   and `e1 != e2`. `via_edge` is `to_edge`, `cost` is `e1.cost_s` (NULL without the cost step), `uses`
   is `car`, `walk` or `bike` for the mode.
2. The same pairs over the edges plus the `private_edges` with `access` in (`bus`, `private`,
   `pedestrian`, `delivery`), when at least one of the two is a private edge: `uses` is the access label
   of the private edge. Two private edges with different labels give no row.
3. On a bridge, no turn back: for a pair where both edges have a `bridge` tag that is not `no` or empty,
   the heading at the end of the first edge (its last two points) and the heading at the start of the
   second edge (its first two points) are compared in a local metre frame (`x * cos(lat)`). The pair is
   removed when the turn is more than 90 degrees.
4. `turn_permission(from_edge, to_edge, allowed, vehicles, except_vehicles, condition, restriction_id)`:
   the rules that the graph cannot say. A ban for all with exceptions gives `allowed = true` for the exempt
   vehicles; a conditional ban or a ban for one vehicle class gives `allowed = false` with the class, the
   exceptions and the condition. The banned pairs of a rule are: `no_*` the named pair, `only_*` every
   other pair out of `from_edge`.
5. The rules without `applies_to` and `condition` change the graph: `no_*` removes the named pair,
   `only_*` removes every other pair out of `from_edge`.
6. duckOSM's `ComponentFilter` runs after the edge graph and removes the rows of dropped edges.

## The Rust design

- **The reader keeps the restriction relations.** `pbf.rs` gets a third item from pass 1: the relations
  with `type=restriction` (their tags, and their members as (id, type, role)). These are few (Estonia:
  23,049 relations of all types in the file). The byte reader gets the same.
- **A new module `restrictions.rs`** (driving only) does steps 1 to 5 above with the ways of the mode
  (`ModeWay.refs`, which has the same nodes as `way_nodes`) and the edges after step 7 of
  `build_network`. The synthetic rules come from `fixes.rs` (`OsmFixes.turn_restrictions`, read by
  `read_osm_fixes`; today the reader skips them). The `MAX` rule for repeated roles and the `DISTINCT`
  are kept.
- **A new module `edge_graph.rs`** does steps 1 to 5 of the edge graph. The pairs come from a sort of
  the edges by `source` and a binary search for each `target`. The bridge rule uses `atan2` and `cos`
  of the Rust standard library.
- **Order in `build_network`.** The restrictions and the graph are built before step 10 (the component
  filter), as in duckOSM. The component filter then also removes the rows of `edge_graph`,
  `turn_restrictions`, `turn_permission` and `turn_path_restrictions` whose edges are gone.
- **Output.** Four plain Parquet files for each mode: `edge_graph_<mode>.parquet`, and for driving
  `turn_restrictions_driving.parquet`, `turn_permission_driving.parquet` and
  `turn_path_restrictions_driving.parquet` (`via_edges` is a `BIGINT[]`). The `.duckdb` writer loads
  them. `Network` gets the four lists.
- **Options.** `build_graph` (default on) and `extract_restrictions` (default on) in `Options`, the CLI
  (`--no-graph`, `--no-restrictions`), the Python module and `config.rs` (the two keys leave the
  "skipped" list).
- **duckOSM with `options.engine: rust`.** `rust_engine.load_mode` loads the four tables too, and the
  steps `extract_restrictions` and `build_edge_graph` do not run in Python.

## Risks

- **Float functions.** The bridge rule compares an angle with 90 degrees. `atan2` and `cos` of DuckDB and
  of Rust can differ in the last bit, so a pair with a turn of exactly 90 degrees can differ. This is the
  same limit as the `cos` of the sidewalk rule (design section 7).
- **Ambiguous way ends.** Two ways that share the same two consecutive nodes give two candidate ways for an
  edge end. The SQL gives both and relies on `DISTINCT`; Rust does the same.
- **The depth-first walk** of the via-way rules visits paths in the order of the SQL result, which has
  no defined order. The rows are a set, so the order in the table does not matter for equality, but the
  comparison must sort.

## Test plan

- The Monaco sample has 38 restrictions of 6 types (`only_straight_on` 14, `no_u_turn` 10,
  `no_left_turn` 6, `only_right_turn` 5, `only_left_turn` 2, `no_right_turn` 1), none conditional, with a
  vehicle class or with exceptions, so `turn_permission` and `turn_path_restrictions` are empty there.
  Its driving `edge_graph` has 6,567 rows: `car` 4,937, `pedestrian` 1,141, `private` 438, `bus` 51.
  Golden files from `monaco.duckdb`: `edge_graph` of the three modes and `turn_restrictions`, every row.
- Tartu as a local golden (70 restrictions), the same tables. The conditional rules, the vehicle
  classes, the exceptions and the via ways need Estonia or the unit fixtures below.
- Unit tests with small fixtures: a `no_left_turn` and an `only_straight_on` at one node, a conditional
  rule, a rule with `except`, a via-way rule, a bridge with a ramp (the turn-back), and a bus-only edge
  (the `uses` label).

## Estimate

About 700 lines of Rust with the tests. The relations in the reader and the restrictions are the larger
half. The browser build gets the same code, because it has no C code and no files.
