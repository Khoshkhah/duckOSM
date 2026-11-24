"""Cycling dismount edges: footway/pedestrian enter the cycling graph as push-the-bike edges. They are kept by the
filter, bidirectional, flagged dismount=TRUE, costed at walking speed and classified cycle_type='dismount'.
Gated by options.cycling_dismount (default on); flag off restores the pre-feature graph.
See docs/design/cycling_dismount_edges.md. Checked on `cycling.ways` and `cycling.edges` of the Rust build."""
import duckdb

from osm_build import build


def _cycling(tmp_path, tagsets, config=None):
    """One way per tag set (ids 1, 2, ...; 3 nodes each, so a plain way is one edge each way at most).
    Returns a connection to the cycling build."""
    nodes, ways = {}, {}
    for i, tags in enumerate(tagsets, 1):
        ids = [i * 10, i * 10 + 1, i * 10 + 2]
        for k, n in enumerate(ids):
            nodes[n] = (7.0 + k * 0.0006, 43 + i * 0.01)
        ways[i] = (ids, tags)
    args = []
    if config:
        cfg = tmp_path / "c.yaml"
        cfg.write_text("name: t\noptions: " + config + "\n")
        args = ["-c", str(cfg)]
    return duckdb.connect(str(build(tmp_path, nodes, ways, modes=("cycling",), args=args)), read_only=True)


def _kept(con):
    return {r[0]: r[1:] for r in con.execute("SELECT osm_id, highway, oneway FROM cycling.ways").fetchall()}


# ---- filter: what enters the cycling graph ------------------------------------------

def test_footway_kept_as_dismount_even_bicycle_no(tmp_path):
    # bicycle=no forbids RIDING; pushing is walking, so the way still enters the graph.
    kept = _kept(_cycling(tmp_path, [{"highway": "footway", "bicycle": "no"}, {"highway": "pedestrian"}]))
    assert 1 in kept and 2 in kept


def test_private_kept_for_the_map_foot_no_excluded(tmp_path):
    """A dismount way is walked, so walking access decides: foot=no keeps it out; access=private
    keeps it as a private road (moved to private_edges by the build: seen, never routed)."""
    kept = _kept(_cycling(tmp_path, [{"highway": "footway", "access": "private"}, {"highway": "footway", "foot": "no"}]))
    assert 1 in kept and 2 not in kept


def test_flag_off_restores_old_filter(tmp_path):
    kept = _kept(_cycling(tmp_path, [{"highway": "footway"}, {"highway": "footway", "bicycle": "yes"}],
                          config="{cycling_dismount: false}"))
    assert 1 not in kept
    assert 2 in kept          # a rideable footway is kept regardless of the flag


def test_steps_stay_out(tmp_path):
    assert 1 not in _kept(_cycling(tmp_path, [{"highway": "steps"}]))


# ---- oneway: dismount ways are always bidirectional ----------------------------------

def test_dismount_way_ignores_oneway(tmp_path):
    kept = _kept(_cycling(tmp_path, [{"highway": "footway", "oneway": "yes"},
                                     {"highway": "footway", "bicycle": "yes", "oneway": "yes"}]))
    assert not kept[1][1]
    assert kept[2][1]         # a RIDEABLE footway keeps the vehicular rules


# ---- flag + speed + cycle_type on the built edges -------------------------------------

def test_marker_speed_and_cycle_type(tmp_path):
    con = _cycling(tmp_path, [{"highway": "footway"}, {"highway": "footway", "bicycle": "yes"}, {"highway": "cycleway"}])
    rows = {r[0]: r[1:] for r in con.execute(
        "SELECT osm_id, any_value(dismount), any_value(maxspeed_kmh), any_value(cycle_type) FROM cycling.edges GROUP BY osm_id").fetchall()}
    assert rows[1] == (True, 5.0, "dismount")
    assert rows[2][:2] == (False, 15.0)
    assert rows[3] == (False, 15.0, "cycleway")


def test_connector_follows_its_path(tmp_path):
    """A connector (negative osm_id) that extends a dismount footway is a dismount edge; one that extends a rideable
    footway is not."""
    nodes = {1: (7, 43), 2: (7.0006, 43), 3: (7.0006, 43.00004), 4: (7.0006, 43.0006), 5: (7.0012, 43.00004),
             11: (7, 43.02), 12: (7.0006, 43.02), 13: (7.0006, 43.02004), 14: (7.0006, 43.0206), 15: (7.0012, 43.02004)}
    ways = {1: ([1, 2], {"highway": "footway"}), 2: ([3, 4, 5], {"highway": "residential"}),       # a footway ends 4 m from a road
            3: ([11, 12], {"highway": "footway", "bicycle": "yes"}), 4: ([13, 14, 15], {"highway": "residential"})}
    con = duckdb.connect(str(build(tmp_path, nodes, ways, modes=("cycling",))), read_only=True)
    got = {r[0]: r[1:] for r in con.execute(
        "SELECT osm_id, any_value(dismount), any_value(cycle_type) FROM cycling.edges WHERE osm_id < 0 GROUP BY osm_id").fetchall()}
    assert got == {-1: (True, "dismount"), -3: (False, "mixed_traffic")}
