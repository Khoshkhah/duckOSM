"""The load step of the Rust engine (docs/design/streaming_load.md) gives the same raw.* tables as ST_READOSM: the same
column types and the same rows, on Monaco and in a small memory budget."""
from pathlib import Path
from types import SimpleNamespace

import duckdb
import pytest

from duckosm import rust_engine
from duckosm.importer import DuckOSM

PBF = Path(__file__).parent / "data" / "monaco.osm.pbf"
TABLES = ("nodes", "ways", "relations")


@pytest.fixture(scope="module")
def con(tmp_path_factory):
    pytest.importorskip("duckosm._rust")
    con = duckdb.connect()
    con.execute("INSTALL spatial; LOAD spatial; CREATE SCHEMA raw; CREATE SCHEMA st")
    DuckOSM._load_pbf_st_readosm(SimpleNamespace(con=con, pbf_path=PBF))
    for t in TABLES:
        con.execute(f"CREATE TABLE st.{t} AS SELECT * FROM raw.{t}")
    res = rust_engine.load_raw(con, PBF, tmp_path_factory.mktemp("load"), "monaco", "16MB")
    assert res["budget"] == 16_000_000
    assert (res["nodes"], res["ways"], res["relations"]) == (34291, 5650, 336)
    return con


@pytest.mark.parametrize("table", TABLES)
def test_same_types(con, table):
    cols = lambda s: con.execute(f"SELECT column_name, data_type FROM information_schema.columns "
                                 f"WHERE table_schema = '{s}' AND table_name = '{table}' ORDER BY ordinal_position").fetchall()
    assert cols("raw") == cols("st")


@pytest.mark.parametrize("table", TABLES)
def test_same_rows(con, table):
    n = con.execute(f"SELECT count(*) FROM st.{table}").fetchone()[0]
    assert n > 0
    assert con.execute(f"SELECT count(*) FROM raw.{table}").fetchone()[0] == n
    for a, b in (("raw", "st"), ("st", "raw")):
        diff = con.execute(f"SELECT count(*) FROM (SELECT * FROM {a}.{table} EXCEPT ALL SELECT * FROM {b}.{table})").fetchone()[0]
        assert diff == 0, f"{diff} rows of {a}.{table} are not in {b}.{table}"
