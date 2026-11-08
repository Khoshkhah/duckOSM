"""options.engine: rust (the default) builds the network step with the Rust engine (the module duckosm._rust). The
output file must have the same tables as the Python build (options.engine: python, the reference): the network
tables that Rust writes, and the tables that the later Python steps make from them. Every row is equal, also the
connector rows (the order of the dangling ends is fixed in both engines). See docs/design/rust_engine.md."""
from pathlib import Path

import duckdb

from duckosm import Config, DuckOSM
from duckosm import _rust  # noqa: F401  # no skip: every wheel has the module, and a missing one is a failure

ROOT = Path(__file__).resolve().parents[1]
MODES = ("driving", "walking", "cycling")


def _build(tmp_path, engine):
    cfg = Config.from_yaml(str(ROOT / "config" / "sample_monaco.yaml"))
    cfg.output_path = str(tmp_path / engine)
    cfg.options.engine = engine
    cfg.options.sea = None                      # the sea comes from the network (S3): not in a test
    return DuckOSM(cfg).run()


def _rows(con, db, table):
    """All rows of a table, with the geometry as WKB, as a sorted list."""
    desc = con.execute(f"DESCRIBE {db}.{table}").fetchall()
    cols = [(r[0], r[1]) for r in desc]            # the names and the types
    sel = ", ".join(f"ST_AsWKB({r[0]})" if r[0] in ("geometry", "geom") else r[0] for r in desc)
    return cols, sorted(map(repr, con.execute(f"SELECT {sel} FROM {db}.{table}").fetchall()))


def test_rust_is_the_default_engine():
    assert Config().options.engine == "rust"


def test_rust_engine_equals_python(tmp_path, monkeypatch):
    monkeypatch.chdir(ROOT)                     # the sample config has relative paths
    py, rs = _build(tmp_path, "python"), _build(tmp_path, "rust")
    con = duckdb.connect()
    con.execute("LOAD spatial")
    con.execute(f"ATTACH '{py}' AS py (READ_ONLY)")
    con.execute(f"ATTACH '{rs}' AS rs (READ_ONLY)")
    tables = [f"{m}.{t}" for m in MODES
              for t in ("edges", "private_edges", "nodes", "virtual_nodes", "edge_id_map", "edge_graph", "turn_permission")]
    tables += ["driving.turn_restrictions", "driving.turn_path_restrictions"]
    for t in tables:
        pc, pr = _rows(con, "py", t)
        rc, rr = _rows(con, "rs", t)
        assert pc == rc, f"{t}: the columns differ"
        assert pr == rr, f"{t}: {len(set(pr) ^ set(rr))} rows differ"
    # the sample config turns the walls of the buildings on: only the Rust engine makes them and the doors that go with them (the Python engine writes a warning)
    names = "SELECT table_schema, table_name FROM information_schema.tables WHERE table_catalog = '%s' AND table_name NOT IN ('building_faces', 'building_entrances') ORDER BY 1, 2"
    assert con.execute(names % "py").fetchall() == con.execute(names % "rs").fetchall(), "the two files have other tables"
    assert con.execute("SELECT count(*) FROM rs.main.building_faces").fetchone()[0] > 10_000
    assert con.execute("SELECT count(*) FROM rs.driving.edges").fetchone()[0] > 2000
