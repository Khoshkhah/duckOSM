# Design: the lane profile in Rust

Status: done, 2026-10-22 (the result is in "The result" at the end). Kaveh approved the design and the two
answers below ("yes. go ahead").

This document is about the table `driving.lane_profile`, group 2 of [tables.md](tables.md). It ports duckOSM's
lane rules (`src/duckosm/processors/lane_profile.py`, 164 lines, and `_inherit_lanes` in `gmns.py`) to Rust.
duckOSM's design is `docs/design/gmns_lane_profile.md`, and its reference is the `lane_profile` section of
`docs/reference/database.md`.

## The short version

- The table has one row for each lane of each driving edge (`driving.edges`) and each bus-only edge
  (`driving.private_edges` with `access = 'bus'`), from left to right. Monaco: 3,176 rows on 2,783 edges.
- The rules are pure functions of the edge, the OSM tags of its way and four small sets. The port is one new
  module, `duckosm-core/src/lanes.rs`, with no new crate.
- The build calls it after all modes, as duckOSM does, and writes `lane_profile_driving.parquet`. The CLI, the
  Python package and the page get the table from the same code.
- The check: every row equal to duckOSM's Monaco golden (`EXCEPT ALL` in both directions), and the same check on
  Tartu.

## The table

| Column | Type | Meaning |
|---|---|---|
| `edge_id` | BIGINT | the edge |
| `lane_num` | INTEGER | 1 is the left-most motor lane, 1..n are the motor lanes, -1 is a bike lane to the left of them, n+1 a bike lane to the right of them |
| `use` | VARCHAR | `auto`, `bus`, `bike` or `bus,bike` |
| `width_m` | DOUBLE | the width of the lane |
| `turn` | VARCHAR | the entry of the lane in `turn:lanes`, or NULL |
| `source` | VARCHAR | the tag or the rule that put the lane there with its use |
| `width_source` | VARCHAR | `width:lanes`, `cycleway:*:width` or `default` |

Monaco, by `source`: `default` 1,593, `lanes` 1,409, `override` 84, `cycleway:left` 46, `access=bus` 18,
`bus:lanes` 8, `lanes:forward` 8, `cycleway:right` 3, `lanes:backward` 3, `inherited` 2, `turn:lanes` 2. All
widths are `default` in Monaco: no way has `width:lanes`.

## What the rules do

For each edge of `driving.edges`:

1. **The source of the lane count.** `override` when a rule of the fixes file sets `lanes`, `lanes_forward` or
   `lanes_backward` on the way. Else `inherited` when step 2 changes the count. Else `lanes:forward` (or
   `lanes:backward` on a reverse edge) when that tag has a value. Else `lanes` when that tag has a value. Else
   `default`.
2. **Inheritance** (`_inherit_lanes`). A one-way edge whose way has no lane tag (`lanes`, `lanes:forward`,
   `lanes:backward`, `turn:lanes`, `width:lanes`) and no fixes rule for its lanes takes the count of the edge it
   continues. That edge must be the only other edge at the node, one-way, of the same class (the `highway`
   value before `;`, without `_link`), point the same way and have more lanes. If no edge leads in, the edge
   that it leads into can give the count. The rule repeats, at most 6 passes, so that a cut way gets the count
   piece by piece.
3. **The motor lanes** (`lanes_of`). The count is the largest of: the number of `turn:lanes` entries, the
   edge's `lanes` (or the inherited count), and 1. When `turn:lanes` has more entries than `lanes`, the source
   is `turn:lanes`. Each lane is `auto`, `bike` (from `bicycle:lanes`) or `bus` (from `psv:lanes`, then
   `bus:lanes`). The width comes from `width:lanes`, else from the class: 2.5 m for `service`, 3.0 m for
   `residential`, `living_street` and `unclassified`, else 3.25 m.
4. **The arrows.** A lane gets its `turn:lanes` entry only on the edge that ends where its way ends, in the
   direction of the edge. An empty entry and `none` give NULL.
5. **Bus lanes by count or by side.** If no lane is a bus lane yet, `lanes:bus` (or `lanes:psv`) gives the
   number of bus lanes. Else `busway:<side>`, `busway:both` or `busway` with the value `lane` gives one. The
   bus lanes are the right-most motor lanes. A count that is equal to or more than the lanes changes no lane.
6. **A bus lane that bikes share.** `cycleway:<side>`, `cycleway:both` or `cycleway` with the value
   `share_busway` makes the right-most bus lane `bus,bike`.
7. **Bike lanes.** They are not in `lanes`. If no lane is `bike` from `bicycle:lanes`, `cycleway:<side>`,
   `cycleway:both` or `cycleway` with the value `lane` adds a bike lane n+1 on the right. On a forward one-way
   edge without `oneway:bicycle=no`, `cycleway:left` or `cycleway:both` with the value `lane` adds a bike lane
   -1 on the left, unless its `:oneway` tag is `-1` or `no`. The width is `<key>:width`, else
   `cycleway:width`, else 1.5 m.

The side is `right` on a forward edge and `left` on a reverse edge. A per-direction tag `<key>` is
`<key>:backward` on a reverse edge, and `<key>:forward` (else `<key>`) on a forward edge.

For each bus-only edge of `driving.private_edges`: one lane, `lane_num` 1, `source` `access=bus`, the width from
the first `width:lanes` entry, else 3.25 m. The use is `bus,bike` when the way has `oneway:bicycle=no`, a
`cycleway`, `cycleway:left` or `cycleway:right` with the value `share_busway`, or when `cycling.edges` has the
same `edge_id`. Else it is `bus`.

## The inputs, and where the Rust build has them

