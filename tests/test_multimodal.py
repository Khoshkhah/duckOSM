"""Tests for intermodal (multimodal) routing — the ``mm.*`` layered graph.

Covers :class:`duckosm.processors.multimodal.MultimodalBuilder` (union edges + coarse transfers)
and :func:`duckosm.routing.route_multimodal` (node-based Dijkstra with the walk* veh* walk* guard).

Fast tests build a tiny in-memory DuckDB with hand-written ``walking`` / ``driving`` (/ ``cycling``)
schemas sharing junction ``node_id``s — mirroring ``tests/test_routing.py`` /
``tests/test_road_filter_oneway.py``. One gated integration test builds a real 2-mode area from the
metro-vancouver PBF (skipped if the data file or ``osmium`` is absent).

Run: ``pytest tests/test_multimodal.py -q``
"""
import shutil
from pathlib import Path

import duckdb
import pytest

from duckosm import Config, route_multimodal
from duckosm.processors import MultimodalBuilder


# ---- fixtures ---------------------------------------------------------------------------------

def _wkt(w):
    return f"ST_GeomFromText('{w}')"


def _new_db(modes=("walking", "driving")):
    """Empty in-memory db with one nodes+edges schema per mode (columns match graph_builder)."""
    con = duckdb.connect()
    con.execute("INSTALL spatial; LOAD spatial;")
    for m in modes:
        con.execute(f"CREATE SCHEMA {m}")
        con.execute(f"CREATE TABLE {m}.nodes(node_id BIGINT, geom GEOMETRY)")
        con.execute(
            f"CREATE TABLE {m}.edges(edge_id BIGINT, source BIGINT, target BIGINT, cost_s DOUBLE, "
            f"length_m DOUBLE, highway VARCHAR, name VARCHAR, geometry GEOMETRY)")
    return con


def _add_node(con, mode, nid, x, y):
    con.execute(f"INSERT INTO {mode}.nodes VALUES ({nid}, {_wkt(f'POINT({x} {y})')})")


def _add_edge(con, mode, eid, s, t, cost, length=100.0, x0=0, y0=0, x1=1, y1=0, hw="residential"):
    con.execute(
        f"INSERT INTO {mode}.edges VALUES ({eid},{s},{t},{cost},{length},'{hw}','n',"
        f"{_wkt(f'LINESTRING({x0} {y0},{x1} {y1})')})")


def _mm_db():
    """walking 10->20->30 (edges 101,102) + driving 20->40 (edge 201); junction 20 shared."""
    con = _new_db()
    for m in ("walking", "driving"):
        _add_node(con, m, 20, 1, 0)
    _add_node(con, "walking", 10, 0, 0)
    _add_node(con, "walking", 30, 2, 0)
    _add_node(con, "driving", 40, 1, 1)
    _add_edge(con, "walking", 101, 10, 20, 50, x0=0, y0=0, x1=1, y1=0, hw="footway")
    _add_edge(con, "walking", 102, 20, 30, 50, x0=1, y0=0, x1=2, y1=0, hw="footway")
    _add_edge(con, "driving", 201, 20, 40, 10, x0=1, y0=0, x1=1, y1=1)
    return con


# ---- MultimodalBuilder: mm.edges ---------------------------------------------------------------

def test_builder_unions_edges_with_mode():
    con = _mm_db()
    stats = MultimodalBuilder(con, transfer_s=60).run()
    # rows = Σ per-mode edges, each tagged with its mode
    assert stats["edge_count"] == 3
    by_mode = dict(con.execute(
        "SELECT mode, COUNT(*) FROM mm.edges GROUP BY mode").fetchall())
    assert by_mode == {"walking": 2, "driving": 1}
    cols = [r[1] for r in con.execute("PRAGMA table_info('mm.edges')").fetchall()]
    assert "mode" in cols and "cost_s" in cols and "geometry" in cols


