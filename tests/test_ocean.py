"""features.ocean, the sea from Overture Maps (docs/design/sea.md)."""
import os

import duckdb
import pytest

from duckosm.features import ocean


def _db(box="POLYGON((7.409 43.725, 7.439 43.725, 7.439 43.752, 7.409 43.752, 7.409 43.725))"):
    con = duckdb.connect()
    con.execute("INSTALL spatial; LOAD spatial")
    con.execute(f"CREATE TABLE main.boundary AS SELECT ST_GeomFromText('{box}') AS geom")
    return con


def test_area_box_grows_by_a_tenth_and_at_least_500_m():
    x0, y0, x1, y1 = ocean.area_box(_db())
    assert x0 == pytest.approx(7.409 - 500 / (111320 * 0.7225), rel=0.01)   # 3 km wide: 500 m wins
    assert y1 == pytest.approx(43.752 + 500 / 111320, rel=1e-6)
    big = _db("POLYGON((17 59, 19 59, 19 60, 17 60, 17 59))")
    assert ocean.area_box(big)[0] == pytest.approx(16.8)                     # 10 % of 2 degrees


def test_offline_leaves_an_empty_table_and_never_fails(monkeypatch):
    def down(_remote):
        raise OSError("network is unreachable")
    monkeypatch.setattr(ocean, "latest_release", down)
    con = _db()
    assert ocean.build_ocean(con) == 0
    assert con.execute("SELECT count(*) FROM features.ocean").fetchone()[0] == 0
    assert [r[0] for r in con.execute("DESCRIBE features.ocean").fetchall()] == \
        ["osm_id", "osm_type", "kind", "name", "source", "geom"]


@pytest.mark.skipif(not os.environ.get("DUCKOSM_NETWORK_TESTS"), reason="reads Overture Maps from S3")
def test_monaco_sea_from_overture():
    con = _db()
    assert ocean.build_ocean(con) >= 1
    sea, land = con.execute("SELECT bool_or(ST_Contains(geom, ST_Point(7.4300, 43.7250))), "
                            "bool_or(ST_Contains(geom, ST_Point(7.4280, 43.7390))) FROM features.ocean").fetchone()
    assert sea and not land                       # off Monaco-Ville / the Casino
