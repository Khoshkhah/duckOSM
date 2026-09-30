"""Synthetic turn-restriction overrides — RestrictionProcessor injects `turn_restrictions:` rules from
the OSM-overrides file into `restrictions_pivoted` (then the normal mapping turns them into edge-level
restrictions). See docs/design/turn-restriction-overrides.md and known_osm_issues.md #6."""
import duckdb
import pytest

from duckosm.processors.restrictions import RestrictionProcessor

_PIVOTED = ("CREATE TABLE restrictions_pivoted(restriction_id BIGINT, restriction_type VARCHAR, "
            "from_way BIGINT, via_node BIGINT, to_way BIGINT)")


def _con():
    con = duckdb.connect()
    con.execute(_PIVOTED)
    return con


def _rules(tmp_path, text):
    p = tmp_path / "ov.yaml"; p.write_text(text); return str(p)


def test_synthetic_restriction_injected(tmp_path):
    con = _con()
    path = _rules(tmp_path, """
turn_restrictions:
  - from_way: 1307524008
    via_node: 330045016
    to_way: 997402723
    restriction: no_u_turn
""")
    assert RestrictionProcessor(con, path)._inject_overrides() == 1
    row = con.execute("SELECT restriction_type, from_way, via_node, to_way "
                      "FROM restrictions_pivoted WHERE restriction_id < 0").fetchone()
    assert row == ("no_u_turn", 1307524008, 330045016, 997402723)   # negative id keeps it clear of OSM ids


def test_default_restriction_and_skips(tmp_path):
    con = _con()
    path = _rules(tmp_path, """
turn_restrictions:
  - from_way: 1        # no `restriction` -> defaults to no_u_turn
    via_node: 2
    to_way: 3
  - from_way: 4        # unknown restriction -> skipped
    via_node: 5
    to_way: 6
    restriction: banana
  - via_node: 9        # missing from_way -> skipped
    to_way: 8
""")
    assert RestrictionProcessor(con, path)._inject_overrides() == 1
    assert con.execute("SELECT restriction_type FROM restrictions_pivoted").fetchone()[0] == "no_u_turn"


def test_no_section_and_missing_file_are_noops(tmp_path):
    con = _con()
    assert RestrictionProcessor(con, _rules(tmp_path, "overrides:\n  - osm_id: 1\n    oneway: true\n"))._inject_overrides() == 0
    assert RestrictionProcessor(con, "/no/such/file.yaml")._inject_overrides() == 0
    assert RestrictionProcessor(con, None)._inject_overrides() == 0


if __name__ == "__main__":
    raise SystemExit(pytest.main([__file__, "-q"]))