def test_builder_key_is_mode_plus_edge_id():
    """edge_id collides across modes; only (mode, edge_id) is unique."""
    con = _new_db()
    for m in ("walking", "driving"):
        _add_node(con, m, 1, 0, 0)
        _add_node(con, m, 2, 1, 0)
        _add_edge(con, m, 999, 1, 2, 30)          # SAME edge_id in both modes (real: no mode in hash)
    MultimodalBuilder(con).run()
    assert con.execute("SELECT COUNT(*) FROM mm.edges WHERE edge_id = 999").fetchone()[0] == 2
    dup = con.execute(
        "SELECT COUNT(*) FROM (SELECT mode, edge_id FROM mm.edges "
        "GROUP BY mode, edge_id HAVING COUNT(*) > 1)").fetchone()[0]
    assert dup == 0                                # (mode, edge_id) is unique


def test_builder_discovers_present_modes_only():
    con = _mm_db()
    con.execute("CREATE TABLE main.notes(x INTEGER)")   # a non-mode schema table must be ignored
    MultimodalBuilder(con).run()
    modes = {r[0] for r in con.execute("SELECT DISTINCT mode FROM mm.edges").fetchall()}
    assert modes == {"walking", "driving"}              # not 'main'/'raw'/'mm'


def test_builder_requires_cost_s():
    con = duckdb.connect()
    con.execute("INSTALL spatial; LOAD spatial;")
    con.execute("CREATE SCHEMA walking; CREATE SCHEMA driving")
    for m in ("walking", "driving"):
        con.execute(f"CREATE TABLE {m}.nodes(node_id BIGINT, geom GEOMETRY)")
        con.execute(f"CREATE TABLE {m}.edges(edge_id BIGINT, source BIGINT, target BIGINT)")  # no cost_s
    with pytest.raises(ValueError, match="cost_s"):
        MultimodalBuilder(con).run()


# ---- MultimodalBuilder: mm.transfers (v1 coarse) ----------------------------------------------

def test_coarse_transfers_at_shared_nodes():
    con = _mm_db()
    MultimodalBuilder(con, transfer_s=60).run()
    rows = con.execute(
        "SELECT node_id, from_mode, to_mode, cost_s, kind FROM mm.transfers "
        "ORDER BY from_mode").fetchall()
    # exactly the one shared junction (20), both directions, park/retrieve kinds
    assert rows == [
        (20, "driving", "walking", 60.0, "retrieve"),
        (20, "walking", "driving", 60.0, "park"),
    ]


def test_transfer_cost_overrides_are_per_direction():
    con = _mm_db()
    MultimodalBuilder(con, transfer_s=60,
                      transfer_costs={"walking->driving": 120, "driving->walking": 15}).run()
    costs = dict(con.execute(
        "SELECT from_mode || '->' || to_mode, cost_s FROM mm.transfers").fetchall())
    assert costs == {"walking->driving": 120.0, "driving->walking": 15.0}


def test_transfers_route_through_walking_only():
    """3-mode db: transfers connect walking<->driving and walking<->cycling, never driving<->cycling."""
    con = _new_db(modes=("walking", "driving", "cycling"))
    for m in ("walking", "driving", "cycling"):
        _add_node(con, m, 20, 1, 0)                 # junction shared by ALL three modes
        _add_edge(con, m, 100 + hash(m) % 50, 20, 20 + len(m), 10)   # a stub edge so the mode is 'present'
    MultimodalBuilder(con).run()
    pairs = {(f, t) for f, t in con.execute(
        "SELECT DISTINCT from_mode, to_mode FROM mm.transfers").fetchall()}
    assert ("driving", "cycling") not in pairs and ("cycling", "driving") not in pairs
    assert ("walking", "driving") in pairs and ("walking", "cycling") in pairs
    assert ("driving", "walking") in pairs and ("cycling", "walking") in pairs


