"""Where roads are cut (main.global_junctions): at a node another OSM way uses (a road, or a
footway / path / cycleway joining it mid-way), and where a road ends. docs/design/split_at_every_way.md"""
import duckdb

from duckosm.processors.global_junctions import GlobalJunctions


def _junctions(ways):
    con = duckdb.connect()
    con.execute("CREATE SCHEMA raw")
    con.execute("CREATE TABLE raw.ways(osm_id BIGINT, tags MAP(VARCHAR, VARCHAR), refs BIGINT[])")
    for i, (hw, refs) in enumerate(ways, 1):
        con.execute(f"INSERT INTO raw.ways VALUES ({i}, MAP {{'highway': '{hw}'}}, {refs})")
    GlobalJunctions(con).run()
    return {r[0] for r in con.execute("SELECT node_id FROM main.global_junctions").fetchall()}


def test_a_crosswalk_cuts_the_road():
    # road 1-2-3; a crosswalk 7-2-8 crosses it at 2 (mid-way): 2 is a cut, so walkers connect there
    assert _junctions([("residential", [1, 2, 3]), ("footway", [7, 2, 8])]) == {1, 2, 3}


def test_what_does_not_cut_a_road():
    # a path's own ends away from roads, and a planned path, are not road cuts
    assert _junctions([("residential", [1, 2, 3]), ("footway", [8, 9])]) == {1, 3}
    assert _junctions([("residential", [1, 2, 3]), ("proposed", [7, 2, 8])]) == {1, 3}
    # two roads meeting: cut (unchanged)
    assert _junctions([("residential", [1, 2, 3]), ("service", [2, 5])]) == {1, 2, 3, 5}
