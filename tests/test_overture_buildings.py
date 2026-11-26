"""duckosm overture-buildings: the attributes of Overture Maps in features.buildings (needs the internet)."""
import duckdb
import pytest

from duckosm import overture_buildings


def test_overture_adds_tags_and_osm_wins(tmp_path):
    db = tmp_path / "b.duckdb"
    con = duckdb.connect(str(db))
    con.execute("INSTALL spatial; LOAD spatial; CREATE SCHEMA features")
    box = "ST_GeomFromText('POLYGON((7.41 43.73, 7.42 43.73, 7.42 43.74, 7.41 43.74, 7.41 43.73))')"
    con.execute(f"""CREATE TABLE features.buildings AS SELECT * FROM (VALUES
        (143225899, 'way', 'yes', NULL::VARCHAR, MAP {{'building': 'yes', 'height': '999'}}, {box}),
        (143229720, 'way', 'yes', NULL, MAP {{'building': 'yes'}}, {box}),
        (1, 'way', 'yes', NULL, MAP {{'building': 'yes'}}, {box})
    ) t(osm_id, osm_type, kind, name, tags, geom)""")
    con.close()
    try:
        s = overture_buildings(db)
    except RuntimeError as e:
        if not any(w in str(e) for w in ("not reachable", "IO Error", "HTTP")):
            raise
        pytest.skip(f"Overture Maps not reachable: {e}")
    assert s["buildings"] == 3 and s["matched"] == 2
    tags = dict(duckdb.connect(str(db)).execute("SELECT osm_id, tags FROM features.buildings").fetchall())
    assert tags[143225899]["height"] == "999"                      # OSM wins
    assert tags[143225899]["overture:id"] and tags[143229720]["overture:id"]
    assert "overture:id" not in tags[1]                            # not in Overture: unchanged
    assert overture_buildings(db, release=s["release"])["updated"] == 0   # a second run adds nothing
