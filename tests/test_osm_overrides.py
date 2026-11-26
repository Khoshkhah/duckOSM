"""The OSM fixes file (`osm_overrides`): local corrections that the Rust build applies to the OSM data before it makes the network.
Each test writes OSM nodes and ways, a fixes file, builds with `duckosm build --fixes` (tests/osm_build.py) and reads the tables
(`<mode>.edges`, and `raw.*` for the tag fixes). The parse rules are also tested in rust/duckosm-build/src/config.rs
(`osm_fixes_file`, `extra_tag_edits`) and the tag edits in rust/duckosm-core/tests/tag_edits.rs."""
import subprocess
import sys
from pathlib import Path

import duckdb
import pytest

sys.path.insert(0, str(Path(__file__).parent))
from osm_build import build, write

# three roads of about 100 m; 4392632 and 507979055 are two-way, 999 is the control that no rule names
NODES = {i: (7.0 + 0.001 * i, 43.0) for i in range(1, 7)}
WAYS = {w: (refs, {"highway": "residential"}) for w, refs in {4392632: [1, 2], 507979055: [3, 4], 999: [5, 6]}.items()}


def _build(tmp_path, text, nodes=NODES, ways=WAYS, modes=("driving",)):
    f = tmp_path / "ov.yaml"
    f.write_text(text)
    return duckdb.connect(str(build(tmp_path, nodes, ways, modes=modes, args=["--fixes", f])), read_only=True)


def _edges(con, mode="driving"):
    """``{edge_ref: (lanes, oneway)}``: ``<osm id>#1f`` is the forward edge of a way, ``#1r`` its reverse."""
    return {r: v for r, *v in con.execute(f"SELECT edge_ref, lanes, oneway FROM {mode}.edges").fetchall()}


def test_oneway_and_lanes_overrides_apply(tmp_path):
    con = _build(tmp_path, """
overrides:
  - osm_id: 4392632
    oneway: true
  - osm_id: 507979055
    lanes: 3
""")
    e = _edges(con)
    assert "4392632#1f" in e and "4392632#1r" not in e                       # a one-way road has no reverse edge
    assert e["507979055#1f"][0] == e["507979055#1r"][0] == 3
    assert e["999#1f"] == [1, False] and e["999#1r"] == [1, False]           # control row untouched


def test_absent_osm_id_is_noop(tmp_path):
    con = _build(tmp_path, "overrides:\n  - osm_id: 111222333\n    oneway: true\n")   # a valid rule, but no such way here
    assert len(_edges(con)) == 6 and not any(o for *_, o in _edges(con).values())


def test_asymmetric_lane_override(tmp_path):
    con = _build(tmp_path, "overrides:\n  - osm_id: 999\n    lanes_forward: 4\n    lanes_backward: 2\n")
    e = _edges(con)
    assert (e["999#1f"][0], e["999#1r"][0]) == (4, 2)


def test_missing_file_stops_the_build(tmp_path):
    """A named fixes file that does not exist is an error, not a no-op."""
    pbf = write(tmp_path / "t.osm.pbf", NODES, WAYS)
    p = subprocess.run([sys.executable, "-m", "duckosm", "build", "-p", str(pbf), "-o", str(tmp_path / "t.duckdb"), "--no-features",
                        "--fixes", str(tmp_path / "no_such.yaml")], capture_output=True, text=True)
    assert p.returncode != 0


def test_tags_rule_retags_the_raw_way(tmp_path):
    """`tags:` is set on raw.ways in the global step, so the road filter sees the new class: a pedestrian way becomes a road."""
    ways = {755033491: ([1, 2], {"highway": "pedestrian", "name": "Fredsgatan"}), 999: ([3, 4], {"highway": "pedestrian"})}
    con = _build(tmp_path, "overrides:\n  - osm_id: 755033491\n    tags: {highway: unclassified}\n", ways=ways)
    tags = dict(con.execute("SELECT osm_id, tags FROM raw.ways").fetchall())
    assert tags[755033491] == {"highway": "unclassified", "name": "Fredsgatan"}
    assert tags[999] == {"highway": "pedestrian"}
    assert {r[0] for r in con.execute("SELECT osm_id FROM driving.edges").fetchall()} == {755033491}   # only the retagged way is a road


