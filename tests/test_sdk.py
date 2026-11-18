"""The Python SDK (docs/reference/python.md): each command function gives the same files as its command on Monaco."""
import json
import shutil
import subprocess
import sys
from pathlib import Path

import duckdb
import pytest

import duckosm

DATA = Path(__file__).resolve().parent / "data"
PBF, BOUNDARY = DATA / "monaco.osm.pbf", DATA / "monaco.geojson"


def _cmd(*args):
    subprocess.run([sys.executable, "-m", "duckosm", *map(str, args)], check=True, capture_output=True)


def _select(con, rel):
    """The sorted rows of ``rel``, without the time of the build (the columns of a time type and ``created``)."""
    cols = [f'"{c}"' for c, t, *_ in con.execute(f"DESCRIBE SELECT * FROM {rel}").fetchall()
            if not t.startswith("TIMESTAMP") and c != "created"]
    return sorted(map(repr, con.execute(f"SELECT {', '.join(cols)} FROM {rel}").fetchall()))


def _rows(path):
    """The rows of a GeoParquet file, or of each table of a .duckdb file."""
    con = duckdb.connect()
    con.execute("LOAD spatial")
    if path.suffix == ".parquet":
        return _select(con, f"read_parquet('{path}')")
    con.execute(f"ATTACH '{path}' AS d (READ_ONLY)")
    tables = con.execute("SELECT table_schema, table_name FROM information_schema.tables WHERE table_catalog = 'd' ORDER BY 1, 2").fetchall()
    return {f"{s}.{t}": _select(con, f'd."{s}"."{t}"') for s, t in tables}


def _same(a, b):
    """Folders ``a`` and ``b`` have the same files, and the .parquet and .duckdb files have the same rows."""
    fa = sorted(p.relative_to(a) for p in a.rglob("*") if p.is_file())
    assert fa == sorted(p.relative_to(b) for p in b.rglob("*") if p.is_file())
    for f in fa:
        if f.suffix in (".parquet", ".duckdb"):
            assert _rows(a / f) == _rows(b / f), f


@pytest.mark.parametrize("kw, args", [
    ({"formats": ["geoparquet", "duckdb"]}, ["-f", "geoparquet,duckdb"]),
    ({"gdal": True, "formats": "geoparquet"}, ["--gdal", "-f", "geoparquet"]),
    ({"formats": ["geoparquet"], "tile_zoom": 14}, ["-f", "geoparquet", "--tile-zoom", "14"]),
    ({"formats": "geoparquet", "fixes": DATA / "osm_fixes_quack.yaml"}, ["-f", "geoparquet", "--fixes", DATA / "osm_fixes_quack.yaml"]),
])
def test_quack(tmp_path, kw, args):
    files = duckosm.quack(PBF, out=tmp_path / "py", name="monaco", **kw)
    _cmd("quack", PBF, "-o", tmp_path / "cmd", "--name", "monaco", *args)
    assert files and all(f.parent == tmp_path / "py" for f in files)
    _same(tmp_path / "py", tmp_path / "cmd")


def test_init_config(tmp_path):
    assert duckosm.init_config(tmp_path / "a.yaml") == tmp_path / "a.yaml"
    _cmd("init-config", tmp_path / "b.yaml")
    assert (tmp_path / "a.yaml").read_text() == (tmp_path / "b.yaml").read_text()
    with pytest.raises(RuntimeError, match="already exists"):
        duckosm.init_config(tmp_path / "a.yaml")
    duckosm.init_config(tmp_path / "a.yaml", force=True)


@pytest.mark.skipif(shutil.which("ogr2ogr") is None, reason="the borders in the PBF need GDAL's ogr2ogr")
def test_boundary(tmp_path):
    got = duckosm.boundary(osm_id=2220322, pbf=PBF, offline=True, out=tmp_path / "a.geojson")
    _cmd("boundary", "--osm-id", 2220322, "--pbf", PBF, "--offline", "-o", tmp_path / "b.geojson")
    assert json.loads(got.read_text()) == json.loads((tmp_path / "b.geojson").read_text())


