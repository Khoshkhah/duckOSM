# Design: the drawing levels in Rust

Status: done, 2026-10-22 (the results are in "The equality check" and "Time and memory"). Kaveh's decision: "we
can have simple tag version as default but we should have real optimization version like what we have already
for python version too." His answers to the design questions are at the end.

This document is about the table `visualization.edge_levels` (and `edge_levels_meta`), group 2 of
[tables.md](tables.md). It ports roadstyle's solver, `compute_levels(method="solve")`, to Rust, next to the tags
rule.

## The short version

- There are two methods. `tags` is the default: roadstyle's `compute_levels(method="tags")`, the same rule as the
  SQL `TAG_LEVELS` of the page (roadstyle's `roads_sql.js`, `www/map/roadstyle/`), in Rust. `solve` is roadstyle's optimization.
- roadstyle solves in two ways: a minimum-cost flow (OR-tools) when all the rules can hold, and HiGHS (a MILP in
  four stages) when some rules must be broken. Monaco needs the HiGHS way: 16 order wishes cannot hold.
- A test on Monaco (section "The finding") shows that each HiGHS stage is a linear program with a whole-number
  answer. Thus a minimum-cost flow can solve each stage. The Rust port is one algorithm in pure Rust, for all the
  cases: no OR-tools, no HiGHS, no WebAssembly build of a C++ solver.
- The code was in `duckosm-core` (the module `levels`). Since 2026-10-23 it is in roadstyle's own Rust engine
  (`roadstyle_rs::levels`, roadstyle repository `rust/`). Since 2026-10-26 it is the crate `roadstyle-levels` at roadstyle `f0a161b`, with the crossing rule (a road that crosses another at one level keeps its order). The columns of `edge_levels` do not change. `duckosm-build/src/levels.rs` (in `duckosm-core` until
  2026-10-24) only gives it duckosm's edges. The CLI, the Python package and the page all have it.
- Tartu (flow only): every row is equal to roadstyle's. Monaco (stages): the values of the stages are equal, and
  13,288 of 13,298 rows are equal. The other 10 edges have other numbers of the same quality.
- Time of a solve, natively: Monaco 0.45 s (roadstyle 16.6 s), Tartu 4.1 s (roadstyle about 21 s). The tags rule
  takes 0.05 s on Tartu.

## What the levels are

A road is drawn in two layers: the casing (the dark edge line) and the fill (the color on top of it). Each edge
gets four numbers: `casing_start`, `casing_level`, `casing_end` (the casing in three parts: a head at the start,
the main part, a head at the end) and `fill_level`. The map paints number by number, the lowest first. At each
number it paints all the casings, then all the fills. 0 is the ground.

With these numbers, a bridge covers the road under it, a tunnel goes under the road over it, and two roads that
meet join with no casing line across the joint. duckOSM's table also has `head_start_m`, `head_end_m` (the length
of each head as drawn, 5 m) and `cap_start`, `cap_end` (the shape of the line end: `round`, `square` or `flat`).

## What the solver does

The source is `roadstyle/src/roadstyle/levels.py` (816 lines). duckOSM's `duckosm levels` calls it through
`roadstyle.level_area` (`make_area`, then `solve_area`) with the default parameters. The steps:

