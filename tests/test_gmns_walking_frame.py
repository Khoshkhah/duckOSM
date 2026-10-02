"""A sidewalk takes its place from its road's cross-section (docs/design/gmns_walking_frame.md)."""
import duckdb
import pytest

pytest.importorskip("pandas")
from duckosm.gmns import to_gmns  # noqa: E402
from tests.test_gmns import A, AR, _source  # noqa: E402

M = 111320.0


def _run(tmp_path, y, tags="MAP{'highway':'footway','footway':'sidewalk'}", road_in_walking=True, both_ways=False, virtual=False, layer=None, **kw):
    """Roads A / AR along y = 59.32; one footway (edge 9001, way 200) along it at latitude ``y``."""
    n = len(list(tmp_path.glob("s_*.duckdb")))
    src = tmp_path / f"s_{n}.duckdb"
    _source(src)
    con = duckdb.connect(str(src))
    con.execute("LOAD spatial; CREATE SCHEMA walking")
    con.execute("CREATE TABLE walking.nodes AS SELECT * FROM driving.nodes")
    con.execute("CREATE TABLE walking.edges AS SELECT * FROM driving.edges")
    if not road_in_walking:                                  # a road without a sidewalk is not in the walking network
        con.execute("DELETE FROM walking.edges")
    con.execute(f"INSERT INTO raw.ways VALUES (200, {tags}, [1,2])")
    con.execute("INSERT INTO walking.edges (edge_id, source, target, osm_id, highway, lanes, is_reverse, length_m, maxspeed_kmh, geometry) "
                f"VALUES (9001, 1, 2, 200, 'footway', 1, false, 80, 5, ST_GeomFromText('LINESTRING(18.06 {y}, 18.07 {y})'))")
    if both_ways:                                            # the way back: a footpath's second link
        con.execute("INSERT INTO walking.edges (edge_id, source, target, osm_id, highway, lanes, is_reverse, length_m, maxspeed_kmh, geometry) "
                    f"VALUES (9002, 2, 1, 200, 'footway', 1, true, 80, 5, ST_GeomFromText('LINESTRING(18.07 {y}, 18.06 {y})'))")
    if layer:                                                # the footway is on another level than the road
        con.execute("ALTER TABLE walking.edges ADD COLUMN layer VARCHAR")
        con.execute(f"UPDATE walking.edges SET layer = '{layer}' WHERE edge_id = 9001")
    if virtual:                                              # a way duckOSM split carries a virtual id: its way's id with a minus
        con.execute("UPDATE walking.edges SET osm_id = -200 WHERE osm_id = 200")
    con.close()
    out = str(tmp_path / f"o_{n}.duckdb")
    kw.setdefault("walk_frame", True)                        # the moves are opt-in: these tests are about them
    to_gmns(str(src), out, modes=["walking"] if road_in_walking else ["driving", "walking"], **kw)
    c = duckdb.connect(out, read_only=True)
    c.execute("LOAD spatial")
    return c


def _gap_m(c, parent):
    """Metres between the footway lane and the parent road's kerb-side lane (they run east-west: a latitude difference)."""
    f = c.execute("SELECT ST_Y(ST_StartPoint(geom)), ST_Y(ST_EndPoint(geom)) FROM gmns_walking.lane WHERE link_id = 9001").fetchone()
    p = c.execute(f"SELECT ST_Y(ST_StartPoint(geom)) FROM gmns_walking.lane WHERE link_id = {parent} ORDER BY lane_num DESC").fetchone()[0]
    assert f[0] == pytest.approx(f[1], abs=1e-7)             # parallel to the road
    return abs(f[0] - p) * M


