"""docs/design/gmns_crossings.md: where a zebra is painted, on which lanes, on which stretch of each."""
import duckdb
import pytest

pytest.importorskip("pandas")
from duckosm.gmns import to_gmns  # noqa: E402
from tests.test_gmns import _source  # noqa: E402

M = 111320.0


def _build(tmp_path, way=None, node=None, name="c", pre_sql=None):
    """Roads A / AR (way 100, two lanes each, east-west along y = 59.32, x 18.06 .. 18.07). ``way``: (tags, [(lon, lat), ...]) a
    crossing way; ``node``: (tags, lon): a crossing node on way 100 (no crossing way)."""
    src = tmp_path / f"{name}.duckdb"
    _source(src)
    con = duckdb.connect(str(src))
    if pre_sql:
        con.execute(pre_sql)
    if node:
        tags, lon = node
        con.execute(f"INSERT INTO raw.nodes VALUES (800, 59.32, {lon}, {tags})")
        con.execute("UPDATE raw.ways SET refs = [1, 800, 2] WHERE osm_id = 100")
    if way:
        tags, pts = way
        for i, (lon, lat) in enumerate(pts):
            con.execute(f"INSERT INTO raw.nodes VALUES ({700 + i}, {lat}, {lon}, MAP{{}})")
        refs = [700, 800, 701] if node else [700 + i for i in range(len(pts))]     # the node on the road is the crossing way's middle node
        con.execute(f"INSERT INTO raw.ways VALUES (900, {tags}, {refs})")
    con.close()
    out = tmp_path / f"{name}_gmns.duckdb"
    to_gmns(str(src), str(out), modes=["driving"])
    c = duckdb.connect(str(out), read_only=True)
    c.execute("LOAD spatial")
    return c


ACROSS = [(18.065, 59.32 - 12 / M), (18.065, 59.32 + 12 / M)]


def test_a_crossing_way_across_the_road_is_a_crossing_on_each_lane_it_covers(tmp_path):
    c = _build(tmp_path, way=("MAP{'highway':'footway','footway':'crossing','crossing':'marked'}", ACROSS))
    rows = c.execute("SELECT crossing_id, source, crossing_type, painted, width, length, osm_id FROM gmns_driving.crossing").fetchall()
    assert len(rows) == 1 and rows[0][1:4] == ("way", "marked", True) and rows[0][6] == 900
    assert rows[0][4] == pytest.approx(min(max(0.4 * rows[0][5], 2.5), 4.0), abs=1e-6)       # the width is estimated from the road crossed
    lc = c.execute("SELECT l.lane_num, lc.start_lr, lc.end_lr, lc.across_from, lc.across_to FROM gmns_driving.lane_crossing lc "
                   "JOIN gmns_driving.lane l USING (lane_id) ORDER BY lc.across_from").fetchall()
    assert c.execute("SELECT count(DISTINCT link_id) FROM gmns_driving.lane_crossing").fetchone()[0] == 2     # both directions' carriageways
    assert rows[0][5] == pytest.approx(3.25 * len(lc), abs=0.3)       # its length is the lanes it crosses, side by side
    assert lc[0][3] == pytest.approx(0.0, abs=1e-6) and lc[-1][4] == pytest.approx(rows[0][5], abs=1e-6)
    assert all(b - a == pytest.approx(rows[0][4], abs=0.2) for _, a, b, _, _ in lc)   # the zebra's width along each lane
    assert all(lc[i][4] <= lc[i + 1][3] + 0.05 for i in range(len(lc) - 1))     # the lanes follow one another across the zebra
    # across grows northwards: to the left of A (heading east), to the right of AR (heading west)
    left = dict(c.execute("SELECT link_id, bool_and(to_left) FROM gmns_driving.lane_crossing GROUP BY link_id").fetchall())
    assert sorted(left.values()) == [False, True]


def test_the_zebra_is_a_rectangle_square_to_the_road_even_when_the_crossing_is_skewed(tmp_path):
    """The best rectangle for the road: its sides along and across the (east-west) road, however the crossing way runs; on every lane of
    a link the same stretch, so the band is one straight line."""
    skew = [(18.065 - 6 / (M * 0.51), 59.32 - 6 / M), (18.065 + 6 / (M * 0.51), 59.32 + 6 / M)]          # 45 degrees to the road
    for name, pts in (("sq", ACROSS), ("skew", skew)):
        c = _build(tmp_path, way=("MAP{'footway':'crossing'}", pts), name=name)
        area, box = c.execute("SELECT ST_Area(geom), ST_Area(ST_Envelope(geom)) FROM gmns_driving.crossing").fetchone()
        assert area == pytest.approx(box, rel=1e-3)                                                      # axis-aligned: a rectangle
        lanes = c.execute("""SELECT lc.link_id, lc.start_lr, lc.end_lr,
                                    ST_X(ST_LineInterpolatePoint(l.geom, lc.start_lr / (ST_Length(l.geom) * 111320 * cos(radians(59.32))))),
                                    ST_X(ST_LineInterpolatePoint(l.geom, lc.end_lr / (ST_Length(l.geom) * 111320 * cos(radians(59.32)))))
                             FROM gmns_driving.lane_crossing lc JOIN gmns_driving.lane l USING (lane_id)""").fetchall()
        wd = c.execute("SELECT width FROM gmns_driving.crossing").fetchone()[0]
        assert all(e - s_ == pytest.approx(wd, abs=0.2) for _, s_, e, _, _ in lanes)                     # width along every lane
        xs = sorted(x for r in lanes for x in r[3:])
        assert xs[-1] - xs[0] == pytest.approx(wd / (M * 0.51), abs=0.3 / (M * 0.51))                   # every lane's two cuts are the same two lines