1. **The edges.** All the edges of all modes, `edges` and `private_edges`, one row for each `edge_id`
   (duckOSM's `load_roads`). Monaco: 13,298 edges. Tartu: 63,965.
2. **The roads.** Edges with the same line (the same points, in one direction or the other) and the same
   `highway`, `tunnel`, `bridge` and `layer` are one road. The two directions of a street are one road. Monaco:
   6,830 roads. Tartu: 32,741.
3. **The band of each road.** A non-zero `layer`, else 1 for a bridge, -1 for a tunnel, else 0.
4. **The priority of each road.** The class order of the map (roadstyle's `class_order` of `highway`), plus 300
   for a roundabout, 200 for a tunnel, 100 for a bridge.
5. **The relations** (in metres, in the UTM zone of the area):
   - `meet`: two roads have an end at the same node.
   - `stack`: two roads of different bands come within 10 m of each other. A pair that meets is a stack only if
     the roads come within 10 m of each other at a distance of more than 10 m from the nodes they share. The road
     of the higher band is the upper road.
   - `order`: two roads meet and have different priorities (and the same band, or they only meet). The road with
     the higher priority wants its fill on top. This is a wish, not a rule.
6. **The casing parts.** Each road has a start head and an end head, 5 m long, and the main part between them. A
   stack gets one row for each part of the upper road that crosses the lower road. A head that meets the lower
   road (or a road that meets the lower road) is not lifted.
7. **The numbers.** Each road gets a fill number `b` and three casing numbers `a`. The rules:
   - H1: each casing part is not above the fill of its road (`a <= b`).
   - H2: where two roads meet, the head of each one is not above the fill of the other.
   - H3 (stack): the lifted casing part of the upper road is above the fill of the lower road (`a_upper >= b_lower + 1`).
   - H4 (order wish): the fill of the higher road is above the fill of the lower road (`b_high >= b_low + 1`).

   Each rule has the form `x_i - x_j <= w`, with `w` 0 or -1. The solver minimizes, in this sequence:
   1. the stack rules that it must break,
   2. the order wishes that it must break,
   3. the "near" rules (off in duckOSM),
   4. the cost: the sum of `b - a` over all the casing parts, plus a large weight on the span of the numbers.

   Stage 0: if all the rules can hold, one minimum-cost flow (OR-tools) solves the problem. The numbers are then
   the shortest distances in the flow's residual graph. Else, HiGHS solves the stages 1 to 4 as MILPs. Each stage
   keeps the best value of the stage before it.
8. **The result.** The casing number that most edges have becomes 0 (the ground). The numbers are then made
   consecutive (negative to -1, -2, ..., positive to 1, 2, ...). Each edge gets its road's numbers. An edge that runs
   against its road gets the two heads in the other sequence. The heads are 5 m. A cap is `round`, but `square` at
   the dead end of a two-way road.

duckOSM writes `edge_levels_meta` with `source` = `area` and the path of the level area folder. The level editor
of roadstyle reads the folder and changes the numbers with your edits (`level_edits`, `level_heads`, `level_caps`).

## The finding: the stages need no MILP

roadstyle uses a MILP for the stages because an LP gave fractional numbers ("a staged problem has fractional
corners", in `_solve_intervals`). I did a test on Monaco: the same stages as an LP (no integer variables), with
the row "keep the stage before" at its exact value, not at the value plus roadstyle's tolerance of `1e-7`:

| Monaco, no edits | Stage 1 | Stage 2 | Stage 4 | Fractional numbers |
|---|---|---|---|---|
| MILP (roadstyle) | 0 | 37 | 6,562,870 | 0 |
| LP, with the tolerance `1e-7` | 0 | 37 | 6,562,866.97 | 13,796 |
| LP, exact | 0 | 37 | 6,562,870 | 0 |

The tolerance is the cause of the fractions, not the problem. This is also the theory. Every rule is
`x_i - x_j <= w` (a network matrix). A broken rule adds one slack variable to its row only. The best answers of
a stage are the answers that agree with the stage's flow (complementary slackness):

- A rule with flow on it holds as an equality.
- A rule of the stage with flow below its capacity must hold (its slack is 0).
- A broken rule (flow at its capacity) holds in the other direction (`x_i - x_j >= w`).

These are again rules of the form `x_i - x_j <= w`. Thus each stage is a minimum-cost flow on the rules that the
stage before leaves, and its answer is whole numbers. The script of the test is `tools/levels_lp_check.py`.
Edits do not change this. With Kaveh's 58 edits (`monaco.levels/edits.csv`), the exact LP and the MILP also
agree at each stage: 0, 46 and 7,382,499, with no fractional number.

## The Rust design

### Where it runs

Since 2026-10-23 the files below are in roadstyle's repository (`rust/roadstyle-rs/src/levels/`), as one part of
roadstyle's Rust engine; duckosm-rust takes them at a fixed commit (`roadstyle-rs` in `duckosm-build/Cargo.toml`
and `duckosm-parquet/Cargo.toml`; `duckosm-core` has no roadstyle dependency since 2026-10-24).
The rows did not change (Monaco and Tartu, every row).

The module `crates/duckosm-core/src/levels/` (pure Rust; the one new dependency is `sha2`, for the hash of the
meta row):

| File | What it does | Lines |
|---|---|---|
| `mod.rs` | `compute(edges, method)`: the edges of all modes, the tags rule, the rows of a solve (the ground, the compaction, the heads and the caps), the hash | 259 |
| `input.rs` | the roads, the bands, the priorities, the relations and the casing parts (roadstyle's `level_input`), with the UTM projection and the plane geometry | 625 |
| `flow.rs` | a minimum-cost flow with whole numbers (`i64`): the primal-dual algorithm (Dijkstra, then a push-relabel maximum flow on the arcs of reduced cost 0), and its check | 320 |
| `solve.rs` | the stages, the rules that each stage leaves, and the numbers from the last flow | 236 |

`duckosm-build` writes the two tables after the other area tables (`write_levels` in `area.rs`), as
`edge_levels_visualization.parquet` and `edge_levels_meta_visualization.parquet`, and the `.duckdb` file gets the
schema `visualization`. The page loads them, puts them in its zip and in `load.sql`, and its map reads them.

The flow is not the network simplex of the design: the primal-dual algorithm is shorter and simple to check, and
the costs of the levels are small (0, -1 and the span), so it needs few rounds of Dijkstra (2 to 8 on Tartu).

The last stage is done in two steps (2026-10-23). Its cost is the sum of `b - a` plus a large weight on the span
of the numbers. The weight is larger than any sum, so the best answers are the answers with the least span, and
among them the least sum. The least span is the longest chain of rules: one Dijkstra on the reduced costs gives
it (`least_span` in `solve.rs`). Then the box of the numbers gets that size, and the flow carries only the casing
parts. The set of best answers is the same, so the numbers and the stage values do not change. Before, the weight
was a supply at the two span nodes. That sent millions of units through the whole graph.

The maximum flow in each round is a push-relabel (FIFO, with a global relabel by a backward search). Dinic needed
one search of the whole graph for each distinct path length: 274 phases in the last stage of Burnaby. A
push-relabel has no phases.

The geometry of `input.rs` must give the same pairs as shapely and GEOS. Thus it copies their rules, not only
their idea:

- The metres: the UTM zone of the center of the area's box (as geopandas' `estimate_utm_crs`), and the transverse
  Mercator series of Krüger to the 6th order. The test `utm_as_pyproj` agrees with pyproj to 1 mm.
- The roads near each other: an R-tree (`rstar`, already in `duckosm-core`) and the exact distance between two
  lines.
- The circle around a node (GEOS `buffer`): a polygon of 32 sides, with the same vertices as GEOS. A real
  circle gives other answers at the limit of 10 m.
- The part of a line out of the circles, the crossing of a part and a road (`_crosses`, with circles of 0.5 m
  at the ends), and the cut of a line at 5 m (shapely's `substring`).

The flow finds the best value of each stage. For the numbers, the solver uses roadstyle's rule of stage 0 at all
stages: the shortest distances from an extra node, shifted to start at 0. There is only one such answer for a
given set of rules. Thus the result does not change with the flow algorithm or the sequence of the edges.

After each solve, the flow is checked: it keeps the supplies and the capacities, and no arc has a reduced cost
against it (complementary slackness). This is a proof that the flow is the best. A failed check is an error.

### The options

- CLI: `--levels tags|solve|none`. The default is `tags`.
- A duckOSM config: `options.levels` is on by default in duckOSM and solves there. Thus on (or no option) gives
  `solve`, and `false` gives no table, as in duckOSM. `--levels` replaces the config's choice.
- Python package: `build(..., levels="tags")`, with the same three values.
- WebAssembly: a last argument `levels` of `build`, `build_area` and `build_osm_json` (`undefined` is `tags`).
  `run_node.mjs` takes it as its fifth argument.
- The page: step 4 (**Build**) always uses "Optimized" (roadstyle's solver). Until 2026-11-04 it had a choice, "From the tags (fast)"
  or "Optimized (slower)", with "Optimized" as the default since 2026-10-24; Kaveh always used the solver, so the choice is gone. The CLI keeps `tags`. The map reads `visualization.edge_levels`; it computes `TAG_LEVELS` only for a build
  without that table.

### What the meta row says

| Column | `tags` | `solve` |
|---|---|---|
| `source` | `tags` | `solve` |
| `area` | NULL | NULL |
| `n_edges`, `edge_hash` | as roadstyle (`sha256` of the sorted `edge_id` values, 32 characters) | the same |
| `roadstyle_version` | the roadstyle version that the port follows (0.24.2) | the same |
| `created` | the time of the build | the same |

The page's check (at roadstyle `083224c`: `levels_keyword` in roadstyle-io's `duckosm.rs`) accepts `area`, `tags`
and `solve`. roadstyle's Python level editor (`load_area_levels`) accepts `area` only. To edit the levels of a Rust build:

- in the page, card 8 **Edit levels** ([app.md](app.md), "Step 8: edit the levels"): roadstyle's level editor solves in
  the page, and **Apply** replaces the two tables (`source` = `area`) and writes `level_edits`, `level_heads` and
  `level_caps`;
- natively, run `duckosm levels` on the file: it makes a level area and replaces the two tables.

### Not in the port

These parts of roadstyle are off in duckOSM, or only roadstyle's level editor has them. Each one can come later in the
Rust build, when a user needs it:

- the edits, heads and caps (`level_edits`, `level_heads`, `level_caps`) in a build. The page's editor (card 8) now
  makes them and solves with them, with roadstyle's own solver (roadstyle-wasm), but `duckosm-rust` does not read them in
  a build;
- `auto_ends` (heads and caps from the geometry; duckOSM uses the fixed 5 m and the round caps);
- `near_rules`, `band_col` and an `order` other than `priority`;
- the local solve of the editor (`solve_local`, `fixed`): the page's editor has it, the build does not need it;
- the time limit: the flow has no time limit, because it always ends.

The levels of `duckosm-core` come from roadstyle `aba5884` (`crates/duckosm-core/Cargo.toml`). The page's map and editor
use roadstyle `083224c` (`build.sh`). At `083224c` the levels are the crate `roadstyle-levels`, with another API and
another `ROADSTYLE_VERSION`. A move of `duckosm-core` to it changes the code and the meta row, and needs a new check
against the golden files.

## The equality check

The golden `monaco.duckdb` of duckOSM is not the correct reference: its levels contain Kaveh's 58 edits and a
local solve of the editor. Tartu's golden has no levels. Thus the reference is a new solve by roadstyle on the
edges of the Rust file:

```bash
/home/kaveh/projects/duckOSM/.venv/bin/python tools/compare_levels.py out/monaco.duckdb
```

`tools/compare_levels.py` runs roadstyle 0.24.2 (`load_roads`, `level_input`, `solve_levels` with duckOSM's
defaults, no edits) and compares the rows. Then it checks both results against roadstyle's rules (its pairs): the
hard rules, the stack parts that are not lifted and the order wishes that are not kept. With `source` = `tags`, it
compares the rows with `compute_levels(method="tags")`.

The results (2026-10-22). Monaco: `duckosm-rust --config config/sample_monaco.yaml --allow-skipped` in the duckOSM
folder. Tartu: the cut file `pbf/tartu.smart.b433d09b44.osm.pbf` with `data/boundaries/tartu.geojson` and duckOSM's
OSM fixes.

| Area | Method | Rows equal to roadstyle's | Rules of roadstyle (hard broken / stack parts not lifted / wishes not kept) |
|---|---|---|---|
| Monaco | `tags` | 13,298 of 13,298 | |
| Monaco | `solve` | 13,288 of 13,298 | roadstyle 0 / 0 / 16, duckosm-rust 0 / 0 / 16 |
| Tartu | `solve` | 64,828 of 64,828 | roadstyle 0 / 0 / 0, duckosm-rust 0 / 0 / 0 |

On Monaco, the values of the stages are the values of roadstyle's MILP: `[0, 37, 6562870]` (the CLI prints them).
The 10 edges that differ have other numbers of the same quality: many answers have the same best value, and
HiGHS and the flow choose different ones. Kaveh: equality "is not important and is not possible".

`cargo test` has:

- unit tests of the flow, the stages (a bridge over a road, a ring of wishes that cannot all hold), the UTM
  projection, the cut of a line by circles and the crossing test;
- `browser_area_build_equals_native_build` (duckosm-wasm): the browser build with `solve` writes the same files as
  the native build, byte for byte;
- `monaco_levels` (duckosm-build): the summary of both methods on the Monaco sample, against the values above.

## Time and memory

roadstyle (this machine, 2026-10-22, the whole Python process):

| Area | Edges | Roads | `level_input` | Solve | Way | Memory (peak) |
|---|---|---|---|---|---|---|
| Monaco | 13,298 | 6,830 | 2.3 s | 14.3 s | stages (4 HiGHS solves) | 630 MB |
| Tartu | 63,965 | 32,741 | 11.1 s | 9.7 s | flow (OR-tools, then a slow relaxation in numpy) | 670 MB |

duckosm-rust (2026-10-22, release build, three modes; other sessions used the machine, load average about 25, so
each time is the best of three runs). Native: the step `levels` of `--timing`. WebAssembly: the whole build in Node
(`run_node.mjs`, one thread), so the time of the levels is the difference to the build with `tags`.

| Area | Method | Native | WebAssembly (whole build) | Memory |
|---|---|---|---|---|
| Monaco (`tests/data/monaco.osm.pbf`) | `tags` | 0.007 s | 0.33 s | 35 MiB (WebAssembly) |
| Monaco | `solve` | 0.45 s | 0.82 s | 35 MiB (WebAssembly) |
| Tartu (`tartu.smart.b433d09b44.osm.pbf`, 64,828 edges) | `tags` | 0.05 s | 1.2 s | 96 MiB (WebAssembly) |
| Tartu | `solve` | 4.1 to 4.6 s | 5.3 s | 124 MiB (WebAssembly); native 235 MB, against 163 MB with `--levels none` |

The page (headless Chromium, `e2e.py`, the Monaco sample with its boundary, 8 threads): the build takes 0.54 s with
`tags` and 1.0 s with `solve`; the map draws from the stored levels in both cases.

Against the targets of the design: Monaco is less than 1 s, and WebAssembly is not slower than three times native.
Tartu missed its target of 3 s with Dinic; with the changes of 2026-10-23 it takes 1.0 s (below). The memory of a solve on
Tartu is 72 MB more natively (the target was 50 MB) and 28 MiB more in WebAssembly.

The faster solver (2026-10-23: the least span first, and a push-relabel; three modes, release build). The rows are
equal to the rows of the solver before, on every row: Monaco 13,298, Burnaby 64,314 (driving) and 157,808 (three
modes). Tartu: 64,828 of 64,828 rows equal to roadstyle's (`tools/compare_levels.py`).

| Area | Roads | Native before | Native after | WebAssembly before (whole build, Node) | WebAssembly after |
|---|---|---|---|---|---|
| Tartu | 32,546 | 4.1 s | 1.0 s | | |
| Burnaby (South Coast file, cut with osmium) | 79,199 | about 20 s | 3.1 s | 21.8 s (tags: 2.5 s) | 5.2 s |

## Risks

- **Points at the limit.** The pairs depend on distances of 10 m and 0.5 m. A small difference in the projection
  or in the circles can add or remove a pair. On Monaco and Tartu, all pairs agree (the hard rules of roadstyle's
  pairs hold on the Rust result).
- **The UTM zone.** geopandas chooses the zone from the center of the box. An area across two zones can get the
  other zone in pyproj's database. That changes the metres a little, not the rules.
- **The time of large areas.** Before 2026-10-23, the last stage took 260 phases of Dinic on Burnaby (17 s, 79,199
  roads). The two steps of the last stage and the push-relabel make it 3 s (see "Time and memory").

## Kaveh's answers (2026-10-22)

1. **Monaco's numbers.** "equality is not important and is not possible". The values of the stages are equal;
   the rows are not all equal.
2. **The meta row** (`source` = `tags` or `solve`, `area` = NULL): "good idea".
3. **The edits.** "first we create without editor, then we add editor to the page (the main page need redesign for
   support many things not only create a duckosm db)".
4. **The config** (`options.levels`): no preference. It follows duckOSM: on solves, `false` writes no table.
