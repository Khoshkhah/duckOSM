"""Run duckOSM with the same config as `duckosm build --config`, and time each step inside the process.

    python bench/python_steps.py config.yaml [--no-lane-profile]

It prints one line per step: `STEP <name> <seconds>`. The steps are wrapped methods of DuckOSM, so the
code that runs is the code of `duckosm build`. With `--no-lane-profile` the lane profile step does nothing.
duckOSM cannot switch that step off, and it is not part of the network step (it is a Python loop over
every edge and takes more time than all the other steps together).
"""
import sys
import time

from duckosm.config import Config
from duckosm.importer import DuckOSM
from duckosm.processors import SeparateSidewalks

STEPS = ["_connect", "_load_pbf", "_raw_way_fixes", "_build_global_junctions",
         "_rust_build", "_filter_roads", "_apply_osm_overrides", "_build_edges", "_simplify_graph", "_rust_network",
         "_connect_paths", "_mark_dismount", "_process_speeds", "_calculate_costs", "_add_functional_type",
         "_split_private", "_extract_restrictions", "_build_edge_graph", "_component_filter", "_add_h3_indexing",
         "_create_indexes", "_lane_profile", "_build_features", "_levels_step", "_cleanup", "_generate_metadata",
         "_checkpoint"]


def wrap(name, cls=DuckOSM, attr=None):
    orig = getattr(cls, attr or name)

    def timed(self, *a, **k):
        t = time.perf_counter()
        try:
            return orig(self, *a, **k)
        finally:
            print(f"STEP {name} {time.perf_counter() - t:.4f}", flush=True)

    setattr(cls, attr or name, timed)


if "--no-lane-profile" in sys.argv:
    DuckOSM._lane_profile = lambda self: None
for s in STEPS:
    if hasattr(DuckOSM, s):
        wrap(s)
wrap("separate_sidewalks", SeparateSidewalks, "run")   # the sidewalk rule is a lambda in run(), not a method
t0 = time.perf_counter()
DuckOSM(Config.from_yaml(sys.argv[1])).run()
print(f"STEP total_run {time.perf_counter() - t0:.4f}")