def test_a_crossing_along_the_road_and_a_footway_clear_of_it_are_none(tmp_path):
    along = [(18.062, 59.32 + 0.4 / M), (18.068, 59.32 + 0.4 / M)]
    c = _build(tmp_path, way=("MAP{'footway':'crossing'}", along), name="along")
    assert c.execute("SELECT count(*) FROM gmns_driving.crossing").fetchone()[0] == 0
    far = [(18.065, 59.32 + 30 / M), (18.065, 59.32 + 40 / M)]
    c = _build(tmp_path, way=("MAP{'footway':'crossing'}", far), name="far")
    assert c.execute("SELECT count(*) FROM gmns_driving.lane_crossing").fetchone()[0] == 0


def test_painted_follows_the_osm_tags(tmp_path):
    painted = lambda tags, name: _build(tmp_path, way=(tags, ACROSS), name=name).execute(      # noqa: E731
        "SELECT painted FROM gmns_driving.crossing").fetchone()[0]
    assert painted("MAP{'footway':'crossing','crossing':'zebra'}", "p1") is True
    assert painted("MAP{'footway':'crossing','crossing':'uncontrolled','crossing:markings':'yes'}", "p2") is True
    assert painted("MAP{'footway':'crossing','crossing':'unmarked'}", "p3") is False
    assert painted("MAP{'footway':'crossing','crossing':'marked','crossing:markings':'no'}", "p4") is False
    assert painted("MAP{'footway':'crossing','crossing':'traffic_signals'}", "p5") is False         # signals alone: no stripes
    assert painted("MAP{'footway':'crossing'}", "p6") is False


def test_a_crossing_node_with_no_way_is_a_crossing_square_across_the_road(tmp_path):
    c = _build(tmp_path, node=("MAP{'highway':'crossing','crossing':'marked'}", 18.065))
    rows = c.execute("SELECT crossing_id, source, painted, length, ST_XMin(geom), ST_XMax(geom) FROM gmns_driving.crossing").fetchall()
    n_lanes = c.execute("SELECT count(*) FROM gmns_driving.lane_crossing").fetchone()[0]
    assert len(rows) == 1 and rows[0][1:3] == ("node", True) and rows[0][3] == pytest.approx(3.25 * n_lanes, abs=0.3) and n_lanes >= 2
    assert (rows[0][4] + rows[0][5]) / 2 == pytest.approx(18.065, abs=1e-6) and (rows[0][5] - rows[0][4]) * M * 0.51 == pytest.approx(c.execute('SELECT width FROM gmns_driving.crossing').fetchone()[0], abs=0.3)   # `width` along the road, centred on the node
    assert c.execute("SELECT count(DISTINCT link_id) FROM gmns_driving.lane_crossing").fetchone()[0] == 2


def test_the_nodes_of_a_crossing_way_are_not_crossings_of_their_own(tmp_path):
    c = _build(tmp_path, way=("MAP{'footway':'crossing','crossing':'marked'}", ACROSS), node=("MAP{'highway':'crossing'}", 18.065), name="both")
    assert c.execute("SELECT count(*) FROM gmns_driving.crossing").fetchone()[0] == 1               # the way and the node on it: one crossing


def test_a_crossing_over_a_bike_lane_only_is_not_a_zebra(tmp_path):
    c = _build(tmp_path, way=("MAP{'footway':'crossing'}", ACROSS), name="bike")
    assert c.execute("SELECT count(*) FROM gmns_driving.lane_crossing lc JOIN gmns_driving.lane l USING (lane_id) "
                     "WHERE l.allowed_uses NOT IN ('auto', 'bus')").fetchone()[0] == 0


def test_the_zebra_is_wider_on_a_wider_road_and_a_tagged_width_wins(tmp_path):
    c = _build(tmp_path, way=("MAP{'footway':'crossing'}", ACROSS), name="est")
    wide = c.execute("SELECT length, width FROM gmns_driving.crossing").fetchone()
    assert wide[1] == pytest.approx(min(max(0.4 * wide[0], 2.5), 4.0)) and 2.5 < wide[1] <= 4.0          # a 2-link road of 3 lanes: about 4 m
    c = _build(tmp_path, way=("MAP{'footway':'crossing','width':'2.0'}", ACROSS), name="tag")
    assert c.execute("SELECT width FROM gmns_driving.crossing").fetchone()[0] == 2.0