@pytest.fixture(scope="module")
def monaco_db(tmp_path_factory):
    d = tmp_path_factory.mktemp("built")
    _cmd(PBF, "-o", d, "--name", "monaco", "--boundary", BOUNDARY, "-m", "driving,walking", "--levels", "none")
    return d / "monaco.duckdb"


def test_extract(tmp_path, monaco_db):
    (tmp_path / "py").mkdir()
    (tmp_path / "cmd").mkdir()
    got = duckosm.extract(monaco_db, tmp_path / "py" / "a.duckdb", boundary=BOUNDARY)
    _cmd("extract", "--source", monaco_db, "--db", tmp_path / "cmd" / "a.duckdb", "--boundary", BOUNDARY)
    assert got == tmp_path / "py" / "a.duckdb"
    assert _rows(got) == _rows(tmp_path / "cmd" / "a.duckdb")


def test_tiles(tmp_path):
    got = duckosm.tiles(PBF, boundary=BOUNDARY, out=tmp_path / "py", build_args=["--modes", "driving"])
    _cmd("tiles", PBF, "--boundary", BOUNDARY, "-o", tmp_path / "cmd", "--", "--modes", "driving")
    assert got == tmp_path / "py"
    _same(tmp_path / "py", tmp_path / "cmd")


def test_error_has_the_message(tmp_path):
    with pytest.raises(RuntimeError, match="extract"):
        duckosm.extract(tmp_path / "none.duckdb", tmp_path / "a.duckdb", boundary=BOUNDARY)


def test_build(tmp_path, monkeypatch):
    monkeypatch.chdir(tmp_path)
    got = duckosm.build(pbf=PBF, boundary=BOUNDARY, modes=["driving", "walking"], output="py/monaco.duckdb", levels=False)
    _cmd("build", "-p", PBF, "-b", BOUNDARY, "-m", "driving", "-m", "walking", "-o", "cmd/monaco.duckdb", "--no-levels")
    assert got == Path("py/monaco.duckdb")
    a, b = _rows(got), _rows(Path("cmd/monaco.duckdb"))
    assert {t: r for t, r in a.items() if t != "main.visualization_metadata"} == \
           {t: r for t, r in b.items() if t != "main.visualization_metadata"}
    assert "visualization.edge_levels" not in a


def test_levels_keeps_the_edits(tmp_path, monkeypatch):
    """duckosm levels solves with visualization.level_edits; a row that names no road waits in level_unmatched; a new build of the
    file keeps both."""
    monkeypatch.chdir(tmp_path)
    db = duckosm.build(pbf=PBF, boundary=BOUNDARY, output="monaco.duckdb")
    con = duckdb.connect(str(db))
    a, b = con.execute("SELECT edge_id FROM driving.edges ORDER BY edge_id LIMIT 2").fetchall()
    con.execute("CREATE OR REPLACE TABLE visualization.level_edits AS SELECT * FROM (VALUES "
                f"('order', {a[0]}::BIGINT, {b[0]}::BIGINT, NULL, NULL, 'true'), ('order', 1::BIGINT, 2::BIGINT, NULL, NULL, 'true')) "
                "t(relation, a, b, a_end, b_end, enabled)")
    con.close()
    assert duckosm.levels(db) == db

    def state():
        con = duckdb.connect(str(db), read_only=True)
        try:
            return (con.execute("SELECT a, b FROM visualization.level_edits").fetchall(),
                    con.execute("SELECT kind FROM visualization.level_unmatched").fetchall(),
                    con.execute("SELECT count(*), count(DISTINCT edge_id) FROM visualization.edge_levels").fetchone(),
                    con.execute("SELECT source FROM visualization.edge_levels_meta").fetchone())
        finally:
            con.close()
    edits, unmatched, (n, distinct), meta = state()
    assert edits == [(a[0], b[0])] and unmatched == [("level_edits",)]
    assert n == distinct > 10_000 and meta == ("area",)
    duckosm.build(pbf=PBF, boundary=BOUNDARY, output="monaco.duckdb")
    assert state()[:2] == (edits, unmatched)