def test_tag_edits_set_then_delete_on_raw_ways_nodes_and_relations(tmp_path):
    """`extra.tag_edits` (the fixes editor of the page) act on raw.* after the `tags` of the rules."""
    nodes = {**NODES, 7: (7.0, 43.001, {})}
    ways = {5: ([1, 2], {"highway": "residential", "roof:shape": "flat"}), 6: ([3, 4], {"highway": "service"})}
    rels = {8: ([("way", 5, "outer")], {"type": "multipolygon", "building": "yes"})}
    f = tmp_path / "ov.yaml"
    f.write_text("""
overrides:
  - osm_id: 5
    tags: {surface: asphalt}
extra:
  tag_edits:
    - {type: way, id: 5, set: {surface: paved, sidewalk: both}, delete: [roof:shape]}
    - {type: node, id: 7, set: {highway: traffic_signals}}
    - {type: relation, id: 8, delete: [building]}
    - {type: way, id: 999, set: {a: b}}
""")
    con = duckdb.connect(str(build(tmp_path, nodes, ways, rels, args=["--fixes", f])), read_only=True)
    tags = {t: dict(con.execute(f"SELECT osm_id, tags FROM raw.{t}").fetchall()) for t in ("ways", "nodes", "relations")}
    assert tags["ways"] == {5: {"highway": "residential", "surface": "paved", "sidewalk": "both"}, 6: {"highway": "service"}}
    assert tags["nodes"][7] == {"highway": "traffic_signals"}
    assert tags["relations"] == {8: {"type": "multipolygon"}}
    assert con.execute("SELECT surface FROM driving.edges WHERE osm_id = 5 LIMIT 1").fetchone()[0] == "paved"   # the edge has it too


def test_exclude_modes_removes_the_way_only_in_that_mode(tmp_path):
    con = _build(tmp_path, "overrides:\n  - osm_id: 999\n    exclude_modes: [driving]\n", modes=("driving", "walking"))
    assert {r[0] for r in con.execute("SELECT osm_id FROM driving.edges").fetchall()} == {4392632, 507979055}   # the others stay
    assert 999 in {r[0] for r in con.execute("SELECT osm_id FROM walking.edges").fetchall()}                    # kept for walking


def test_fixes_file_is_only_used_when_named(tmp_path, monkeypatch):
    """No hidden default: a rules file sitting in the working directory is NOT picked up."""
    from duckosm import Config
    monkeypatch.chdir(tmp_path)
    (tmp_path / "osm_overrides").mkdir()
    (tmp_path / "osm_overrides" / "osm_overrides.yaml").write_text("overrides: []\n")
    assert Config.from_args(pbf_path="x.osm.pbf", output_path="x.duckdb").osm_overrides is None
    cfg = tmp_path / "c.yaml"
    cfg.write_text("name: t\nsource: {pbf_path: x.osm.pbf}\n")
    assert Config.from_yaml(str(cfg)).osm_overrides is None


def test_a_named_fixes_file_must_exist(tmp_path):
    from duckosm import Config
    pbf = tmp_path / "x.osm.pbf"
    pbf.write_bytes(b"")
    cfg = Config.from_args(pbf_path=str(pbf), output_path=str(tmp_path / "x.duckdb"),
                           osm_overrides=str(tmp_path / "missing.yaml"))
    with pytest.raises(FileNotFoundError, match="OSM fixes file not found"):
        cfg.validate()


