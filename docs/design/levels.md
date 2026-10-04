# The drawing order in the duckOSM file (`duckosm levels`)

**Status:** implemented. The algorithm is in roadstyle: `roadstyle/docs/design/levels_split_casing.md`.

Every road gets a **casing number** (in three parts: start, main, end) and a **fill number**. They say which road is painted over which: the casings and fills are painted
position by position, lowest number first. roadstyle computes the numbers (`compute_levels`) and can store them in the file (`save_levels`). `duckosm levels` is the command
that does both for a built database, so that a map or another tool can read the numbers instead of computing them again. Computing takes long on a large network.

## The command

```
duckosm levels DB [--order class|none] [--band-dist M] [--head-m M] [--max-level N] [--margin X] [--time-limit S] [--no-min-positions]
```

| Argument | Default | Meaning |
|---|---|---|
| `DB` | | a built duckOSM file; it is changed in place (the schema `visualization` is written) |
| `--order` | `class` | `class`: where roads meet, the higher road class is painted later; `none`: no such wish |
| `--band-dist` | 10 | metres: two roads closer than this, with different bands, are a stack pair |
| `--head-m` | 5.0 | metres: the length of each casing head |
| `--max-level` | 20 | the range of the numbers before the shift to the ground |
| `--margin` | 1.0 | how much later a road is painted where one must be painted after another |
| `--time-limit` | 60 | seconds for each solve of the slack stages (the main solve has no limit) |
| `--min-positions` / `--no-min-positions` | on | also minimise the span of the numbers (roadstyle's `min_positions`): fewer positions, so fewer layers in a page, a little less compaction |

The options are those of roadstyle's `compute_levels`, with its names and defaults. The command needs roadstyle with its solver: `pip install "duckosm[levels]"`. Without it, the command stops with that message.

## The roads

The order depends on all the roads near each other, so the numbers are computed on the roads of **all modes together**, one row per `edge_id`, as mapstyle's `load_roads` builds them:

- Every mode that is in the file contributes its `edges` and its `private_edges` (`driving`, `walking`, `cycling`). A file with none of them is refused.
- One row per `edge_id`. Its `highway`, `layer`, `bridge`, `tunnel` and geometry are those of the first row, taking the modes in the order driving, walking, cycling, and `edges` before `private_edges`.
- The rows are in `edge_id` order, so that the result does not depend on the order of the modes.
- A road with no `highway` (a ferry) takes no part in the class order: no wish is made for it, with any road. It still gets its casing and fill numbers.

### A road and its reverse row

The optimization is for **roads**. A two-way street is two rows in the file, a road (`is_reverse` false) and its reverse row (`is_reverse` true): the same line the other way round, the same OSM way, `source` and `target` swapped.
Giving roadstyle both would put every street twice, one on top of the other. The mode networks do not all flag an edge the same way (an edge can be a road in `walking` and a reverse row in `cycling`): a row is a reverse row when it is `is_reverse` in **every** table it is in. So:

- the numbers are computed on the roads, **without** the reverse rows;
- a reverse row takes the **numbers of its road** (the row of the same `osm_id` with `source` and `target` swapped; the smallest `edge_id` if there are several), with the **two heads swapped**: a casing number belongs to the end of the line it is given for, and the reverse row's line runs the other way, so its start head is its road's end head and its end head is its road's start head; the main part and the fill are the same;
- the stored table still has **one row for every `edge_id`** (the readers look the edges up by `edge_id`, and `edge_levels_meta.n_edges` / `edge_hash` are those of all the rows, as before);
- a reverse row with no road in the file is an error that gives the number of such rows and the first ids. Nothing is guessed.

## The band

The **band** of a road says whether it is under, on, or over the ground. It is the input of the optimization:

- Its level from the tags: the OSM `layer` if that is a number, else 1 for a bridge, −1 for a tunnel, else 0 (`duckosm.crossings._level`).
- Except for a path (`footway`, `path`, `cycleway`, `steps`, `pedestrian`, `bridleway`, `corridor`) with a `walk_type`: a `sidewalk` has band −1 (under its street) and a `crossing` has band 1 (over it).
  The column `walk_type` exists only when the walking network was built with it; where it does not, the tags decide.

## What the command does

1. Read the roads as above (read-only, with the spatial extension).
2. `roadstyle.compute_levels(roads, method="solve", band_col="band", order=<--order>, ...)`.
3. `roadstyle.save_levels(con, levels)` on a writable connection: this writes the two tables below and replaces them if they exist.
4. Print the counts: roads, positions used, pairs given up, order wishes not kept, the solver used and the seconds.

A pair that cannot be satisfied is reported by roadstyle (a warning and `levels_info`), never hidden. Nothing else in the file is changed.

## Where

A new schema **`visualization`**, next to `driving`, `walking`, `cycling`, `features` and `raw`. It holds layers computed for drawing.

| Table | Content |
|---|---|
| `visualization.edge_levels` | `edge_id` BIGINT, `casing_start`, `casing_level`, `casing_end`, `fill_level` (INTEGER): one row for every edge of the roads above |
| `visualization.edge_levels_meta` | one row: the parameters (`method`, `head_m`, `band_dist`, `margin`, `max_level`, `band_source`, `order_source`, `min_positions`), `n_edges`, `edge_hash`, `roadstyle_version`, `created` |

`band_source` is `"band"`: the band above, which is a column the command builds. The two tables are the ones roadstyle writes; their columns and rules are in its design, section 11.

## Reading

```python
import duckdb, roadstyle as rs
con = duckdb.connect("area.duckdb", read_only=True)
levels = rs.load_levels(con, roads, band_col="band", order="class")       # roads: a table with edge_id; the expected parameters
```

The reader checks that the parameters and the edges are the ones the numbers were computed for, and stops with a message that says what differs. It never recomputes.

## Rules

- **Explicit.** The numbers are not part of `duckosm build`: a build stays fast, and the numbers depend on the options of this command.
- **No silent fallback.** A missing mode table, missing roadstyle, or a solver that finds no answer is an error that says so.
- **Rebuilds.** A rebuild of the file replaces the schemas it writes; the numbers must then be computed again. The meta table's `edge_hash` shows that the stored numbers do not belong to the new edges.
- **`duckosm info`** lists the schema `visualization` among the others.

## Not in this change

mapstyle still computes its own node levels when it draws (its `node_levels`). Making it read `visualization.edge_levels` is a later change in mapstyle.
