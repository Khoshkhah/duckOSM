"""OsmOverrides — local corrections applied to the `ways` table before GraphBuilder."""
import duckdb
import pytest

from duckosm.processors.osm_overrides import OsmOverrides


def _ways():
    con = duckdb.connect()
    con.execute("CREATE TABLE ways(osm_id BIGINT, oneway BOOLEAN, lanes_fwd INT, lanes_bwd INT)")
    # rows: Hökens Gata (-> oneway), Trans-Canada WB (-> lanes 3), and an untouched control (999)
    con.execute("INSERT INTO ways VALUES (4392632, FALSE, 1, 1), "
                "(507979055, TRUE, 2, 2), (999, FALSE, 1, 1)")
    return con


def _rules(tmp_path, text):
    p = tmp_path / "ov.yaml"; p.write_text(text); return str(p)


def test_oneway_and_lanes_overrides_apply(tmp_path):
    con = _ways()
    path = _rules(tmp_path, """
overrides:
  - osm_id: 4392632
    oneway: true
  - osm_id: 507979055
    lanes: 3
""")
    n = OsmOverrides(con, path).run()
    assert n == 2
    assert con.execute("SELECT oneway FROM ways WHERE osm_id=4392632").fetchone()[0] is True
    assert con.execute("SELECT lanes_fwd, lanes_bwd FROM ways WHERE osm_id=507979055").fetchone() == (3, 3)
    # control row untouched
    assert con.execute("SELECT oneway, lanes_fwd FROM ways WHERE osm_id=999").fetchone() == (False, 1)


def test_absent_osm_id_is_noop_not_counted(tmp_path):
    con = _ways()
    path = _rules(tmp_path, "overrides:\n  - osm_id: 111222333\n    oneway: true\n")
    assert OsmOverrides(con, path).run() == 0     # rule valid but no such way here


def test_asymmetric_lane_override(tmp_path):
    con = _ways()
    path = _rules(tmp_path, "overrides:\n  - osm_id: 999\n    lanes_forward: 4\n    lanes_backward: 2\n")
    assert OsmOverrides(con, path).run() == 1
    assert con.execute("SELECT lanes_fwd, lanes_bwd FROM ways WHERE osm_id=999").fetchone() == (4, 2)


def test_missing_file_is_noop():
    con = _ways()
    assert OsmOverrides(con, "/no/such/overrides.yaml").run() == 0
    assert OsmOverrides(con, None).run() == 0


def test_tags_rule_retags_the_raw_way(tmp_path):
    """`tags:` is set on raw.ways in the global step, so RoadFilter and global_junctions see the new class."""
    from types import SimpleNamespace
    from duckosm.importer import DuckOSM
    d = DuckOSM.__new__(DuckOSM)
    d.con = duckdb.connect()
    d.con.execute("CREATE SCHEMA raw; CREATE TABLE raw.ways(osm_id BIGINT, tags MAP(VARCHAR, VARCHAR))")
    d.con.execute("INSERT INTO raw.ways VALUES (755033491, MAP {'highway': 'pedestrian', 'name': 'Fredsgatan'}), (999, MAP {'highway': 'pedestrian'})")
    d.config = SimpleNamespace(osm_overrides=_rules(tmp_path, "overrides:\n  - osm_id: 755033491\n    tags: {highway: unclassified}\n"))
    d._raw_way_fixes()
    tags = dict(d.con.execute("SELECT osm_id, tags FROM raw.ways").fetchall())
    assert tags[755033491] == {"highway": "unclassified", "name": "Fredsgatan"}
    assert tags[999] == {"highway": "pedestrian"}


if __name__ == "__main__":
    raise SystemExit(pytest.main([__file__, "-q"]))


def test_exclude_modes_removes_the_way_only_in_that_mode(tmp_path):
    rules = _rules(tmp_path, """
overrides:
  - osm_id: 999
    exclude_modes: [driving]
""")
    con = _ways()
    assert OsmOverrides(con, rules, mode="driving").run() == 1
    assert con.execute("SELECT count(*) FROM ways WHERE osm_id=999").fetchone()[0] == 0
    assert con.execute("SELECT count(*) FROM ways").fetchone()[0] == 2        # the others stay
    con = _ways()
    assert OsmOverrides(con, rules, mode="walking").run() == 0               # kept for walking
    assert con.execute("SELECT count(*) FROM ways WHERE osm_id=999").fetchone()[0] == 1



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
    import pytest
    from duckosm import Config
    pbf = tmp_path / "x.osm.pbf"
    pbf.write_bytes(b"")
    cfg = Config.from_args(pbf_path=str(pbf), output_path=str(tmp_path / "x.duckdb"),
                           osm_overrides=str(tmp_path / "missing.yaml"))
    with pytest.raises(FileNotFoundError, match="OSM fixes file not found"):
        cfg.validate()


