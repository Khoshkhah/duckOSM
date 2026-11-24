"""Tests for `duckosm way` — the cross-mode edge table and the raw way of one OSM id."""
from cli_run import invoke
import json

import duckdb
import pytest


def _db():
    """Two mode schemas with DIFFERENT edge columns + a raw way."""
    con = duckdb.connect()
    con.execute("CREATE SCHEMA driving; CREATE SCHEMA walking; CREATE SCHEMA raw")
    con.execute("""CREATE TABLE driving.edges(edge_id BIGINT, edge_ref VARCHAR, source BIGINT,
        target BIGINT, osm_id BIGINT, is_reverse BOOLEAN, maxspeed_kmh FLOAT)""")
    con.execute("""CREATE TABLE walking.edges(edge_id BIGINT, edge_ref VARCHAR, source BIGINT,
        target BIGINT, osm_id BIGINT, is_reverse BOOLEAN, walk_type VARCHAR)""")
    con.execute("INSERT INTO driving.edges VALUES (10, '5#1f', 1, 2, 5, FALSE, 50.0), "
                "(11, '5#2f', 2, 3, 5, FALSE, 50.0), (12, '5#1r', 2, 1, 5, TRUE, 50.0)")
    con.execute("INSERT INTO walking.edges VALUES (10, '5#1f', 1, 2, 5, FALSE, 'sidewalk')")
    con.execute("CREATE TABLE raw.ways(osm_id BIGINT, refs BIGINT[], tags MAP(VARCHAR, VARCHAR))")
    con.execute("INSERT INTO raw.ways VALUES (5, [1, 2, 3], MAP {'highway': 'residential'})")
    return con


@pytest.fixture
def db(tmp_path):
    db = tmp_path / "t.duckdb"
    src, dst = _db(), duckdb.connect(str(db))
    for schema, table in (("driving", "edges"), ("walking", "edges"), ("raw", "ways")):
        dst.execute(f"CREATE SCHEMA IF NOT EXISTS {schema}")
        rows = src.execute(f"SELECT * FROM {schema}.{table}").fetchall()
        cols = src.execute(f"DESCRIBE {schema}.{table}").fetchall()
        dst.execute(f"CREATE TABLE {schema}.{table} "
                    f"({', '.join(f'{c[0]} {c[1]}' for c in cols)})")
        for r in rows:
            dst.execute(f"INSERT INTO {schema}.{table} VALUES "
                        f"({', '.join('?' for _ in r)})", list(r))
    dst.close()
    return db


def test_way_unions_modes_with_null_fill(db):
    r = invoke(["way", str(db), "5", "--json"])
    assert r.exit_code == 0, r.output
    d = json.loads(r.stdout)
    rows = d["edges"]
    assert len(rows) == 4
    assert d["raw"]["refs"] == [1, 2, 3] and d["raw"]["tags"]["highway"] == "residential"
    by_mode = {(x["mode"], x["edge_ref"]): x for x in rows}
    # walking rows NULL-fill the driving-only column and vice versa
    assert by_mode[("walking", "5#1f")]["maxspeed_kmh"] is None
    assert by_mode[("driving", "5#1f")]["walk_type"] is None
    # traversal order: seq 1 (fwd, both modes) before seq 1 reverse, before seq 2
    assert [(x["edge_ref"], x["mode"]) for x in rows] == [("5#1f", "driving"), ("5#1f", "walking"), ("5#1r", "driving"),
                                                          ("5#2f", "driving")]


def test_way_mode_filter_and_missing(db):
    d = json.loads(invoke(["way", str(db), "5", "-m", "walking", "--json"]).stdout)
    assert len(d["edges"]) == 1
    d = json.loads(invoke(["way", str(db), "999", "--json"]).stdout)
    assert d["edges"] == []                                  # unknown id -> empty, not an error
    assert invoke(["way", str(db), "5", "-m", "cycling"]).exit_code != 0   # no such schema in this db


def test_way_without_raw_schema(db):
    con = duckdb.connect(str(db))
    con.execute("DROP TABLE raw.ways")
    con.close()
    r = invoke(["way", str(db), "5", "--json"])
    assert r.exit_code == 0, r.output
    assert not json.loads(r.stdout).get("raw")               # clip build: no raw schema


def test_cli_way_writes_csv(db, tmp_path):
    out = tmp_path / "way.csv"
    res = invoke(["way", str(db), "5", "-o", str(out)])
    assert res.exit_code == 0, res.output
    lines = out.read_text().strip().splitlines()
    assert lines[0].startswith("mode,edge_id,edge_ref")     # header = column union
    assert len(lines) == 5                                  # 4 edge rows + header
    assert "walking" in lines[2] and "driving" in lines[1]  # both modes present
