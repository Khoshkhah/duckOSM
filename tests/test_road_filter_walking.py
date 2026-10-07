"""RoadFilter walking rule: a way is walkable when OSM's access rules let people walk on it
(docs/guides/walking.md). Main roads are walkable without a sidewalk tag; motorways, motorroads and
roads whose sidewalk is a separate way are not; an explicit foot permission or sidewalk tag wins."""
import duckdb
import pytest

from duckosm.processors.road_filter import RoadFilter


def _walkable(tags):
    con = duckdb.connect()
    con.execute("CREATE SCHEMA raw")
    con.execute("CREATE TABLE raw.ways(osm_id BIGINT, tags MAP(VARCHAR, VARCHAR), refs BIGINT[])")
    kv = ", ".join(f"'{k}': '{v}'" for k, v in tags.items())
    con.execute(f"INSERT INTO raw.ways VALUES (1, MAP {{{kv}}}, [1, 2])")
    RoadFilter(con, mode="walking")._create_ways_table()
    return con.execute("SELECT count(*) FROM ways").fetchone()[0] == 1


@pytest.mark.parametrize("hw", ["tertiary", "secondary_link", "primary", "trunk", "unclassified",
                                "track", "bridleway", "road", "residential", "footway"])
def test_walkable_classes(hw):
    assert _walkable({"highway": hw})


@pytest.mark.parametrize("tags", [
    {"highway": "motorway"},
    {"highway": "motorway_link"},
    {"highway": "motorway", "foot": "yes"},                     # a motorway never
    {"highway": "trunk", "motorroad": "yes"},
    {"highway": "primary", "foot": "no"},
    {"highway": "primary", "access": "no"},
    {"highway": "secondary", "foot": "use_sidepath"},
    {"highway": "tertiary", "sidewalk": "separate"},
    {"highway": "residential", "sidewalk:both": "separate"},
    {"highway": "residential", "sidewalk:left": "separate", "sidewalk:right": "separate"},
    {"highway": "cycleway"},
])
def test_not_walkable(tags):
    assert not _walkable(tags)


@pytest.mark.parametrize("tags", [
    {"highway": "residential", "sidewalk:left": "separate", "sidewalk:right": "no"},
    {"highway": "primary", "access": "no", "foot": "yes"},
    {"highway": "service", "sidewalk": "separate", "foot": "yes"},
    {"highway": "trunk", "motorroad": "yes", "foot": "permissive"},
    {"highway": "cycleway", "foot": "designated"},
    {"highway": "cycleway", "sidewalk": "right"},
    {"route": "ferry", "foot": "yes"},                          # no highway tag
])
def test_explicit_permission_wins(tags):
    assert _walkable(tags)
