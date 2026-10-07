"""Where roads are cut (main.global_junctions): at a node another OSM way uses (a road, or a
footway / path / cycleway joining it mid-way), and where a road ends. docs/design/split_at_every_way.md"""
import duckdb

from duckosm.processors.global_junctions import GlobalJunctions


def _junctions(ways, road=True):
    con = duckdb.connect()
    con.execute("CREATE SCHEMA raw")
    con.execute("CREATE TABLE raw.ways(osm_id BIGINT, tags MAP(VARCHAR, VARCHAR), refs BIGINT[])")
    for i, (hw, refs) in enumerate(ways, 1):
        con.execute(f"INSERT INTO raw.ways VALUES ({i}, MAP {{'highway': '{hw}'}}, {refs})")
    GlobalJunctions(con).run()
    return {r[0] for r in con.execute(
        "SELECT node_id FROM main.global_junctions WHERE is_road_junction = ?", [road]).fetchall()}


def test_a_crosswalk_cuts_the_road():
    # road 1-2-3; a crosswalk 7-2-8 crosses it at 2 (mid-way): 2 is a cut, so walkers connect there
    assert _junctions([("residential", [1, 2, 3]), ("footway", [7, 2, 8])]) == {1, 2, 3}


def test_what_does_not_cut_a_road():
    # a path's own ends away from roads, and a planned path, are not road cuts
    assert _junctions([("residential", [1, 2, 3]), ("footway", [8, 9])]) == {1, 3}
    assert _junctions([("residential", [1, 2, 3]), ("proposed", [7, 2, 8])]) == {1, 3}
    # two roads meeting: cut (unchanged)
    assert _junctions([("residential", [1, 2, 3]), ("service", [2, 5])]) == {1, 2, 3, 5}


def test_paths_are_cut_where_any_way_meets_them():
    # pedestrian street 1-2-3-4: a footway joins at 2 (only walking has it), a cycleway at 3 (only
    # cycling): both are path cuts for every mode, so the street is cut the same way in both; they
    # are not road cuts. A path's own end in the open (1, 4) is not a shared cut; 9, where two meet, is.
    ways = [("pedestrian", [1, 2, 3, 4]), ("footway", [2, 9]), ("cycleway", [3, 8]), ("cycleway", [8, 9])]
    assert _junctions(ways, road=False) == {2, 3, 8, 9}
    assert _junctions(ways) == set()