def test_turn_lanes_override_sets_the_raw_tag(tmp_path):
    """The painted arrows: turn_lanes_forward becomes the way's turn:lanes:forward tag in raw.ways (every lane reader's source)."""
    ways = {999: ([1, 2], {"highway": "residential", "turn:lanes:forward": "through|right", "lanes": "3"})}
    con = _build(tmp_path, "overrides:\n  - osm_id: 999\n    turn_lanes_forward: \"through;left|right\"\n", ways=ways)
    tags = con.execute("SELECT tags FROM raw.ways WHERE osm_id = 999").fetchone()[0]
    assert tags["turn:lanes:forward"] == "through;left|right" and tags["lanes"] == "3"


# a cycleway of five nodes in a row (11..15), 50 m apart
CYCLE = {i: (7.0 + 0.0005 * i, 43.0) for i in range(11, 16)}
CYCLEWAY = {40274443: ([11, 12, 13, 14, 15], {"highway": "cycleway", "foot": "yes"})}


def test_keep_nodes_and_new_ways(tmp_path):
    """A way cut (keep_nodes) and a new way on OSM nodes and a [lon, lat] node, its tags from the cut way before any fix."""
    con = _build(tmp_path, """
overrides:
  - osm_id: 40274443
    keep_nodes: [11, 13]
new_ways:
  - id: 9000003678314
    source: nvdb:3678314
    nodes: [13, [7.003, 43.0], 15]
    tags_from: 40274443
    tags: {highway: unclassified, oneway: "yes"}
""", nodes=CYCLE, ways=CYCLEWAY, modes=("driving", "cycling"))
    ways = {w: (t, r) for w, t, r in con.execute("SELECT osm_id, tags, refs FROM raw.ways").fetchall()}
    assert ways[40274443] == ({"highway": "cycleway", "foot": "yes"}, [11, 12, 13])
    assert ways[9000003678314] == ({"highway": "unclassified", "foot": "yes", "oneway": "yes"}, [13, 9000003678314002, 15])
    assert con.execute("SELECT lon, lat FROM raw.nodes WHERE osm_id = 9000003678314002").fetchone() == (7.003, 43.0)
    assert con.execute("SELECT refs FROM cycling.edges WHERE edge_ref = '40274443#1f'").fetchone()[0] == [11, 12, 13]   # the cut way is shorter
    assert con.execute("SELECT highway, oneway, refs FROM driving.edges WHERE osm_id = 9000003678314").fetchall() \
        == [("unclassified", True, [13, 9000003678314002, 15])]                                                          # the new way is a road


def test_new_way_is_a_noop_where_its_nodes_are_not(tmp_path):
    """One global rules file: a new way whose OSM nodes are not in this area adds nothing."""
    con = _build(tmp_path, "new_ways:\n  - {id: 9000000000001, nodes: [11, [7.1, 43.0], 777], tags: {highway: service}}\n",
                 nodes=CYCLE, ways=CYCLEWAY)
    assert con.execute("SELECT count(*) FROM raw.ways WHERE osm_id = 9000000000001").fetchone()[0] == 0
    assert con.execute("SELECT count(*) FROM raw.nodes WHERE osm_id > 1000").fetchone()[0] == 0


@pytest.mark.parametrize("ids", [[12345], [9000000000001, 9000000000001]])
def test_new_way_id_below_the_range_or_used_twice_stops_the_build(tmp_path, ids):
    f = tmp_path / "ov.yaml"
    f.write_text("new_ways:\n" + "".join(f"  - {{id: {i}, nodes: [11, 12]}}\n" for i in ids))
    pbf = write(tmp_path / "t.osm.pbf", CYCLE, CYCLEWAY)
    p = subprocess.run([sys.executable, "-m", "duckosm", "build", "-p", str(pbf), "-o", str(tmp_path / "t.duckdb"), "--no-features",
                        "--fixes", str(f)], capture_output=True, text=True)
    assert p.returncode != 0 and "new_ways" in p.stderr + p.stdout, p.stderr
