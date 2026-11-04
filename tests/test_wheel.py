"""Check the installed wheel (the module duckosm._rust) on the sample file of Monaco. Run: python tests/test_wheel.py [path to monaco_sample.osm.pbf]

The counts are those of duckOSM 0.6.0 (see tests/data/monaco_modes_summary.tsv, variant nomerge_gj).
"""
import os
import sys
import tempfile

import duckosm
from duckosm import _rust as dr

pbf = sys.argv[1] if __name__ == "__main__" and len(sys.argv) > 1 else os.path.join(os.path.dirname(__file__), "data", "monaco_sample.osm.pbf")
want = {"driving": (3092, 724, 1904), "walking": (12238, 166, 4818), "cycling": (10670, 141, 4867)}

assert dr.edge_id_hash(4230114, 1868723946, 1080509801) == 414242052250797   # an edge of the golden file
with tempfile.TemporaryDirectory() as out:
    res = dr.build(pbf, out, modes=list(want), formats=["geoparquet"], merge_segments=False, keep_largest_component=False,
                   process_speeds=False, calculate_costs=False, connectivity_rescue=False, infer_separate_sidewalks=False)
    for mode, (edges, private, nodes) in want.items():
        got = res["modes"][mode]
        assert (got["edges"], got["private_edges"], got["nodes"]) == (edges, private, nodes), (mode, got)
        assert os.path.exists(res["files"][f"edges_{mode}"])
# the .duckdb output (only in a wheel that was built with the cargo feature duckdb)
with tempfile.TemporaryDirectory() as out:
    try:
        res = dr.build(pbf, out, modes=["driving"], formats=["duckdb"], name="monaco", merge_segments=False, keep_largest_component=False,
                       process_speeds=False, calculate_costs=False, connectivity_rescue=False)
    except RuntimeError as e:
        assert "no DuckDB output" in str(e), e
        print("this wheel has no DuckDB output")
    else:
        import duckdb
        con = duckdb.connect(res["files"]["duckdb"], read_only=True)
        assert con.execute("SELECT count(*) FROM driving.edges").fetchone()[0] == 3092
        assert con.execute("SELECT count(*) FROM driving.edges WHERE edge_id <> edge_id_hash(osm_id, source, target)").fetchone()[0] == 0
try:
    dr.build(pbf, "x", modes=["flying"])
    raise SystemExit("a wrong mode must raise ValueError")
except ValueError:
    pass
assert dr.__version__ == duckosm.__version__, (dr.__version__, duckosm.__version__)   # rust/Cargo.toml and pyproject.toml
print("wheel OK, duckosm._rust", dr.__version__)
