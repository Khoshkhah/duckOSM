"""The test of docs/levels.md, "The finding": roadstyle's solver stages as a MILP and as an LP (no integer variables).

Run it with duckOSM's virtual environment (it has duckosm and roadstyle):

    /home/kaveh/projects/duckOSM/.venv/bin/python tools/levels_lp_check.py /home/kaveh/projects/duckOSM/monaco.duckdb [EDITS.csv]

For each way, it prints the value of each solve and the number of fractional variables. "LP, exact" removes
roadstyle's tolerance (1e-7) from the rows that keep the value of the stage before.
"""
import sys
import warnings

import numpy as np
import scipy.optimize as so

warnings.simplefilter("ignore")
from duckosm.levels import load_roads  # noqa: E402
import roadstyle as rs  # noqa: E402

roads, pairs = rs.level_input(load_roads(sys.argv[1]), id_col="edge_id")
edits = sys.argv[2] if len(sys.argv) > 2 else None    # a level area's edits.csv, for example duckOSM's monaco.levels/edits.csv
milp = so.milp


def run(name, integer, exact):
    log = []

    def wrapped(c, constraints, integrality=None, **kw):
        ub = np.asarray(constraints.ub, float)
        if exact:
            ub = np.where(np.abs(ub - np.round(ub)) < 1e-4, np.round(ub), ub)
        r = milp(c, constraints=so.LinearConstraint(constraints.A, constraints.lb, ub),
                 integrality=integrality if integer else np.zeros(len(c)), **kw)
        frac = 0 if r.x is None else int((np.abs(r.x - np.round(r.x)) > 1e-6).sum())
        log.append((None if r.fun is None else round(r.fun, 4), frac))
        return r

    so.milp = wrapped                    # roadstyle imports milp inside the function, so this replaces it
    s = rs.solve_levels(roads, pairs, edits=edits)
    print(f"{name}: (value, fractional variables) of each solve: {log}; "
          f"order wishes broken: {len(s.attrs['levels_orders_not_kept'])}")


run("MILP", integer=True, exact=False)
run("LP, with the tolerance", integer=False, exact=False)
run("LP, exact", integer=False, exact=True)
