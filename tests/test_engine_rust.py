"""options.engine: rust builds the network step with duckosm-rust (the module duckosm_rust). The
output file must have the same tables as the Python build: the network tables that Rust writes, and
the tables that the later Python steps make from them. See docs/design/rust_engine.md."""
from pathlib import Path

import duckdb
import pytest

from duckosm import Config, DuckOSM

pytest.importorskip("duckosm_rust")

ROOT = Path(__file__).resolve().parents[1]
MODES = ("driving", "walking", "cycling")


def _build(tmp_path, engine):
    cfg = Config.from_yaml(str(ROOT / "config" / "sample_monaco.yaml"))
    cfg.output_path = str(tmp_path / engine)
    cfg.options.engine = engine
    cfg.options.sea = None                      # the sea comes from the network (S3): not in a test
    return DuckOSM(cfg).run()


# The known difference (duckosm-rust design, section 6, item 3c): a connector between two facing
# dangling ends (osm_id < 0) can get the other direction label in the two engines. duckOSM does not fix
# the order. The columns that follow from the direction are left out for connector rows only.
LABELS = ("edge_ref", "highway", "is_reverse", "walk_type", "cycle_type", "dismount", "maxspeed_kmh", "cost_s")


def _rows(con, db, table):
    """All rows of a table, with the geometry as WKB, as a sorted list."""
    cols = [r[0] for r in con.execute(f"DESCRIBE {db}.{table}").fetchall()]

    def expr(c):
        if c in ("geometry", "geom"):
            return f"ST_AsWKB({c})"
        if c in LABELS and "osm_id" in cols:
            return f"CASE WHEN osm_id < 0 THEN NULL ELSE {c} END"
        return c
    sel = ", ".join(expr(c) for c in cols)
    return cols, sorted(map(repr, con.execute(f"SELECT {sel} FROM {db}.{table}").fetchall()))


def test_rust_engine_equals_python(tmp_path, monkeypatch):
    monkeypatch.chdir(ROOT)                     # the sample config has relative paths
    py, rs = _build(tmp_path, "python"), _build(tmp_path, "rust")
    con = duckdb.connect()
    con.execute("LOAD spatial")
    con.execute(f"ATTACH '{py}' AS py (READ_ONLY)")
    con.execute(f"ATTACH '{rs}' AS rs (READ_ONLY)")
    tables = [f"{m}.{t}" for m in MODES for t in ("edges", "private_edges", "nodes", "edge_id_map", "edge_graph")]
    tables.append("driving.turn_restrictions")
    for t in tables:
        pc, pr = _rows(con, "py", t)
        rc, rr = _rows(con, "rs", t)
        assert pc == rc, f"{t}: the columns differ"
        assert pr == rr, f"{t}: {len(set(pr) ^ set(rr))} rows differ"
    assert con.execute("SELECT count(*) FROM rs.driving.edges").fetchone()[0] > 2000