def test_turn_lanes_override_sets_the_raw_tag(tmp_path):
    """The painted arrows: turn_lanes_forward becomes the way's turn:lanes:forward tag in raw.ways (every lane reader's source)."""
    con = _ways()
    con.execute("CREATE SCHEMA raw; CREATE TABLE raw.ways(osm_id BIGINT, tags MAP(VARCHAR, VARCHAR), refs BIGINT[])")
    con.execute("INSERT INTO raw.ways VALUES (999, MAP {'turn:lanes:forward': 'through|right', 'lanes': '3'}, [1, 2])")
    path = _rules(tmp_path, "overrides:\n  - osm_id: 999\n    turn_lanes_forward: \"through;left|right\"\n")
    assert OsmOverrides(con, path).run() == 1
    tags = con.execute("SELECT tags FROM raw.ways WHERE osm_id = 999").fetchone()[0]
    assert tags["turn:lanes:forward"] == "through;left|right" and tags["lanes"] == "3"


def _raw_fixer(tmp_path, text):
    from types import SimpleNamespace
    from duckosm.importer import DuckOSM
    d = DuckOSM.__new__(DuckOSM)
    d.con = duckdb.connect()
    d.con.execute("CREATE SCHEMA raw; CREATE TABLE raw.ways(osm_id BIGINT, tags MAP(VARCHAR, VARCHAR), refs BIGINT[]); "
                  "CREATE TABLE raw.nodes(osm_id BIGINT, lat DOUBLE, lon DOUBLE, tags MAP(VARCHAR, VARCHAR))")
    d.con.execute("INSERT INTO raw.ways VALUES (40274443, MAP {'highway': 'cycleway', 'foot': 'yes'}, [1, 2, 3, 4, 5])")
    d.con.execute("INSERT INTO raw.nodes SELECT i, 59.3, 18.0, NULL FROM range(1, 6) t(i)")
    d.config = SimpleNamespace(osm_overrides=_rules(tmp_path, text))
    return d


def test_keep_nodes_and_new_ways(tmp_path):
    """A way cut (keep_nodes) and a new way on OSM nodes and a [lon, lat] node, its tags from the cut way before any fix."""
    d = _raw_fixer(tmp_path, """
overrides:
  - osm_id: 40274443
    keep_nodes: [1, 3]
new_ways:
  - id: 9000003678314
    source: nvdb:3678314
    nodes: [3, [18.1, 59.3], 5]
    tags_from: 40274443
    tags: {highway: unclassified, oneway: "yes"}
""")
    d._raw_way_fixes()
    ways = {w: (t, r) for w, t, r in d.con.execute("SELECT osm_id, tags, refs FROM raw.ways").fetchall()}
    assert ways[40274443] == ({"highway": "cycleway", "foot": "yes"}, [1, 2, 3])
    assert ways[9000003678314] == ({"highway": "unclassified", "foot": "yes", "oneway": "yes"}, [3, 9000003678314002, 5])
    assert d.con.execute("SELECT lon, lat FROM raw.nodes WHERE osm_id = 9000003678314002").fetchone() == (18.1, 59.3)


def test_new_way_is_a_noop_where_its_nodes_are_not(tmp_path):
    """One global rules file: a new way whose OSM nodes are not in this area adds nothing."""
    d = _raw_fixer(tmp_path, "new_ways:\n  - {id: 9000000000001, nodes: [1, [18.1, 59.3], 777]}\n")
    d._raw_way_fixes()
    assert d.con.execute("SELECT count(*) FROM raw.ways WHERE osm_id = 9000000000001").fetchone()[0] == 0
    assert d.con.execute("SELECT count(*) FROM raw.nodes WHERE osm_id > 1000").fetchone()[0] == 0


@pytest.mark.parametrize("ids", [[12345], [9000000000001, 9000000000001]])
def test_new_way_id_below_the_range_or_used_twice_stops_the_build(tmp_path, ids):
    rules = "new_ways:\n" + "".join(f"  - {{id: {i}, nodes: [1, 2]}}\n" for i in ids)
    with pytest.raises(ValueError):
        _raw_fixer(tmp_path, rules)._raw_way_fixes()