| Input | duckOSM | duckosm-rust |
|---|---|---|
| The driving edges: `edge_id`, `osm_id`, `is_reverse`, `oneway`, `highway`, `lanes`, `source`, `target` | `driving.edges` | `Network::edges` of the driving mode (`Edge`) |
| The bus-only edges: `edge_id`, `is_reverse`, `highway` | `driving.private_edges` | `Network::private_edges`, `access == "bus"` |
| All the tags of the way | `raw.ways.tags` | `Way::all_tags()` of the build's `ways` (the reader already keeps all tags for `ways.tags`) |
| The first and the last node of the way | `raw.ways.refs` | `Way::refs` |
| The arrows of the fixes (`turn:lanes:forward`, `turn:lanes:backward`) | written into `raw.ways` by each mode | `OsmFixes::set_arrows`, after each mode, on the same `ways` |
| The ways that a fixes rule gives a lane count | `osm_overrides.yaml` | `OsmFixes::ways` with `lanes`, `lanes_forward` or `lanes_backward` |
| The cycling edge ids | `cycling.edges` | `Network::edges` of the cycling mode, when cycling is built |

The build drops each `Network` after its mode. For the lane profile it keeps a small copy of the driving edges
(the eight columns above) and of the bus-only edges. After the cycling mode it keeps the set of cycling
`edge_id`s. After all modes, the arrows of the fixes are in the tags, and the rules run once. The `ways` of the
build are sorted by id, so a binary search finds the way of an edge.

## Where the code goes

- `duckosm-core/src/lanes.rs` (new): `lanes_of` and `inherit_lanes`, as in duckOSM, and
  `lane_profile(driving, bus_only, ways, cycling, overrides) -> Vec<LaneRow>`. Unit tests for the tag cases of
  the rules table (one test for each row of the rules, as duckOSM's design asks).
- `duckosm-parquet`: `write_lane_profile_to`, with the column types of the table.
- `duckosm-build/src/lib.rs`: keep the inputs during the mode loop, then write `lane_profile_driving.parquet`
  when the driving mode is built. `duckosm-wasm/src/lib.rs` has its own mode loop and gets the same change.
- `duckosm-duckdb`: `lane_profile` in `MODE_TABLES`, so the `.duckdb` file gets `driving.lane_profile`.
- `www/app/duck.js`: a description of the table. The page loads each `<table>_<schema>.parquet` file by its
  name, so SQL, the zip and `load.sql` get the table with no other change.

## The equality check

- A test in `duckosm-build/tests/lanes.rs` builds Monaco from `tests/data/monaco_sample.osm.pbf` and
  `monaco.geojson` (the same files as duckOSM's `data/sample/`) with three modes and duckOSM's fixes file, and
  compares every row with `tests/data/monaco_lane_profile.tsv`. The TSV is an export of `driving.lane_profile`
  of `/home/kaveh/projects/duckOSM/monaco.duckdb` (3,176 rows), and `tests/data/osm_fixes.yaml` is a copy of
  duckOSM's `osm_overrides/osm_overrides.yaml`. So `cargo test` needs no file outside this repository. The rows
  have no order in duckOSM, so the test compares them as sorted lists.
- `tools/compare_tables.py` gets a line for `driving.lane_profile` (`EXCEPT ALL` in both directions), for
  Monaco and for Tartu.
- `cargo test -p duckosm-wasm`: the browser build writes the same files as the native build, byte for byte.
- The page (`e2e.py`): the table is in SQL and in the zip.

## Points where the port can differ

- **The order of inheritance.** duckOSM's `_inherit_lanes` goes through the edges in the order that DuckDB
  returns them (no `ORDER BY`), and a change in a pass is seen by the next edges of the same pass. The Rust
  port goes through them in the order of `Network::edges`. A count only goes up, so both orders get the same
  result when 6 passes are enough. Monaco has 2 inherited lanes. If Tartu shows a difference, I will report
  it.
- **Numbers in tags.** Python's `float()` reads `1_000` and leading and trailing spaces; Rust's `parse::<f64>`
  does not read `1_000`. The port trims the spaces. A width of `0` gives the default in both.
- **A new way of the fixes.** If a fixes rule adds a way, duckOSM's `raw.ways` can lack it, and then the way has
  no tags. Monaco shows whether this case occurs.

## Decisions (Kaveh, 2026-10-22)

1. The code is one module in `duckosm-core`, and the build calls it after all modes. It must run after all
   modes: the arrows of the fixes get into the tags in the mode loop, and `bus,bike` needs the cycling network.
2. There is no option to turn the table off, as in duckOSM. The table is small (about one row for each edge).

## The result (2026-10-22)

- Monaco: `cargo test -p duckosm-build --test lanes` passes. All 3,176 rows are equal to duckOSM's.
- Tartu (the Estonia file cut to `data/boundaries/tartu.geojson`, three modes, no merge, duckOSM's fixes file):
  `tools/compare_tables.py` gives `ok driving.lane_profile: 30075 rows` against a build of duckOSM at commit
  `2044c4f`. The rule for the lane count gives `inherited` to 2 edges in each area.
- The page (`e2e.py` on Monaco with the boundary and the fixes file): "lane profile: 3176 lanes". Its
  `lane_profile_driving.parquet` has the same 3,176 rows as duckOSM's table. The file is in the zip, with its
  line in `README.txt` and in `load.sql`.
- `cargo test --workspace` passes. The browser build writes `lane_profile_driving.parquet` byte for byte as the
  native build does.
- None of the three "points where the port can differ" occurred in Monaco or Tartu.
- Time (`--timing`, natively): the lane profile of Tartu takes 0.13 s, with the file written. The whole build
  takes 12.2 s with `--levels tags`.
