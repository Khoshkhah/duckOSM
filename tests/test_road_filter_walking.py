"""Which ways each mode's network keeps, checked on `<mode>.ways` of the Rust build.

Walking rule (docs/guides/walking.md): a way is walkable when OSM's access rules let people walk on it.
Main roads are walkable without a sidewalk tag; motorways, motorroads and roads whose sidewalk is a
separate way are not; an explicit foot permission or sidewalk tag wins. A way with no highway tag is no road.
"""
import duckdb

from osm_build import build


def _ways(tmp_path, modes, tagsets):
    """One short way per tag set (ids 1, 2, ...): {mode: {id: highway}} of the ways each mode keeps."""
    nodes, ways = {}, {}
    for i, tags in enumerate(tagsets, 1):
        nodes[i * 10], nodes[i * 10 + 1] = (7.0, 43 + i * 0.01), (7.0012, 43 + i * 0.01)
        ways[i] = ([i * 10, i * 10 + 1], tags)
    con = duckdb.connect(str(build(tmp_path, nodes, ways, modes=modes)), read_only=True)
    return {m: dict(con.execute(f"SELECT osm_id, highway FROM {m}.ways").fetchall()) for m in modes}


def _check(tmp_path, mode, tagsets, expected):
    kept = _ways(tmp_path, (mode,), tagsets)[mode]
    for i, tags in enumerate(tagsets, 1):
        assert (i in kept) is expected, f"{mode} {tags}: kept={i in kept}"


def test_walkable_classes(tmp_path):
    hws = ["tertiary", "secondary_link", "primary", "trunk", "unclassified",
           "track", "bridleway", "road", "residential", "footway"]
    _check(tmp_path, "walking", [{"highway": h} for h in hws], True)


def test_not_walkable(tmp_path):
    _check(tmp_path, "walking", [
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
    ], False)


def test_explicit_permission_wins(tmp_path):
    _check(tmp_path, "walking", [
        {"highway": "residential", "sidewalk:left": "separate", "sidewalk:right": "no"},
        {"highway": "primary", "access": "no", "foot": "yes"},
        {"highway": "service", "sidewalk": "separate", "foot": "yes"},
        {"highway": "trunk", "motorroad": "yes", "foot": "permissive"},
        {"highway": "cycleway", "foot": "designated"},
        {"highway": "cycleway", "sidewalk": "right"},
        {"route": "ferry", "foot": "yes"},                          # no highway tag
    ], True)


def test_no_highway_tag_is_no_road(tmp_path):
    tagsets = [
        {"area:highway": "pedestrian", "bicycle": "yes", "bridge": "yes", "layer": "2"},
        {"area:highway": "pedestrian", "foot": "yes"},
        {"sidewalk": "both", "bicycle": "designated"},
    ]
    for mode in ("driving", "walking", "cycling"):
        _check(tmp_path, mode, tagsets, False)


def test_residential_in_every_network(tmp_path):
    got = _ways(tmp_path, ("driving", "walking", "cycling"), [{"highway": "residential"}])
    assert all(1 in kept for kept in got.values())


def test_ferry_line_has_class_ferry(tmp_path):
    got = _ways(tmp_path, ("walking", "cycling"), [{"route": "ferry", "foot": "yes", "bicycle": "yes"},
                                                   {"highway": "footway"}])
    for mode in ("walking", "cycling"):
        assert got[mode] == {1: "ferry", 2: "footway"}, mode
