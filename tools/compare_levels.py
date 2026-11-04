"""Compare visualization.edge_levels of a duckosm-rust file with roadstyle's result on the same edges (docs/levels.md).

    /home/kaveh/projects/duckOSM/.venv/bin/python tools/compare_levels.py ours.duckdb

roadstyle (duckOSM's virtual environment) solves the edges of the file with duckOSM's defaults and no edits. Then
both results are checked against roadstyle's rules (its pairs): the hard rules (each casing part not over its own
fill, the heads under the fill of the roads they meet), the stack parts that are not lifted, and the order wishes
that are not kept. The numbers can differ where many answers have the same best value. With `source` = `tags`, the
rows are compared with roadstyle's `compute_levels(method="tags")`. Exits with 1 when a hard rule is broken, or a
count is not equal.
"""
import sys
import time
import warnings

import duckdb

warnings.simplefilter("ignore")
from duckosm.levels import load_roads  # noqa: E402
import roadstyle as rs  # noqa: E402
from roadstyle import level_area  # noqa: E402

DB = sys.argv[1]
con = duckdb.connect(DB, read_only=True)
source = con.execute("SELECT source FROM visualization.edge_levels_meta").fetchone()[0]
ours = con.execute("SELECT * FROM visualization.edge_levels ORDER BY edge_id").df()
con.close()
edges = load_roads(DB)
COLS = ["casing_start", "casing_level", "casing_end", "fill_level"]

if source == "tags":
    ref = rs.compute_levels(edges, method="tags")[["edge_id", *COLS]].sort_values("edge_id")
    same = (ref[COLS].to_numpy() == ours[COLS].to_numpy()).all(axis=1)
    print(f"tags: {len(ours)} rows, {int((~same).sum())} not equal to roadstyle's")
    sys.exit(int(len(ours) != len(ref) or not same.all()))

t = time.time()
roads, pairs = rs.level_input(edges, id_col="edge_id")
solved = rs.solve_levels(roads, pairs)
print(f"roadstyle: {len(roads)} roads, {pairs.relation.value_counts().to_dict()}, solve {time.time() - t:.1f} s")
ref = level_area.edge_levels(solved, level_area.defaults(roads, pairs)).rename(columns={"edge": "edge_id"})
ref["edge_id"] = ref["edge_id"].astype("int64")
ref = ref.sort_values("edge_id").reset_index(drop=True)
if list(ref.edge_id) != list(ours.edge_id):
    sys.exit(f"the edges differ: {len(ref)} in roadstyle's result, {len(ours)} in {DB}")
ALL = [*COLS, "head_start_m", "head_end_m", "cap_start", "cap_end"]
same = (ref[ALL].to_numpy() == ours[ALL].to_numpy()).all(axis=1)
print(f"rows: {len(ours)}, {int(same.sum())} equal to roadstyle's in all 8 columns")


def check(name, table):
    """The rules of roadstyle's pairs on one result: (hard rules broken, stack parts not lifted, wishes not kept)."""
    num = {str(e): v for e, *v in table[["edge_id", *COLS]].itertuples(index=False)}
    road = {r: num[r] for r in roads["road"]}            # a road is named by its first edge: its numbers in its own way
    hard = sum(not (s <= b and m <= b and e <= b) for s, m, e, b in road.values())
    part = {"start": 0, "main": 1, "end": 2}
    lifted = kept = 0
    for rel, a, b, ae, be in pairs.itertuples(index=False, name=None):
        if rel == "meet":
            hard += road[a][part[ae]] > road[b][3] or road[b][part[be]] > road[a][3]
        elif rel == "stack":
            lifted += road[a][part[ae]] <= road[b][3]
        elif rel == "order":
            kept += road[a][3] <= road[b][3]
    print(f"{name}: {hard} hard rules broken, {lifted} stack parts not lifted, {kept} order wishes not kept, "
          f"{len(set(v for r in road.values() for v in r))} numbers")
    return hard, lifted, kept


a, b = check("roadstyle", ref), check("duckosm-rust", ours)
sys.exit(int(b[0] > 0 or a != b))