def test_no_walking_leaves_transfers_empty():
    """Nothing to bridge without a pedestrian hub — mm.transfers is created but empty."""
    con = _new_db(modes=("driving", "cycling"))
    for m in ("driving", "cycling"):
        _add_node(con, m, 20, 1, 0)
        _add_edge(con, m, 300 + len(m), 20, 21, 10)
    stats = MultimodalBuilder(con).run()
    assert stats["transfer_count"] == 0
    assert con.execute("SELECT COUNT(*) FROM mm.transfers").fetchone()[0] == 0


# ---- route_multimodal --------------------------------------------------------------------------

def test_route_walk_drive_exists_and_costs():
    """walk 10->20, transfer (park), drive 20->40; time = edges + transfer penalty (exact)."""
    con = _mm_db()
    MultimodalBuilder(con, transfer_s=60).run()
    r = route_multimodal(con, 10, 40, start_mode="walking", end_mode="driving")
    assert r is not None
    assert r["edges"] == [("walking", 101), ("driving", 201)]
    assert [leg["mode"] for leg in r["legs"]] == ["walking", "driving"]
    assert len(r["transfers"]) == 1 and r["transfers"][0]["kind"] == "park"
    assert r["transfers"][0]["node_id"] == 20
    # spec test 4: route weight in seconds == Σ edge cost_s + Σ transfer cost_s
    assert r["time_s"] == 50 + 60 + 10
    assert r["time_s"] == (sum(leg["time_s"] for leg in r["legs"])
                           + sum(t["cost_s"] for t in r["transfers"]))
    # per-leg detail carries geometry (for viz)
    assert r["legs"][0]["path"][0]["geometry"].startswith("LINESTRING")


def test_route_walk_drive_walk_round_trip():
    """A full walk->drive->walk: extend _mm_db with a walking leg off the driving destination."""
    con = _mm_db()
    _add_node(con, "walking", 40, 1, 1)             # node 40 now shared (walk + drive)
    _add_node(con, "driving", 10, 0, 0)             # give driving a copy of 10 so 40->...->10 not needed
    _add_edge(con, "walking", 401, 40, 50, 50, x0=1, y0=1, x1=2, y1=2, hw="footway")
    _add_node(con, "walking", 50, 2, 2)
    MultimodalBuilder(con, transfer_s=60).run()
    r = route_multimodal(con, 10, 50, start_mode="walking", end_mode="walking")
    assert r is not None
    assert [leg["mode"] for leg in r["legs"]] == ["walking", "driving", "walking"]
    assert len(r["transfers"]) == 2                 # park at 20, retrieve at 40
    # 10->20 walk(50) + park(60) + 20->40 drive(10) + retrieve(60) + 40->50 walk(50)
    assert r["time_s"] == 50 + 60 + 10 + 60 + 50


def test_no_transfers_means_no_mode_change():
    """spec test 2: with mm.transfers empty, the layers are disconnected — no cross-mode route."""
    con = _mm_db()
    MultimodalBuilder(con, transfer_s=60).run()
    con.execute("DELETE FROM mm.transfers")
    assert route_multimodal(con, 10, 40, start_mode="walking", end_mode="driving") is None
    # ...but a same-mode route still works
    r = route_multimodal(con, 10, 30, start_mode="walking", end_mode="walking")
    assert r is not None and [leg["mode"] for leg in r["legs"]] == ["walking"]


def _guard_db():
    """Two walking islands bridged only by two SEPARATE drive segments — reaching the far end
    needs walk 1->2, drive 2->3, walk 3->4, drive 4->5, walk 5->6 (TWO vehicular segments)."""
    con = _new_db()
    # walking: 1->2, 3->4, 5->6
    for nid, x in [(1, 0), (2, 1), (3, 2), (4, 3), (5, 4), (6, 5)]:
        _add_node(con, "walking", nid, x, 0)
    _add_edge(con, "walking", 11, 1, 2, 10, hw="footway")
    _add_edge(con, "walking", 12, 3, 4, 10, hw="footway")
    _add_edge(con, "walking", 13, 5, 6, 10, hw="footway")
    # driving: 2->3, 4->5  (the two bridges); nodes 2,3,4,5 shared with walking
    for nid, x in [(2, 1), (3, 2), (4, 3), (5, 4)]:
        _add_node(con, "driving", nid, x, 0)
    _add_edge(con, "driving", 21, 2, 3, 5)
    _add_edge(con, "driving", 22, 4, 5, 5)
    return con