def test_a_sidewalk_drawn_on_the_carriageway_is_placed_clear_of_its_lanes(tmp_path):
    # drawn 1 m north of the road line: with its own line it would lie on the road (a lane 3.25 m wide)
    c = _run(tmp_path, 59.32 + 1 / M, walk_frame=False)
    assert c.execute("SELECT parent_link_id FROM gmns_walking.link WHERE link_id = 9001").fetchone()[0] == AR
    assert _gap_m(c, AR) != pytest.approx(3.25 / 2 + 2.0 / 2, abs=0.05)
    c = _run(tmp_path, 59.32 + 1 / M)                        # frame: touching the kerb = (3.25 + 2.0) / 2 from the lane's centre
    assert _gap_m(c, AR) == pytest.approx(3.25 / 2 + 2.0 / 2, abs=0.05)
    c = _run(tmp_path, 59.32 + 1 / M, walk_clearance_m=0.5)
    assert _gap_m(c, AR) == pytest.approx(3.25 / 2 + 0.5 + 2.0 / 2, abs=0.05)
    # the link, its ends and its length are as before
    q = "SELECT from_node_id, to_node_id, length FROM gmns_walking.link WHERE link_id = 9001"
    assert c.execute(q).fetchone() == _run(tmp_path, 59.32 + 1 / M, walk_frame=False).execute(q).fetchone()


def test_an_untagged_footway_is_never_moved_next_to_a_road(tmp_path):
    """No sidewalk, no footpath beside the road: a footway without footway=sidewalk keeps its own place, however near the road."""
    c = _run(tmp_path, 59.32 + 1 / M, "MAP{'highway':'footway'}")
    assert c.execute("SELECT parent_link_id FROM gmns_walking.link WHERE link_id = 9001").fetchone()[0] is None
    y = c.execute("SELECT ST_Y(ST_StartPoint(geom)) FROM gmns_walking.lane WHERE link_id = 9001").fetchone()[0]
    assert y == pytest.approx(59.32 + 1 / M, abs=1e-7)                     # centred on its own line, as mapped


def test_a_road_that_is_only_in_the_driving_network_is_the_frame_too(tmp_path):
    c = _run(tmp_path, 59.32 + 1 / M, road_in_walking=False)
    # no parent_link_id: the road is not a link of gmns_walking (the key would point out of its table) ...
    assert c.execute("SELECT parent_link_id FROM gmns_walking.link WHERE link_id = 9001").fetchone()[0] is None
    # ... but the footway is placed from its lanes, the two of AR
    f = c.execute("SELECT ST_Y(ST_StartPoint(geom)) FROM gmns_walking.lane WHERE link_id = 9001").fetchone()[0]
    k = c.execute(f"SELECT max(ST_Y(ST_StartPoint(geom))) FROM gmns_driving.lane WHERE link_id = {AR}").fetchone()[0]
    assert (f - k) * M == pytest.approx(3.25 / 2 + 2.0 / 2, abs=0.05)


def test_a_footpath_is_one_strip_for_both_directions(tmp_path):
    """Step 3: a footpath's two links lie on its own line (people walk both ways), not 3.25 m apart on either side."""
    y = 59.32 + 20 / M                                       # far from the roads: nothing moves it
    c = _run(tmp_path, y, "MAP{'highway':'footway'}", both_ways=True)
    ys = {k: v for k, v in c.execute("SELECT link_id, ST_Y(ST_StartPoint(geom)) FROM gmns_walking.lane WHERE link_id IN (9001, 9002)").fetchall()}
    assert ys[9001] == pytest.approx(y, abs=1e-7) and ys[9002] == pytest.approx(y, abs=1e-7)