def test_a_crossing_paints_only_on_the_lanes_of_its_own_level(tmp_path):
    """A ground crossing over a road in a tunnel is no crossing of that road; a crossing that is itself in the tunnel (layer -1) is."""
    tunnel = "ALTER TABLE driving.edges ADD COLUMN tunnel VARCHAR; UPDATE driving.edges SET tunnel = 'yes'"
    ground = _build(tmp_path, way=("MAP{'footway':'crossing'}", ACROSS), name="lvl0", pre_sql=tunnel)
    assert ground.execute("SELECT count(*) FROM gmns_driving.crossing").fetchone()[0] == 0
    under = _build(tmp_path, way=("MAP{'footway':'crossing','layer':'-1'}", ACROSS), name="lvl1", pre_sql=tunnel)
    assert under.execute("SELECT count(*) FROM gmns_driving.crossing").fetchone()[0] == 1
    flat = _build(tmp_path, way=("MAP{'footway':'crossing'}", ACROSS), name="lvl2")                     # no level columns: all ground
    assert flat.execute("SELECT count(*) FROM gmns_driving.crossing").fetchone()[0] == 1


def test_a_crossing_over_a_junction_corner_is_one_square_zebra_per_road(tmp_path):
    """docs/design/gmns_crossings.md, "One rectangle per road": a way running 45 degrees across the corner of A (east-west) and B (north-south) is two zebras,
    each square to its own road, and no lane of one road under the other's rectangle (before: one rectangle at 45 degrees to both, on 12 lanes of the two roads)."""
    kx = M * 0.5106                                                                # metres per degree of longitude at 59.32
    node = (18.07, 59.32)
    way = [(node[0] + dx / kx, node[1] + dy / M) for dx, dy in ((-16, 6), (6, -16))]       # 45 degrees, through (-5, -5)... across both roads
    c = _build(tmp_path, way=("MAP{'highway':'footway','footway':'crossing','crossing':'marked'}", way), name="corner")
    ids = [r[0] for r in c.execute("SELECT crossing_id FROM gmns_driving.crossing ORDER BY crossing_id").fetchall()]
    assert len(ids) == 2 and ids[0] == "w900" and ids[1] == "w900#2"
    links = {cid: {r[0] for r in c.execute("SELECT DISTINCT link_id FROM gmns_driving.lane_crossing WHERE crossing_id = ?", [cid]).fetchall()} for cid in ids}
    from tests.test_gmns import A, AR, B
    assert sorted(map(sorted, links.values())) == sorted([sorted({A, AR}), sorted({B})])    # one road each: the east-west pair, the north-south link
    for cid in ids:                                                                # each rectangle is axis-aligned: square to its (axis-aligned) road
        area, box = c.execute("SELECT ST_Area(geom), ST_Area(ST_Envelope(geom)) FROM gmns_driving.crossing WHERE crossing_id = ?", [cid]).fetchone()
        assert area == pytest.approx(box, rel=1e-3)


def test_a_zebra_spans_every_lane_of_the_roads_it_crosses_even_when_the_crossing_way_ends_inside_the_first_lane(tmp_path):
    """docs/design/gmns_crossings.md, "A zebra crosses a road": the crossing way crosses the near carriageway and ends 1 m into the first lane of the far one; the zebra is across all the lanes of
    both carriageways here (3 car lanes, 9.75 m), not 4.25 m over the two lanes the line reaches."""
    c = _build(tmp_path, way=("MAP{'footway':'crossing'}", [(18.065, 59.32 - 12 / M), (18.065, 59.32 + 1 / M)]))
    length = c.execute("SELECT length FROM gmns_driving.crossing").fetchone()[0]
    lanes = c.execute("SELECT DISTINCT lane_id FROM gmns_driving.lane_crossing").fetchall()
    assert len(lanes) == 3 and length == pytest.approx(3 * 3.25, abs=0.4), (len(lanes), length)


def test_a_crossing_way_with_several_zebra_nodes_on_roads_is_no_line_its_nodes_are_the_crossings(tmp_path):
    """docs/design/gmns_crossings.md, "A crossing way that is a loop": way 901 runs through three zebra nodes (801-803) that lie on road way 100; it is no crossing line. Each node is a zebra square across its road."""
    node = "MAP{'highway':'crossing','crossing':'marked','crossing:markings':'yes'}"
    pre = (f"INSERT INTO raw.nodes VALUES (801, 59.32, 18.062, {node}); INSERT INTO raw.nodes VALUES (802, 59.32, 18.065, {node}); INSERT INTO raw.nodes VALUES (803, 59.32, 18.068, {node}); "
           "UPDATE raw.ways SET refs = [1, 801, 802, 803, 2] WHERE osm_id = 100; "
           "INSERT INTO raw.ways VALUES (901, MAP{'highway':'footway','footway':'crossing','crossing':'marked'}, [801, 802, 803])")
    c = _build(tmp_path, pre_sql=pre)
    rows = c.execute("SELECT source, osm_id, painted FROM gmns_driving.crossing ORDER BY osm_id").fetchall()
    assert rows == [("node", 801, True), ("node", 802, True), ("node", 803, True)], rows