def test_mode_sequence_guard_rejects_two_vehicle_segments():
    con = _guard_db()
    MultimodalBuilder(con, transfer_s=1).run()
    # legal walk* drive* walk* reaching node 4 (ONE drive segment) is fine
    ok = route_multimodal(con, 1, 4, enforce_sequence=True)
    assert ok is not None and [leg["mode"] for leg in ok["legs"]] == ["walking", "driving", "walking"]
    # reaching node 6 needs TWO drive segments -> rejected by the guard...
    assert route_multimodal(con, 1, 6, enforce_sequence=True) is None
    # ...but allowed when the guard is off
    loose = route_multimodal(con, 1, 6, enforce_sequence=False)
    assert loose is not None and sum(leg["mode"] == "driving" for leg in loose["legs"]) == 2


def test_route_unknown_endpoint_raises():
    con = _mm_db()
    MultimodalBuilder(con).run()
    with pytest.raises(ValueError, match="not in"):
        route_multimodal(con, 999999, 40, start_mode="walking", end_mode="driving")


# ---- config + v2 stub --------------------------------------------------------------------------

def test_config_multimodal_block(tmp_path):
    y = tmp_path / "c.yaml"
    y.write_text(
        "name: mm\npbf_path: /x/in.osm.pbf\noutput_path: /tmp/mm\n"
        "modes: [driving, walking]\n"
        "multimodal: {enabled: true, transfer_s: 45, "
        "transfer_costs: {'walking->driving': 90}}\n")
    c = Config.from_yaml(str(y))
    assert c.multimodal.enabled is True
    assert c.multimodal.transfer_s == 45
    assert c.multimodal.transfer_costs == {"walking->driving": 90}
    assert c.multimodal.realistic is False           # default off


def test_config_multimodal_default_disabled():
    assert Config().multimodal.enabled is False


def test_realistic_flag_not_implemented():
    con = _mm_db()
    with pytest.raises(NotImplementedError, match="realistic"):
        MultimodalBuilder(con, realistic=True).run()


# ---- gated integration: build a real 2-mode area + route across it -----------------------------

_REPO = Path(__file__).resolve().parents[1]
_PBF = _REPO / "data" / "maps" / "metro_vancouver.osm.pbf"


@pytest.mark.skipif(not _PBF.exists() or shutil.which("osmium") is None,
                    reason="needs data/maps/metro_vancouver.osm.pbf + osmium")
