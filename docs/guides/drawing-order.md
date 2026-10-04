# Store the drawing order

```bash
pip install "duckosm[levels]"
duckosm levels monaco.duckdb        # computes the order and writes it into the file
```

When roads cross or meet, a map must decide which is painted over which: a bridge over the road under it, a street over its sidewalk, a junction without a ring between the roads that meet.
The map does it with two numbers for every road: a **casing number** (for the outline, in three parts: start, main, end) and a **fill number** (for the colour). The casings and fills are painted number by
number, lowest first, and at each number all casings before all fills. 0 is the ground.

`duckosm levels` computes these numbers with [roadstyle](https://khoshkhah.github.io/roadstyle/) and stores them in the schema `visualization` of the file, so that a map or another tool reads them instead of computing them again.
Computing takes long on a large network, so it is not part of `duckosm build`: you run it once, when you want it.

## What it reads

The roads of **all the modes together** (`edges` and `private_edges` of `driving`, `walking` and `cycling`), one row for every `edge_id`: the order depends on all the roads near each other.

The **band** of a road says whether it is under, on, or over the ground. It comes from the tags: the OSM `layer`, else 1 for a bridge and -1 for a tunnel. A sidewalk (`walk_type`) is under its street, a crossing over it.
Where roads meet, the road with the higher class is painted later (`--order class`, the default; `--order none` turns it off). A road with no `highway`, such as a ferry, takes no part in that.

Each different number is one **position**, and a map page has a set of layers for each position. By default the command asks for few positions: the casings may then lie a little farther from their fills. `--no-min-positions` leaves that out.

## What it writes

| Table | Holds |
|---|---|
| `visualization.edge_levels` | one row for every `edge_id`: `casing_start`, `casing_level`, `casing_end`, `fill_level` |
| `visualization.edge_levels_meta` | one row: the options, the number of edges and a hash of their ids, the roadstyle version, the time |

Both tables are replaced each time. [Every column](../reference/database.md#visualization-schema). A stack of roads that cannot all be satisfied is counted in the message the command prints, never hidden.

## Read it

```python
import duckdb, roadstyle as rs
from duckosm.levels import load_roads

con = duckdb.connect("monaco.duckdb", read_only=True)
levels = rs.load_levels(con, load_roads("monaco.duckdb"), band_col="band", order="class")
```

`load_levels` checks that the options and the edges are those the numbers were computed for, and stops with a message that says what differs. It never recomputes. After a rebuild of the file, run `duckosm levels` again.

Options: [`duckosm levels`](../reference/cli.md#levels).