def test_a_sidewalk_still_on_the_road_is_pushed_out_to_the_kerb_and_not_deleted(tmp_path, monkeypatch):
    """Plan A: a mapped sidewalk that lies inside a road (here the frame step is off, as for a far-side sidewalk) is moved to the road's
    kerb, half its width beyond it; a footway that is no sidewalk stays where it is."""
    monkeypatch.setattr("duckosm.gmns._walk_frame", lambda *a, **k: None)
    inside = 59.32 + 3 / M                                   # inside AR's lanes (the road is 13 m wide: y within +-6.5 m)
    c = _run(tmp_path, inside, road_in_walking=False)
    y = c.execute("SELECT ST_Y(ST_LineInterpolatePoint(geom, 0.1)), ST_Y(ST_LineInterpolatePoint(geom, 0.5)), "
                  "ST_Y(ST_LineInterpolatePoint(geom, 0.9)) FROM gmns_walking.lane WHERE link_id = 9001").fetchone()
    assert all((v - 59.32) * M == pytest.approx(7.5, abs=0.2) for v in y)                    # out through the nearer kerb (north), + half its width
    plain = _run(tmp_path, inside, "MAP{'highway':'footway'}", road_in_walking=False)
    y = plain.execute("SELECT ST_Y(ST_StartPoint(geom)) FROM gmns_walking.lane WHERE link_id = 9001").fetchone()[0]
    assert abs((y - inside) * M) < 1.0                                                       # not a sidewalk: not moved


def test_an_edge_with_a_virtual_negative_id_reads_its_ways_tags(tmp_path):
    """-200 is way 200 split: its tags (a footway, a crossing) are the way's, so its two links are one strip on its line, not two
    lanes 2 m to either side as for a road, and `link.osm_id` is the real way."""
    y = 59.32 + 20 / M
    c = _run(tmp_path, y, "MAP{'highway':'footway','footway':'crossing','crossing':'marked'}", both_ways=True, virtual=True)
    assert c.execute("SELECT osm_id, footway, crossing FROM gmns_walking.link WHERE link_id = 9001").fetchone() == (200, "crossing", "marked")
    ys = [v for (v,) in c.execute("SELECT ST_Y(ST_StartPoint(geom)) FROM gmns_walking.lane WHERE link_id IN (9001, 9002)").fetchall()]
    assert all(v == pytest.approx(y, abs=1e-7) for v in ys)


def test_a_sidewalk_runs_along_a_road_of_its_own_level_only(tmp_path):
    same = _run(tmp_path, 59.32 + 1 / M)
    assert same.execute("SELECT along_link_id IS NOT NULL, along_mode FROM gmns_walking.link WHERE link_id = 9001").fetchone() == (True, "walking")
    over = _run(tmp_path, 59.32 + 1 / M, layer="1")      # a layer above the road
    assert over.execute("SELECT along_link_id FROM gmns_walking.link WHERE link_id = 9001").fetchone()[0] is None


def test_an_unmarked_footway_right_beside_a_road_is_labelled_adjacent_and_never_moved(tmp_path):
    near = _run(tmp_path, 59.32 + 3.2 / M, tags="MAP{'highway':'footway'}")           # about 0.6 m beside the kerb, no footway=sidewalk
    assert near.execute("SELECT along_kind, parent_link_id FROM gmns_walking.link WHERE link_id = 9001").fetchone() == ("adjacent", None)
    far = _run(tmp_path, 59.32 + 25 / M, tags="MAP{'highway':'footway'}")
    assert far.execute("SELECT along_link_id FROM gmns_walking.link WHERE link_id = 9001").fetchone()[0] is None
    y0 = 59.32 + 3.2 / M
    y = near.execute("SELECT ST_Y(ST_StartPoint(geom)) FROM gmns_walking.lane WHERE link_id = 9001").fetchone()[0]
    assert abs(y - y0) * M < 0.05                                                      # exactly where OSM maps it


def test_a_sidewalk_is_matched_to_every_road_it_runs_along_not_only_one(tmp_path):
    c = _run(tmp_path, 59.32 + 1 / M)
    rows = c.execute("SELECT along_link_id, along_mode, covered_m FROM gmns_walking.link_along WHERE link_id = 9001").fetchall()
    assert rows and all(m == "walking" and cov >= 4 for _, m, cov in rows)          # each road in the route runs along it for 4 m or more
    assert c.execute("SELECT along_link_id FROM gmns_walking.link WHERE link_id = 9001").fetchone()[0] in {r[0] for r in rows}
