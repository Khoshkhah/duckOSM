"""Synthetic turn-restriction overrides: the `turn_restrictions:` rules of the OSM fixes file become rows of
`driving.turn_restrictions` (negative id, -position in the list) and close the turn in `driving.edge_graph`.
See docs/design/turn-restriction-overrides.md and known_osm_issues.md #6. Built with the Rust build (tests/osm_build.py);
the parse rules (skipped rules) are also tested in rust/duckosm-build/src/config.rs (`osm_fixes_file`)."""
import sys
from pathlib import Path

import duckdb

sys.path.insert(0, str(Path(__file__).parent))
from osm_build import build

# two roads meet at node 2: way 100 (1-2) and way 200 (2-3)
NODES = {1: (7.0, 43.0), 2: (7.0, 43.001), 3: (7.001, 43.001)}
WAYS = {100: ([1, 2], {"highway": "residential"}), 200: ([2, 3], {"highway": "residential"})}


def _build(tmp_path, fixes):
    f = tmp_path / "ov.yaml"
    f.write_text(fixes)
    con = duckdb.connect(str(build(tmp_path, NODES, WAYS, args=["--fixes", f])), read_only=True)
    ids = {r: i for r, i in con.execute("SELECT edge_ref, edge_id FROM driving.edges").fetchall()}
    return con, ids


def _turn_exists(con, a, b):
    return con.execute("SELECT count(*) FROM driving.edge_graph WHERE from_edge = ? AND to_edge = ?", [a, b]).fetchone()[0] == 1


def test_synthetic_restriction_injected(tmp_path):
    con, ids = _build(tmp_path, "turn_restrictions:\n  - {from_way: 100, via_node: 2, to_way: 100, restriction: no_u_turn}\n")
    assert con.execute("SELECT restriction_id, restriction_type, via_node, from_edge_id, to_edge_id FROM driving.turn_restrictions").fetchall() \
        == [(-1, "no_u_turn", 2, ids["100#1f"], ids["100#1r"])]   # negative id keeps it clear of OSM ids
    assert not _turn_exists(con, ids["100#1f"], ids["100#1r"])    # the U-turn is closed
    assert _turn_exists(con, ids["100#1f"], ids["200#1f"])        # the other turn is open


def test_default_restriction_and_skips(tmp_path):
    con, ids = _build(tmp_path, """
turn_restrictions:
  - from_way: 100        # no `restriction` -> defaults to no_u_turn
    via_node: 2
    to_way: 100
  - from_way: 100        # unknown restriction -> skipped
    via_node: 2
    to_way: 200
    restriction: banana
  - via_node: 2          # missing from_way -> skipped
    to_way: 200
""")
    assert con.execute("SELECT restriction_id, restriction_type FROM driving.turn_restrictions").fetchall() == [(-1, "no_u_turn")]
    assert _turn_exists(con, ids["100#1f"], ids["200#1f"])


def test_no_section_is_a_noop(tmp_path):
    con, ids = _build(tmp_path, "overrides:\n  - osm_id: 100\n    lanes: 2\n")
    assert con.execute("SELECT count(*) FROM driving.turn_restrictions").fetchone()[0] == 0
    assert _turn_exists(con, ids["100#1f"], ids["100#1r"])
