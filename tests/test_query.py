"""Tests for duckosm.query — the cross-mode `way_table` / `way_raw` report."""
import duckdb
import pytest

from duckosm.query import way_raw, way_table, way_table_sql


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


def test_way_table_unions_modes_with_null_fill():
    rows = way_table(_db(), 5).fetchall()
    assert len(rows) == 4
    cols = [d[0] for d in way_table(_db(), 5).description]
    assert cols[0] == "mode"
    assert "maxspeed_kmh" in cols and "walk_type" in cols   # column union across modes
    by_mode = {(r[cols.index("mode")], r[cols.index("edge_ref")]): r for r in rows}
    # walking rows NULL-fill the driving-only column and vice versa
    assert by_mode[("walking", "5#1f")][cols.index("maxspeed_kmh")] is None
    assert by_mode[("driving", "5#1f")][cols.index("walk_type")] is None
    # traversal order: seq 1 (fwd, both modes) before seq 1 reverse, before seq 2
    order = [(r[cols.index("edge_ref")], r[cols.index("mode")]) for r in rows]
    assert order == [("5#1f", "driving"), ("5#1f", "walking"), ("5#1r", "driving"),
                     ("5#2f", "driving")]


def test_way_table_mode_filter_and_missing():
    con = _db()
    assert len(way_table(con, 5, modes=["walking"]).fetchall()) == 1
    assert way_table(con, 999).fetchall() == []             # unknown id -> empty, not an error
    with pytest.raises(ValueError):
        way_table_sql(con, 5, modes=["cycling"])            # no such schema in this db


def test_way_raw():
    con = _db()
    raw = way_raw(con, 5)
    assert raw["refs"] == [1, 2, 3] and raw["tags"]["highway"] == "residential"
    assert way_raw(con, 999) is None
    con.execute("DROP TABLE raw.ways")
    assert way_raw(con, 5) is None                          # clip build: no raw schema


def test_cli_way_writes_csv(tmp_path):
    from click.testing import CliRunner

    from duckosm.cli import main

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

    out = tmp_path / "way.csv"
    res = CliRunner().invoke(main, ["way", str(db), "5", "-o", str(out)])
    assert res.exit_code == 0, res.output
    lines = out.read_text().strip().splitlines()
    assert lines[0].startswith("mode,edge_id,edge_ref")     # header = column union
    assert len(lines) == 5                                  # 4 edge rows + header
    assert "walking" in lines[2] and "driving" in lines[1]  # both modes present
