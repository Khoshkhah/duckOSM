# GMNS by work units

Status: built, 2026-11-24 (duckOSM PRs 459, 464, 465, 467 and the removal of the tiles). It replaced [GMNS by tiles](gmns_by_tiles.md).

`duckosm gmns` reads the whole area into Rust maps in some steps, and keeps those maps until the step ends. GMNS by
tiles limits that memory with map tiles, a band of 2 km and owner rules. With the lane rules
([gmns_lane_rules.md](gmns_lane_rules.md)), GMNS does not need netconvert, so the tiles are no longer necessary. This
design removes them: each step works on small, independent units and keeps only one chunk of units in memory.

## The problem

Kentucky, driving, `--lane-source rules --tile-zoom 0` (the whole area), Mac mini, with PR 450:

| Step | Time | Peak memory |
|---|---|---|
| lanes and curbs (`build_lane_curb`) | 46 s | 8.4 GB |
| movements | 11 s | 7.5 GB |
| lane rules (`lanes_from_rules`) | 51 s | 10.1 GB |
| lane ends meet (`meet_lanes`) | 7 s | 10.2 GB |
| crossings | 22 s | 8.4 GB |
| total | 141 s | 10.2 GB |

The peak stays the same with a DuckDB memory limit of 2 GB, so the memory is Rust data, not the DuckDB cache. The
causes:

- `build_lane_curb` keeps a map entry for each edge of the area: its OSM facts, its line as WKT text, its lane widths,
  its run, and the paired carriageway gaps.
- `lanes_from_rules` (`gmns_lane_rules::pairs`) reads all links, all lanes and all movements before it pairs one
  junction.
- `build_crossings` reads all lanes before it reads one crossing (PR 450 makes surfaces only near a crossing, but it
  still reads all lane rows).
- Most of the geometry work runs on one core.
- The lines come as WKT text, and the parse shows in the stack samples.

## What each GMNS part needs

Each part of GMNS is local. No part needs the whole area at once.

| GMNS part | The unit | What the unit needs |
|---|---|---|
| `node`, `link`, `lane` rows, `movement`, `signal_controller`, `location` | one row | the matching row of the network (one to one) |
| `lane.geom` (lane lines) | one run of lane pieces (`chain_runs`) | the lines and lane widths of the run, the gap to the opposite carriageway |
| lane pairs (lane rules), `lane_connector`, `geom_cut` | one junction (node) | the links at the node, their lanes and their turn tags, the movements of the node |
| `crossing`, `lane_crossing` | one crossing | the crossing line and the lanes within a few metres |
| the gap to the opposite carriageway (`paired_gaps`) | one one-way edge | the one-way edges within the road width |

## The design

### 1. SQL for the one-to-one tables

`node`, `link`, the `lane` rows without lines, `movement`, `signal_controller` and `location` come from
`INSERT INTO … SELECT` in DuckDB. DuckDB runs these on all cores and spills to disk at its memory limit. Many of these
steps already are SQL. The rest move into SQL where the logic is a join or a `CASE`.

### 2. One spatial join for each "nearby" question

Two questions need the roads near a road. Each becomes one DuckDB query that writes a small table:

| Table | Rows |
|---|---|
| `_pair_cand(edge_id, other_id)` | each one-way edge and the one-way edges whose box meets its box plus the road width |
| `_crossing_cand(crossing_id, lane_id)` | each crossing and the lanes whose box meets the crossing line plus 50 m |

The exact test (the distance along the line, the opposite direction, the side) stays in Rust, on the candidate pairs
only. No Rust grid of the whole area is necessary.

### 3. Work units by key, in chunks

The geometry and rule work runs as a stream of units:

1. A query gives the units in a fixed order, with the rows of each unit together: `ORDER BY` the unit key (the node id
   for a junction, the run id for a run).
2. Rust reads one chunk: for example 50,000 units.
3. The chunk runs on all cores (rayon). Each unit is independent.
4. The results go into the GMNS tables with the DuckDB appender, in the order of the units.
5. Rust drops the chunk and reads the next one.

The memory is that of one chunk, for any size of area. The time is about the geometry work divided by the number of
cores.

### 4. Coordinates, not WKT

The units get the coordinates of the lines as DuckDB lists of doubles (`ST_X`, `ST_Y` of `ST_Dump(ST_Points(geom))`,
or the WKB), not as WKT text.

### 5. A metre frame for each unit

Today a step turns lon/lat into metres around the first row of the area. On a large area, that frame is far from most
roads, and the metres are less correct far from it. Each unit gets its own frame, around its first point. The units are
small, so the frame is correct for all of them.

## What changes in the output

| Item | Effect |
|---|---|
| The rows and their keys | The same. |
| The geometry columns | Small changes from the frame of each unit (section 5). The golden tests check the lane lines by their properties, so they stay green. The other geometry columns can change by less than a centimetre. |
| The row order | Fixed: the order of the unit keys. Today the order of a large area changes from run to run (a join). The Monaco golden files can change for the order only. |

A fixed order and a frame for each unit change the golden files. Each step needs Kaveh's yes before its golden files
change.

## The order of the work

One step at a time. Each step is one pull request, keeps the golden tests green (or changes them only for the reasons in
the table above), and has a Kentucky measure.

1. **Lane rules and lane connectors by junction.** `gmns_lane_rules::pairs` reads one chunk of junctions, not all links,
   lanes and movements. It is the largest memory now (10.1 GB).
2. **Lane lines by run.** `build_lane_curb` reads the runs in chunks. `_pair_cand` replaces the grid of `paired_gaps`.
3. **Crossings by crossing.** `_crossing_cand` replaces the read of all lanes.
4. **The rest into SQL.** The steps that read a whole table into Rust for a join.
5. **Remove GMNS by tiles.** `--tile-zoom`, `gmns_tiles.rs` and the owner rules go when Kentucky and Île-de-France build
   by units under the memory of the tiles.

## The goal

Kentucky, driving, lane rules, Mac mini: under 2 GB of peak memory, and under 60 s. Île-de-France and France build with
the same memory.

## Not in this design

- Merged links. GMNS keeps one link for each edge, and `gmns_driving.chains` gives the chains.
- netconvert. `--lane-source netconvert` stays as it is. It needs the whole input, so it keeps GMNS by tiles until it is
  removed or until the lane rules replace it everywhere.
