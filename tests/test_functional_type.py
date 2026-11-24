"""walk_type from OSM sub-tags, on `walking.edges` of the Rust build (docs/concepts/networks.md#walk_type-and-cycle_type)."""
import duckdb

from osm_build import build


def _walk_types(tmp_path, tagsets):
    """One short way per tag set (ids 1, 2, ...): {id: walk_type}. A way the walking network drops has no entry."""
    nodes, ways = {}, {}
    for i, tags in enumerate(tagsets, 1):
        nodes[i * 10], nodes[i * 10 + 1] = (7.0, 43 + i * 0.01), (7.0012, 43 + i * 0.01)
        ways[i] = ([i * 10, i * 10 + 1], tags)
    con = duckdb.connect(str(build(tmp_path, nodes, ways, modes=("walking",))), read_only=True)
    return dict(con.execute("SELECT osm_id, any_value(walk_type) FROM walking.edges GROUP BY osm_id").fetchall())


def test_road_kept_for_its_sidewalk_is_a_sidewalk(tmp_path):
    """A main road is in the walking network only because of its sidewalk tag: you walk the sidewalk."""
    got = _walk_types(tmp_path, [
        {"highway": "primary", "sidewalk": "both"},
        {"highway": "residential", "sidewalk": "right"},
        {"highway": "residential"},
        {"highway": "residential", "sidewalk": "separate"},     # its sidewalk is a separate way: the road is not walked
        {"highway": "footway", "footway": "crossing"},
        {"highway": "path", "sidewalk": "yes"},
    ])
    assert got == {1: "sidewalk", 2: "sidewalk", 3: "shared_road", 5: "crossing", 6: "path"}


def test_road_without_sidewalk_tag_is_a_shared_road(tmp_path):
    """A main road walkable by its class alone is walked on the carriageway, not a footpath."""
    got = _walk_types(tmp_path, [
        {"highway": "tertiary", "oneway": "yes"},
        {"highway": "primary_link"},
        {"highway": "trunk", "sidewalk": "no"},
        {"highway": "road"},
        {"highway": "footway"},
    ])
    assert got == {1: "shared_road", 2: "shared_road", 3: "shared_road", 4: "shared_road", 5: "footpath"}
