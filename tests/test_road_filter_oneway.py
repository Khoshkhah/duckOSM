"""One-way flag, direction, access label and bus lane of a way, per mode (`<mode>.ways` of the Rust build).

Pedestrians are not bound by vehicular `oneway`: a `oneway=yes` street is walkable in both directions,
so the walking network is undirected unless an explicit `oneway:foot` says otherwise. Driving and
cycling honour vehicular `oneway` (cycling with the `oneway:bicycle` contraflow override).
`highway=residential` is kept by every mode, so it is the shared way for comparing modes.
"""
import duckdb

from osm_build import build

COLS = "osm_id, oneway, refs, lanes_fwd, access, bus_back"


def _rows(tmp_path, mode, tagsets):
    """One way per tag set (ids 1, 2, ...; nodes 10i, 10i+1, 10i+2): {id: (oneway, refs, lanes_fwd, access, bus_back)}.
    A way the mode drops has no row."""
    nodes, ways = {}, {}
    for i, tags in enumerate(tagsets, 1):
        ids = [i * 10, i * 10 + 1, i * 10 + 2]
        for k, n in enumerate(ids):
            nodes[n] = (7.0 + k * 0.0006, 43 + i * 0.01)
        ways[i] = (ids, tags)
    con = duckdb.connect(str(build(tmp_path, nodes, ways, modes=(mode,))), read_only=True)
    return {r[0]: r[1:] for r in con.execute(f"SELECT {COLS} FROM {mode}.ways").fetchall()}


def _oneway(tmp_path, mode, tags):
    rows = _rows(tmp_path, mode, [tags])
    assert 1 in rows, f"way was filtered out of the {mode} network: {tags}"
    return bool(rows[1][0])


def test_walking_ignores_vehicular_oneway(tmp_path):
    # A car one-way street is walkable both ways -> the walking way is bidirectional...
    assert not _oneway(tmp_path, "walking", {"highway": "residential", "oneway": "yes"})
    # ...while the very same way stays one-way for driving.
    assert _oneway(tmp_path, "driving", {"highway": "residential", "oneway": "yes"})


def test_walking_honours_explicit_oneway_foot(tmp_path):
    # An explicit pedestrian one-way (escalator / one-way passage) IS respected.
    rows = _rows(tmp_path, "walking", [{"highway": "footway", "oneway:foot": "yes"},
                                       {"highway": "steps", "oneway:foot": "-1"}])
    assert rows[1][0] and rows[2][0]


def test_walking_ignores_roundabout_direction(tmp_path):
    # Roundabouts are one-way for vehicles, but pedestrians walk them either way.
    assert not _oneway(tmp_path, "walking", {"highway": "footway", "junction": "roundabout"})


def test_vehicular_modes_stay_directed(tmp_path):
    rows = _rows(tmp_path, "driving", [{"highway": "primary", "oneway": "yes"}, {"highway": "motorway"}])
    assert rows[1][0] and rows[2][0]                       # a motorway is implicitly one-way
    assert _oneway(tmp_path, "cycling", {"highway": "residential", "oneway": "yes"})
    # Contraflow cycling: oneway:bicycle=no overrides the generic oneway.
    assert not _oneway(tmp_path, "cycling", {"highway": "residential", "oneway": "yes", "oneway:bicycle": "no"})


def test_oneway_minus_one_is_turned_round(tmp_path):
    """OSM oneway=-1 is one-way AGAINST the drawing: the way is reversed, so its single edge runs
    the legal way. Its lanes are the backward ones."""
    r = _rows(tmp_path, "driving", [
        {"highway": "primary", "oneway": "-1", "lanes:backward": "2"},
        {"highway": "primary", "oneway": "yes"},
        {"highway": "motorway"},
    ])
    assert r[1][0] and r[1][2] == 2
    assert r[1][1][0] > r[1][1][2]                         # turned round: the node ids run down
    assert r[2][1][0] < r[2][1][2] and r[3][1][0] < r[3][1][2]
    c = _rows(tmp_path, "cycling", [
        {"highway": "residential", "oneway:bicycle": "-1"},
        {"highway": "residential", "oneway": "-1", "oneway:bicycle": "no"},
    ])
    assert c[1][1][0] > c[1][1][2]
    assert c[2][1][0] < c[2][1][2] and not c[2][0]
    w = _rows(tmp_path, "walking", [
        {"highway": "steps", "oneway:foot": "-1"},
        {"highway": "residential", "oneway": "-1"},
    ])
    assert w[1][1][0] > w[1][1][2]
    assert w[2][1][0] < w[2][1][2] and not w[2][0]


