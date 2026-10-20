"""The restricted edges (private_edges: bus, private, pedestrian, delivery) have turn rows in edge_graph, labelled in `uses`;
every reader takes the mode's own rows by default and has a `uses` option (docs/design/restricted_edge_graph_rows.md)."""
import os
from pathlib import Path

import duckdb
import pytest

from duckosm import Config, DuckOSM
from duckosm.processors.edge_graph import RESTRICTED_USES, check_uses, routed_graph

ROOT = Path(__file__).resolve().parents[1]


@pytest.fixture(scope="module")
def monaco_path(tmp_path_factory):
    out = tmp_path_factory.mktemp("monaco_uses")
    cfg = Config.from_yaml(str(ROOT / "config" / "sample_monaco.yaml"))
    cfg.source.pbf_path = str(ROOT / (cfg.source.pbf_path or ""))
    cfg.boundary.path = str(ROOT / (cfg.boundary.path or ""))
    if cfg.osm_overrides:
        cfg.osm_overrides = str(ROOT / cfg.osm_overrides)
    cfg.output_path = str(out)
    cwd = Path.cwd()
    os.chdir(out)                                  # keep reports etc. out of the repo
    try:
        DuckOSM(cfg).run()
    finally:
        os.chdir(cwd)
    return out / "monaco.duckdb"


@pytest.fixture(scope="module")
def monaco(monaco_path):
    return duckdb.connect(str(monaco_path), read_only=True)


def test_every_row_is_labelled_by_what_it_touches(monaco):
    rows = monaco.execute("""
        WITH r AS (SELECT edge_id, access FROM driving.private_edges)
        SELECT g.uses,
               g.from_edge IN (SELECT edge_id FROM driving.edges) AS from_car, g.to_edge IN (SELECT edge_id FROM driving.edges) AS to_car,
               a.access AS from_acc, b.access AS to_acc
        FROM driving.edge_graph g LEFT JOIN r a ON a.edge_id = g.from_edge LEFT JOIN r b ON b.edge_id = g.to_edge""").fetchall()
    assert {r[0] for r in rows} <= {"car", *RESTRICTED_USES}
    for uses, from_car, to_car, from_acc, to_acc in rows:
        if uses == "car":
            assert from_car and to_car                                  # two usable edges
        else:
            assert (from_car or from_acc) and (to_car or to_acc)       # both edges exist
            assert uses in (from_acc, to_acc)                          # the label of its restricted edge
            assert not (from_acc and to_acc and from_acc != to_acc)    # no row between two different labels
    assert {r[0] for r in rows} >= {"car", "bus", "pedestrian", "private"}   # the sample has them all (Monaco)


def test_the_default_is_the_modes_own_rows(monaco):
    own = monaco.execute(f"SELECT count(*) FROM {routed_graph(monaco, 'driving')}").fetchone()[0]
    assert own == monaco.execute("SELECT count(*) FROM driving.edge_graph WHERE uses = 'car'").fetchone()[0]
    both = monaco.execute(f"SELECT count(*) FROM {routed_graph(monaco, 'driving', 'bus,pedestrian')}").fetchone()[0]
    assert both == monaco.execute("SELECT count(*) FROM driving.edge_graph WHERE uses IN ('car', 'bus', 'pedestrian')").fetchone()[0] > own
    for mode, own_uses in (("walking", "walk"), ("cycling", "bike")):       # the same rule in every mode
        n = monaco.execute(f"SELECT count(*) FROM {routed_graph(monaco, mode)}").fetchone()[0]
        assert n == monaco.execute(f"SELECT count(*) FROM {mode}.edge_graph WHERE uses = '{own_uses}'").fetchone()[0]


def test_uses_is_checked():
    assert check_uses(None) == () and check_uses("") == ()
    assert check_uses("bus, pedestrian,bus") == ("bus", "pedestrian") and check_uses(["private"]) == ("private",)
    with pytest.raises(ValueError, match="choose from bus, private, pedestrian, delivery"):
        check_uses("flying")


def test_networkx_has_the_extra_turns_only_when_asked(monaco):
    pytest.importorskip("networkx")
    from duckosm.routing import to_networkx
    g0 = to_networkx(monaco, "driving")
    g1 = to_networkx(monaco, "driving", uses="pedestrian,private")
    assert g1.number_of_edges() > g0.number_of_edges() and g1.number_of_nodes() > g0.number_of_nodes()
    assert not any("access" in d for _, d in g0.nodes(data=True))
    assert {d["access"] for _, d in g1.nodes(data=True) if "access" in d} == {"pedestrian", "private"}
    assert all(w is not None for *_, w in g1.edges(data="weight"))
    assert to_networkx(monaco, "driving", weight="length", uses="bus").number_of_edges() >= g0.number_of_edges()


def test_gmns_movements_do_not_take_the_new_labels(monaco_path, tmp_path):
    """GMNS reads the mode's own rows and the bus rows only: no movement has a restricted label as `allowed_uses`."""
    pytest.importorskip("pandas")
    from duckosm.gmns import to_gmns
    out = tmp_path / "gmns.duckdb"
    to_gmns(str(monaco_path), str(out), modes=["driving"])
    con = duckdb.connect(str(out))
    uses = {r[0] for r in con.execute("SELECT DISTINCT allowed_uses FROM gmns_driving.movement").fetchall()}
    assert uses and not uses & {"private", "pedestrian", "delivery"}, uses
    n = duckdb.connect(str(monaco_path), read_only=True).execute(
        "SELECT count(*) FROM driving.edge_graph WHERE uses IN ('private', 'pedestrian', 'delivery')").fetchone()[0]
    assert n > 0                                                # the rows the filter keeps out are there
