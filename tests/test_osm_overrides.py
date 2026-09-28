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

