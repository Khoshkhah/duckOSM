# Store the drawing order

Store in the database which road a map paints over which: a bridge over the road under it, a street over its sidewalk.

```bash
duckosm levels monaco.duckdb          # solve the drawing order and write it into the file
duckosm levels-edit monaco.duckdb     # fix places by hand in the level editor
```

Each road gets a **casing number** (the outline, in three parts: start, main, end) and a **fill number** (the color). A map
paints the numbers from the lowest up, and at each number all casings before all fills. 0 is the ground. The numbers come
from [roadstyle](https://khoshkhah.github.io/roadstyle/guides/levels/), the same as for each roadstyle map.

| Command | Does |
|---|---|
| `duckosm <PBF> -o DIR --levels METHOD` | the build writes the order: `tags` (a rule on `layer`, `bridge` and `tunnel`, the default), `solve` (roadstyle's solver) or `none` |
| `duckosm build` | the build solves the order as its last step; `--no-levels` or `options.levels: false` skips it (not ported to Rust yet) |
| `duckosm levels DB` | solve the order of a built database; `--area DIR` for the level area folder. It needs `pip install "duckosm[levels]"` (not ported to Rust yet) |
| `duckosm levels-edit DB` | the level editor at `http://127.0.0.1:8000/` (`--port`, `--no-open`). It writes the order into the file |

`duckosm build` keeps your edits, because they are tables in the file, and solves again with them.

## What it writes

| Table | Holds |
|---|---|
| `visualization.edge_levels` | one row for each `edge_id`: `casing_start`, `casing_level`, `casing_end`, `fill_level`, and the ends as drawn: `head_start_m`, `head_end_m`, `cap_start`, `cap_end` |
| `visualization.edge_levels_meta` | one row: the area, the number of edges and a hash of their ids, the roadstyle version, the time |
| `visualization.level_edits`, `level_heads`, `level_caps` | your edits: pairs added or switched off, head lengths, end shapes. These tables are the master copy |
| `visualization.level_unmatched` | edits for a road that is not in the file now; the next build tries them again |

`duckosm levels` uses a level area folder next to the file (`monaco.levels/`) as its work area, and writes it again from
the tables at each run. The files of the folder:
[the design of the levels](https://github.com/Khoshkhah/duckOSM/blob/main/docs/design/levels.md).

## Read it

```python
import duckdb, roadstyle as rs
from duckosm.levels import load_roads

con = duckdb.connect("monaco.duckdb", read_only=True)
levels = rs.load_area_levels(con, load_roads("monaco.duckdb"))
```

`load_area_levels` stops with a message if the edges are not those that the numbers were solved for. After a rebuild, run
`duckosm levels` again. All options: [`duckosm levels`](../reference/cli.md#levels).
