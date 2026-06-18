"""
Tests for the Workstream-A pipeline: config schema, ComponentFilter, the duckdb clip,
and the Validator. Run: pytest tests/ -q
"""
from pathlib import Path

import duckdb
import pytest

from duckosm import DuckOSM, Config
from duckosm.processors import ComponentFilter
from duckosm.validate import Validator, ValidationError

PARENT = Path(__file__).resolve().parents[1] / "data" / "db" / "sodermalm.duckdb"
BOUND = Path(__file__).resolve().parents[1] / "data" / "boundaries" / "sodermalm.geojson"


# ---- config schema -----------------------------------------------------------------
def test_config_duckdb_source(tmp_path):
    y = tmp_path / "c.yaml"
    y.write_text(
        "name: a\noutput_path: /tmp/a.duckdb\n"
        "source: {type: duckdb, source_db: /x/p.duckdb}\n"
        "boundary: {path: /x/b.geojson}\n"
        "clip: {predicate: within, keep_largest_component: false}\n")
    c = Config.from_yaml(str(y))
    assert c.source_type == "duckdb"
    assert c.source_db == "/x/p.duckdb"
    assert c.effective_boundary_path == "/x/b.geojson"
    assert c.clip.predicate == "within"
    assert c.clip.keep_largest_component is False


def test_config_backcompat_flat(tmp_path):
    y = tmp_path / "c.yaml"
    y.write_text(
        "name: b\npbf_path: /x/in.osm.pbf\noutput_path: /tmp/b\n"
        "boundary_path: /x/bnd.geojson\noptions: {simplify: true}\n")
    c = Config.from_yaml(str(y))
    assert c.source_type == "pbf"
    assert c.effective_pbf_path == "/x/in.osm.pbf"
    assert c.effective_boundary_path == "/x/bnd.geojson"
    assert c.options.simplify is True


# ---- ComponentFilter unit ----------------------------------------------------------
def _toy_graph():
    con = duckdb.connect()
    con.execute("CREATE TABLE edges(edge_id BIGINT, source BIGINT, target BIGINT)")
    con.execute("CREATE TABLE nodes(node_id BIGINT)")
    con.execute("CREATE TABLE edge_graph(from_edge BIGINT, to_edge BIGINT)")
    con.executemany("INSERT INTO edges VALUES (?,?,?)",
                    [(1, 1, 2), (2, 2, 3), (3, 3, 1), (99, 10, 11)])  # triangle + 1 fragment
    con.executemany("INSERT INTO nodes VALUES (?)", [(1,), (2,), (3,), (10,), (11,)])
    con.executemany("INSERT INTO edge_graph VALUES (?,?)", [(1, 2), (2, 3), (3, 1)])
    return con


def test_component_filter_keeps_largest():
    con = _toy_graph()
    ComponentFilter(con, keep_largest=True).run()
    assert {r[0] for r in con.execute("SELECT edge_id FROM edges").fetchall()} == {1, 2, 3}
    assert {r[0] for r in con.execute("SELECT node_id FROM nodes").fetchall()} == {1, 2, 3}


def test_component_filter_noop_when_connected():
    con = duckdb.connect()
    con.execute("CREATE TABLE edges(edge_id BIGINT, source BIGINT, target BIGINT)")
    con.execute("CREATE TABLE nodes(node_id BIGINT)")
    con.executemany("INSERT INTO edges VALUES (?,?,?)", [(1, 1, 2), (2, 2, 3)])
    con.executemany("INSERT INTO nodes VALUES (?)", [(1,), (2,), (3,)])
    ComponentFilter(con, keep_largest=True).run()
    assert con.execute("SELECT count(*) FROM edges").fetchone()[0] == 2


# ---- Validator ---------------------------------------------------------------------
def test_validator_flags_fragment():
    con = _toy_graph()
    con.execute("CREATE SCHEMA driving")
    con.execute('CREATE TABLE driving.edges AS SELECT edge_id, source, target, '
                'CAST(NULL AS VARCHAR) AS "name" FROM edges')

    class V:  # minimal validation config
        assert_single_component = True
        assert_no_stranded_named = False
        assert_edge_id_stable = False
        fail_on_error = True
    with pytest.raises(ValidationError):
        Validator(con, "driving", V()).run()


# ---- duckdb clip integration (skips if the parent isn't built) ---------------------
@pytest.mark.skipif(not (PARENT.exists() and BOUND.exists()), reason="parent db not built")
def test_clip_preserves_edge_ids(tmp_path):
    out = tmp_path / "clip.duckdb"
    cfg = Config.from_args(pbf_path="", output_path=str(out), name="t",
                           boundary_path=str(BOUND), source_db=str(PARENT),
                           build_graph=True, h3_indexing=False)
    DuckOSM(cfg).run()
    con = duckdb.connect()
    con.execute(f"ATTACH '{PARENT}' AS p (READ_ONLY)")
    con.execute(f"ATTACH '{out}' AS c (READ_ONLY)")
    n_clip = con.execute("SELECT count(*) FROM c.driving.edges").fetchone()[0]
    preserved = con.execute(
        "SELECT count(*) FROM c.driving.edges x JOIN p.driving.edges y "
        "ON x.edge_id=y.edge_id AND x.source=y.source AND x.target=y.target").fetchone()[0]
    assert n_clip > 1000 and preserved == n_clip       # every clipped edge_id verbatim in parent
    # fewer edges than the parent (the disconnected fragments were dropped)
    n_parent = con.execute("SELECT count(*) FROM p.driving.edges").fetchone()[0]
    assert n_clip < n_parent
