"""walk_type from OSM sub-tags (docs/concepts/networks.md#walk_type-and-cycle_type)."""
import duckdb

from duckosm.processors.functional_type import FunctionalType


def _walk_types(ways):
    con = duckdb.connect()
    con.execute("CREATE SCHEMA raw")
    con.execute("CREATE TABLE raw.ways(osm_id BIGINT, tags MAP(VARCHAR, VARCHAR))")
    con.execute("CREATE TABLE edges(edge_id BIGINT, osm_id BIGINT, highway VARCHAR)")
    for i, tags in enumerate(ways, 1):
        kv = ", ".join(f"'{k}': '{v}'" for k, v in tags.items())
        con.execute(f"INSERT INTO raw.ways VALUES ({i}, MAP {{{kv}}})")
        con.execute(f"INSERT INTO edges VALUES ({i}, {i}, '{tags['highway']}')")
    FunctionalType(con, mode="walking").run()
    return [r[0] for r in con.execute("SELECT walk_type FROM edges ORDER BY edge_id").fetchall()]


def test_road_kept_for_its_sidewalk_is_a_sidewalk():
    """A main road is in the walking network only because of its sidewalk tag: you walk the
    sidewalk (it used to fall through to 'footpath')."""
    assert _walk_types([
        {"highway": "primary", "sidewalk": "both"},
        {"highway": "residential", "sidewalk": "right"},
        {"highway": "residential"},
        {"highway": "residential", "sidewalk": "separate"},
        {"highway": "footway", "footway": "crossing"},
        {"highway": "path", "sidewalk": "yes"},
    ]) == ["sidewalk", "sidewalk", "shared_road", "shared_road", "crossing", "path"]