def _access(tmp_path, mode, tagsets):
    """[(kept?, effective access)] of one way each, in `mode`."""
    rows = _rows(tmp_path, mode, tagsets)
    return [(i in rows, rows[i][3] if i in rows else None) for i in range(1, len(tagsets) + 1)]


def test_access_most_specific_tag_decides(tmp_path):
    """docs/design/access_private.md: forbidden -> out of that mode; private -> kept, marked
    'private' (the build moves it to private_edges); a more specific tag reopens a road."""
    res = {"highway": "residential"}
    drive = [
        ({**res, "access": "no"}, (False, None)),
        ({**res, "motor_vehicle": "no"}, (False, None)),
        ({**res, "access": "psv"}, (True, "bus")),                   # bus-only: kept as 'bus'
        ({**res, "motor_vehicle": "no", "bus": "yes"}, (True, "bus")),
        ({"highway": "busway"}, (True, "bus")),
        ({**res, "access": "no", "bus": "no"}, (False, None)),       # closed to all
        ({**res, "access": "private"}, (True, "private")),
        ({**res, "access": "private", "motor_vehicle": "yes"}, (True, "yes")),
        ({**res, "access": "no", "motorcar": "destination"}, (True, "destination")),
    ]
    assert _access(tmp_path, "driving", [t for t, _ in drive]) == [e for _, e in drive]
    walk = [
        ({**res, "motor_vehicle": "no"}, (True, None)),              # not about walkers
        ({**res, "foot": "no"}, (False, None)),
        ({**res, "access": "private", "foot": "yes"}, (True, "yes")),
    ]
    assert _access(tmp_path, "walking", [t for t, _ in walk]) == [e for _, e in walk]
    cyc = [
        ({**res, "vehicle": "no"}, (False, None)),
        ({**res, "vehicle": "no", "bicycle": "yes"}, (True, "yes")),
        ({**res, "access": "private"}, (True, "private")),
    ]
    assert _access(tmp_path, "cycling", [t for t, _ in cyc]) == [e for _, e in cyc]


def test_bus_lane_against_a_one_way_street(tmp_path):
    """docs/design/bus_only_edges.md: oneway=yes + oneway:bus=no marks the way bus_back (the build adds
    its reverse as a bus lane); only in driving, only on a one-way way."""
    one = {"highway": "tertiary", "oneway": "yes"}
    d = _rows(tmp_path, "driving", [
        {**one, "oneway:bus": "no"}, {**one, "oneway:psv": "no"}, one,
        {"highway": "tertiary", "oneway:bus": "no"},                 # two-way anyway
    ])
    assert [d[i][4] for i in (1, 2, 3, 4)] == [True, True, False, False]
    c = _rows(tmp_path, "cycling", [{**one, "oneway:bus": "no"}])
    assert c[1][4] is False


def test_a_contraflow_bus_lane_counts_in_lanes(tmp_path):
    """OSM's lanes counts the bus lane too: a one-way way with lanes=2 and a bus lane the other way has ONE lane its way
    (Boulevard des Moulins); lanes:forward / lanes:backward, when tagged, say it themselves."""
    one = {"highway": "tertiary", "oneway": "yes", "lanes": "2"}
    r = _rows(tmp_path, "driving", [
        {**one, "oneway:bus": "no"},
        one,                                                         # no bus lane: as before
        {**one, "lanes": "3", "oneway:bus": "no", "lanes:backward": "1"},
        {**one, "oneway:bus": "no", "lanes:forward": "2"},           # tagged: as tagged
        {**one, "lanes": "1", "oneway:bus": "no"},                   # never below one
    ])
    assert [r[i][2] for i in (1, 2, 3, 4, 5)] == [1, 2, 2, 2, 1]