def test_multimodal_integration_build(tmp_path):
    """End-to-end: clip a tiny downtown-Vancouver bbox, build driving+walking, then the mm graph
    (via the importer), and assert real transfers + a real walk->drive route with a transfer."""
    bbox = tmp_path / "bbox.geojson"                 # ~a few blocks of downtown Vancouver
    bbox.write_text(
        '{"type":"FeatureCollection","features":[{"type":"Feature","properties":{},'
        '"geometry":{"type":"Polygon","coordinates":[[[-123.140,49.270],[-123.110,49.270],'
        '[-123.110,49.290],[-123.140,49.290],[-123.140,49.270]]]}}]}')
    cfg = tmp_path / "c.yaml"
    cfg.write_text(
        f"name: mm_int\noutput_path: {tmp_path}/mm_int.duckdb\n"
        f"source: {{type: pbf, pbf_path: {_PBF}}}\n"
        f"boundary: {{path: {bbox}}}\n"
        "modes: [driving, walking]\n"
        "options: {simplify: true, h3_indexing: false}\n"
        "validation: {enabled: false}\nreport: {enabled: false}\n"
        "multimodal: {enabled: true, transfer_s: 60}\n")

    from duckosm import DuckOSM
    db = DuckOSM(Config.from_yaml(str(cfg))).run()

    con = duckdb.connect(str(db), read_only=True)
    con.execute("INSTALL spatial; LOAD spatial;")
    modes = {r[0] for r in con.execute("SELECT DISTINCT mode FROM mm.edges").fetchall()}
    assert {"driving", "walking"} <= modes
    assert con.execute("SELECT COUNT(*) FROM mm.transfers").fetchone()[0] > 0
    # both transfer directions exist on real data
    dirs = {(f, t) for f, t in con.execute(
        "SELECT DISTINCT from_mode, to_mode FROM mm.transfers").fetchall()}
    assert ("walking", "driving") in dirs and ("driving", "walking") in dirs

    # Force a transfer: a driving edge whose SOURCE has a walking->driving transfer. Routing
    # from that node as a pedestrian and ending in driving MUST cross a transfer.
    pair = con.execute(
        "SELECT e.source, e.target FROM mm.edges e "
        "JOIN mm.transfers t ON t.node_id = e.source AND t.from_mode = 'walking' "
        "WHERE e.mode = 'driving' LIMIT 1").fetchone()
    if pair is None:
        pytest.skip("no walk->drive transfer with an onward driving edge in this tiny clip")
    src, dst = pair
    r = route_multimodal(con, src, dst, start_mode="walking", end_mode="driving")
    assert r is not None
    assert len(r["transfers"]) >= 1 and r["time_s"] > 0
    # cost invariant holds on real data too
    assert abs(r["time_s"] - (sum(leg["time_s"] for leg in r["legs"])
                              + sum(tr["cost_s"] for tr in r["transfers"]))) < 1e-6


# ---- gated visual test: render a Sodermalm multimodal route by mode via mapstyle -------------

_SODER = _REPO / "data" / "db" / "sodermalm_pbf.duckdb"
_MAPSTYLE_SRC = _REPO.parent / "mapstyle" / "src"


def _mapstyle_available():
    import importlib.util
    if importlib.util.find_spec("mapstyle") is not None:
        return True
    return _MAPSTYLE_SRC.exists()


def _has(mod):
    import importlib.util
    return importlib.util.find_spec(mod) is not None


@pytest.mark.skipif(
    not (_SODER.exists() and _has("geopandas") and _mapstyle_available()),
    reason="needs data/db/sodermalm_pbf.duckdb + geopandas + a mapstyle checkout")
def test_sodermalm_mapstyle_route_map(tmp_path):
    """The `scripts/multimodal_route_map.py` visual renders a by-mode Sodermalm route with mapstyle:
    builds mm.* on a scratch COPY, auto-picks a walk->drive->walk trip, and writes the deck.gl
    viewer (index.html + per-mode GeoJSON). Asserts the walking + driving layers were drawn."""
    import importlib.util
    import json
    import shutil

    # load the script by path (it lives in scripts/, not an installed package) — mirrors cli._load_script
    spec = importlib.util.spec_from_file_location(
        "multimodal_route_map", _REPO / "scripts" / "multimodal_route_map.py")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)

    db = tmp_path / "soder.duckdb"                    # copy so we never mutate the real db
    shutil.copy(_SODER, db)
    out_dir = tmp_path / "route_map"
    index = mod.render_multimodal_route(str(db), out_dir=str(out_dir), transfer_cost=60)

    assert Path(index).exists() and Path(index).name == "index.html"
    data = out_dir / "data"
    walking = data / "route_walking.geojson"
    driving = data / "route_driving.geojson"
    assert walking.exists() and driving.exists()      # a real walk->drive->walk trip -> both modes drawn
    for f in (walking, driving):
        assert len(json.loads(f.read_text())["features"]) > 0


if __name__ == "__main__":
    raise SystemExit(pytest.main([__file__, "-q"]))
