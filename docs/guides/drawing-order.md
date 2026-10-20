# Store the drawing order

```bash
pip install "duckosm[levels]"
duckosm levels monaco.duckdb                 # the level area monaco.levels/, solved, written into the file
duckosm levels-edit monaco.duckdb            # optional: fix places by hand, with mapstyle's base map and every road; each Apply writes into the file
```

When roads cross or meet, a map must decide which is painted over which: a bridge over the road under it, a street over its sidewalk, a junction without a ring between the roads that meet.
The map does it with two numbers for every road: a **casing number** (for the outline, in three parts: start, main, end) and a **fill number** (for the colour). The casings and fills are painted number by
number, lowest first, and at each number all casings before all fills. 0 is the ground.

`duckosm build` hands the roads to [roadstyle](https://khoshkhah.github.io/roadstyle/guides/levels/) as its last step, which works them out exactly as for any roadstyle map, and stores the result in the schema
`visualization` of the file, so that a map or another tool reads it instead of computing it again. The file is always current: **a rebuild keeps your edits** (they are tables in the file) and solves again with them.
For a big area the step takes long: `options.levels: false` (`build --no-levels`) skips it, and `duckosm levels` makes it later.

## The level area

The roads of **all the modes together** (`edges` and `private_edges` of `driving`, `walking` and `cycling`) go into one **level area**, a folder next to the file (`monaco.levels/`, or `--area DIR`):

| File | Whose | Holds |
|---|---|---|
| `roads.parquet`, `pairs.csv` | made by each run | the solver's input: one row per road, the pairs of roads that cross or meet |
| `edits.csv`, `heads.csv`, `caps.csv` | **yours**; a copy of the tables in the file | your changes: pairs added or switched off, head lengths and end shapes per road end. The tables are the master: each run writes these files from them |
| `levels.csv`, `levels_info.json` | made by each solve | the result, and what the solver says about it |
| `area.json` | made once | the file the area belongs to |

Your edits are in the file itself (`visualization.level_*`, below): a rebuild carries them over, and the folder is only a work area. A file of the folder that differs from the table is kept as `*.csv.before` when it is written again. A row that names a road that is not in the file any more is ignored with a warning and kept in `visualization.level_unmatched`; the next build tries it again. How the solver decides, and the editor:
[Which road is on top](https://khoshkhah.github.io/roadstyle/guides/levels/).

## What it writes

| Table | Holds |
|---|---|
| `visualization.edge_levels` | one row for every `edge_id`: `casing_start`, `casing_level`, `casing_end`, `fill_level`, and its ends as drawn: `head_start_m`, `head_end_m`, `cap_start`, `cap_end` |
| `visualization.edge_levels_meta` | one row: where it came from (the area), the number of edges and a hash of their ids, the roadstyle version, the time |
| `visualization.level_edits`, `level_heads`, `level_caps` | your `edits.csv`, `heads.csv` and `caps.csv`, copied at each solve, so the database holds everything. A new area folder gets them back from here (needs a roadstyle that has `restore_area_inputs`) |

Both tables are replaced at each solve, by `duckosm levels` and by the editor. A stack of roads that cannot all be kept is counted in the message, never hidden.

## Read it

```python
import duckdb, roadstyle as rs
from duckosm.levels import load_roads

con = duckdb.connect("monaco.duckdb", read_only=True)
levels = rs.load_area_levels(con, load_roads("monaco.duckdb"))
```

`load_area_levels` checks that the edges are those the numbers were solved for, and stops with a message if not. It never recomputes. After a rebuild of the file, run `duckosm levels` again.

Options: [`duckosm levels`](../reference/cli.md#levels).
