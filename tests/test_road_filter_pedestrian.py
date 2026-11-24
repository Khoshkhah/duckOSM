"""Driving keeps every highway=pedestrian way. One cars may use becomes living_street (in edges); the rest
get a label in `access` and move to private_edges (docs/design/pedestrian_private_edges.md)."""
import duckdb

from osm_build import build

CASES = [
    ({"highway": "pedestrian"}, ("pedestrian", "pedestrian")),
    ({"highway": "pedestrian", "motor_vehicle": "no"}, ("pedestrian", "pedestrian")),
    ({"highway": "pedestrian", "access": "no"}, ("pedestrian", "pedestrian")),
    ({"highway": "pedestrian", "vehicle": "yes"}, ("pedestrian", "pedestrian")),
    ({"highway": "pedestrian", "motor_vehicle": "private"}, ("pedestrian", "private")),
    ({"highway": "pedestrian", "access": "private"}, ("pedestrian", "private")),
    ({"highway": "pedestrian", "motor_vehicle": "delivery"}, ("pedestrian", "delivery")),
    # cars may use it: rescued as living_street
    ({"highway": "pedestrian", "motor_vehicle": "yes"}, ("living_street", "yes")),
    ({"highway": "pedestrian", "access": "delivery"}, ("living_street", "delivery")),
    ({"highway": "pedestrian", "access": "destination"}, ("living_street", "destination")),
]


def _build(tmp_path, tagsets, modes):
    nodes, ways = {}, {}
    for i, tags in enumerate(tagsets, 1):
        nodes[i * 10], nodes[i * 10 + 1] = (7.0, 43 + i * 0.01), (7.0012, 43 + i * 0.01)
        ways[i] = ([i * 10, i * 10 + 1], tags)
    return duckdb.connect(str(build(tmp_path, nodes, ways, modes=modes)), read_only=True)


def test_driving_pedestrian_label(tmp_path):
    con = _build(tmp_path, [t for t, _ in CASES], ("driving",))
    got = {r[0]: r[1:] for r in con.execute("SELECT osm_id, highway, access FROM driving.ways").fetchall()}
    for i, (tags, expected) in enumerate(CASES, 1):
        assert got[i] == expected, tags


def test_other_modes_unchanged(tmp_path):
    con = _build(tmp_path, [{"highway": "pedestrian"}], ("walking",))
    assert con.execute("SELECT highway FROM walking.ways").fetchall() == [("pedestrian",)]


def test_split_private_moves_only_driving_pedestrian_streets(tmp_path):
    """Pedestrian streets and bus-only roads leave driving.edges for private_edges; walking keeps its pedestrian streets."""
    con = _build(tmp_path, [{"highway": "tertiary"}, {"highway": "pedestrian"},
                            {"highway": "unclassified", "access": "psv"}, {"highway": "pedestrian", "motor_vehicle": "yes"}],
                 ("driving", "walking"))
    edges = lambda t: sorted({r[0] for r in con.execute(f"SELECT osm_id FROM {t}").fetchall()})
    assert edges("driving.edges") == [1, 4]
    assert edges("driving.private_edges") == [2, 3]
    assert 2 in edges("walking.edges")
    assert con.execute("SELECT highway FROM driving.edges WHERE osm_id = 4").fetchone() == ("living_street",)
