"""The features layer ``paved``, the new rules of ``water_polygons`` / ``land`` / ``sites``, and ``duckosm features-check``
(docs/design/missing_polygons.md), on a small made-up raw OSM dataset."""
import duckdb
import pytest
from click.testing import CliRunner

from duckosm.cli import main
from duckosm.features import FeaturesBuilder
from duckosm.features.check import missing_polygons


@pytest.fixture
def con(tmp_path):
    c = duckdb.connect(str(tmp_path / "osm.duckdb"))
    try:
        c.execute("INSTALL spatial; LOAD spatial")
    except duckdb.Error as e:
        pytest.skip(f"no spatial extension: {e}")
    c.execute("CREATE SCHEMA raw")
    c.execute("CREATE TABLE raw.nodes (osm_id BIGINT, lat DOUBLE, lon DOUBLE, tags MAP(VARCHAR, VARCHAR))")
    c.execute("CREATE TABLE raw.ways (osm_id BIGINT, tags MAP(VARCHAR, VARCHAR), refs BIGINT[])")
    c.execute("CREATE TYPE member AS ENUM ('node', 'way', 'relation')")
    c.execute("CREATE TABLE raw.relations (osm_id BIGINT, tags MAP(VARCHAR, VARCHAR), refs BIGINT[], ref_roles VARCHAR[], ref_types member[])")
    return c


def box(c, way_id, x, y, tags, size=0.001, closed=True):
    """A square way at (x, y) of side `size` degrees: four nodes, closed by the first node again."""
    base = way_id * 10
    pts = [(x, y), (x + size, y), (x + size, y + size), (x, y + size)]
    for i, (lon, lat) in enumerate(pts):
        c.execute("INSERT INTO raw.nodes VALUES (?, ?, ?, MAP {})", [base + i, lat, lon])
    refs = [base, base + 1, base + 2, base + 3] + ([base] if closed else [])
    c.execute("INSERT INTO raw.ways VALUES (?, ?, ?)", [way_id, tags, refs])


def kinds(c, table):
    return sorted(r[0] for r in c.execute(f"SELECT kind FROM features.{table}").fetchall())


def test_paved_and_the_other_new_rules(con):
    box(con, 1, 0.000, 0.000, {"place": "square", "name": "Place"})
    box(con, 2, 0.002, 0.000, {"man_made": "pier"})
    box(con, 3, 0.004, 0.000, {"highway": "pedestrian", "area": "yes"})            # a pedestrian AREA
    box(con, 4, 0.006, 0.000, {"highway": "pedestrian"})                            # a closed pedestrian road with no area=yes: not an area
    box(con, 5, 0.008, 0.000, {"aeroway": "helipad"})
    box(con, 6, 0.010, 0.000, {"amenity": "marketplace"})
    box(con, 7, 0.012, 0.000, {"area:highway": "traffic_island"})
    box(con, 8, 0.014, 0.000, {"man_made": "quay"})
    box(con, 9, 0.016, 0.000, {"leisure": "swimming_pool"})
    box(con, 10, 0.018, 0.000, {"leisure": "beach_resort"})
    box(con, 11, 0.020, 0.000, {"natural": "shrubbery"})
    box(con, 12, 0.022, 0.000, {"tourism": "zoo"})
    # a pedestrian area mapped as a multipolygon relation, with no area=yes: its ring has no tags
    box(con, 13, 0.024, 0.000, {})
    con.execute("INSERT INTO raw.relations VALUES (100, MAP {'type': 'multipolygon', 'highway': 'pedestrian'}, [13], ['outer'], ['way']::member[])")
    FeaturesBuilder(con).run()
    assert kinds(con, "paved") == sorted(["square", "pier", "pedestrian", "pedestrian", "helipad", "marketplace", "traffic_island", "quay"])
    assert con.execute("SELECT count(*) FROM features.paved WHERE osm_type = 'relation'").fetchone()[0] == 1
    assert kinds(con, "water_polygons") == ["swimming_pool"]
    assert kinds(con, "land") == ["beach_resort", "shrubbery"]
    assert kinds(con, "sites") == ["zoo"]
    # the closed pedestrian road with no area=yes is still a line in `streets`, and not a polygon in `paved`
    assert con.execute("SELECT count(*) FROM features.streets WHERE osm_id = 4").fetchone()[0] == 1
    assert con.execute("SELECT count(*) FROM features.paved WHERE osm_id = 4").fetchone()[0] == 0


def test_features_check_lists_the_misses_and_the_reasons(con):
    box(con, 1, 0.000, 0.000, {"place": "square"})                                  # in paved: not listed
    box(con, 2, 0.002, 0.000, {"building:part": "yes", "height": "20"})              # left out on purpose: a part of a building
    box(con, 3, 0.004, 0.000, {"amenity": "fountain"})                               # left out on purpose: street furniture
    box(con, 4, 0.006, 0.000, {"landuse": "salt_pond"})                              # a possible miss: no layer takes it, no reason
    box(con, 5, 0.008, 0.000, {})                                                    # a ring of a relation that a table holds: not listed
    box(con, 6, 0.010, 0.000, {"place": "neighbourhood"})                            # a zone
    con.execute("INSERT INTO raw.relations VALUES (100, MAP {'type': 'multipolygon', 'landuse': 'grass'}, [5], ['outer'], ['way']::member[])")
    FeaturesBuilder(con).run()
    groups = {g["group"]: g for g in missing_polygons(con)}
    assert "place=square" not in groups and "landuse=grass" not in groups                # held by a table
    assert not any(g["group"] == "other: " for g in groups.values())                     # the untagged ring is not a group
    assert groups["landuse=salt_pond"]["reason"] is None and groups["landuse=salt_pond"]["ways"] == 1
    assert groups["landuse=salt_pond"]["area_m2"] > 10_000                                # about 111 m x 111 m
    assert "building" in groups["building:part=yes"]["reason"] and "furniture" in groups["amenity=fountain"]["reason"]
    assert "zone" in groups["place=neighbourhood"]["reason"]
    assert missing_polygons(con)[0]["group"] == "landuse=salt_pond"                       # the possible misses come first


def test_features_check_command(con, tmp_path):
    box(con, 1, 0.000, 0.000, {"landuse": "salt_pond"})
    box(con, 2, 0.002, 0.000, {"amenity": "fountain"})
    FeaturesBuilder(con).run()
    con.close()
    out = CliRunner().invoke(main, ["features-check", str(tmp_path / "osm.duckdb")])
    assert out.exit_code == 0, out.output
    assert "landuse=salt_pond" in out.output and "NOT EXPLAINED" in out.output and "street furniture" in out.output
    assert "1 group(s) of polygons in no features table, with no reason" in out.output


def test_features_check_needs_features(tmp_path):
    c = duckdb.connect(str(tmp_path / "empty.duckdb"))
    c.execute("CREATE SCHEMA raw")
    c.close()
    out = CliRunner().invoke(main, ["features-check", str(tmp_path / "empty.duckdb")])
    assert out.exit_code != 0 and "features" in out.output
