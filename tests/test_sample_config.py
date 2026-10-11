"""The published config/ files: the template matches the one the package ships, and the Monaco
sample builds end to end (the only full build CI runs; the data is in data/sample/)."""
from pathlib import Path

import duckdb

from duckosm import Config, DuckOSM

ROOT = Path(__file__).resolve().parents[1]


def test_repo_template_matches_packaged_template():
    packaged = ROOT / "src" / "duckosm" / "templates" / "config.yaml"
    assert (ROOT / "config" / "template.yaml").read_text() == packaged.read_text(), \
        "config/template.yaml drifted: copy src/duckosm/templates/config.yaml over it"


def test_monaco_sample_builds(tmp_path, monkeypatch):
    cfg = Config.from_yaml(str(ROOT / "config" / "sample_monaco.yaml"))
    cfg.source.pbf_path = str(ROOT / cfg.source.pbf_path)       # the sample is relative to the repo root
    cfg.boundary.path = str(ROOT / cfg.boundary.path)
    if cfg.osm_overrides:
        cfg.osm_overrides = str(ROOT / cfg.osm_overrides)     # the sample's fixes file is relative to the repo too
    cfg.output_path = str(tmp_path)
    monkeypatch.chdir(tmp_path)                                   # keep reports etc. out of the repo
    DuckOSM(cfg).run()

    con = duckdb.connect(str(tmp_path / "monaco.duckdb"), read_only=True)
    for mode in ("driving", "walking", "cycling"):
        assert con.execute(f"SELECT count(*) FROM {mode}.edges").fetchone()[0] > 100
    assert con.execute("SELECT count(*) FROM driving.edge_graph").fetchone()[0] > 0
    # the boundary makes the build drop disconnected fragments: one network per mode
    for mode in ("driving", "walking", "cycling"):
        rows = con.execute(f"SELECT source, target FROM {mode}.edges").fetchall()
        parent = {}
        def find(x):
            while parent.setdefault(x, x) != x:
                x = parent[x]
            return x
        for s_, t in rows:
            parent[find(s_)] = find(t)
        assert len({find(s_) for s_, _ in rows}) == 1, mode
    # a crossing is cut where it crosses a road in every mode, even in walking, which doesn't have
    # that road (a secondary without sidewalks): the same pieces, so the same edge_ids (way 586268007)
    ids = {m: {r[0] for r in con.execute(f"SELECT edge_id FROM {m}.edges WHERE osm_id = 586268007").fetchall()}
           for m in ("walking", "cycling")}
    assert len(ids["walking"]) == 4 and ids["walking"] == ids["cycling"]
    # an extract keeps the boundary in `geom`, like a build, so maps can draw its outline
    # (it once wrote `geometry`, and the viz / route-map outline silently disappeared)
    from duckosm.extract import main as extract
    from duckosm.viz import _boundary_geojson
    part = tmp_path / "part.duckdb"
    extract(["--source", str(tmp_path / "monaco.duckdb"), "--db", str(part),
             "--boundary", str(ROOT / "data" / "sample" / "monaco.geojson")])
    assert _boundary_geojson(duckdb.connect(str(part), read_only=True)) is not None
    # a bus lane against a one-way street is in driving, with the private roads: Boulevard des Moulins
    # (way 166009794) is one-way for cars, its reverse a bus lane (docs/design/bus_only_edges.md)
    lane = 1964280132851416298                                   # that reverse; the same id in cycling
    assert con.execute(f"SELECT access FROM driving.private_edges WHERE edge_id = {lane}").fetchone() == ("bus",)
    assert con.execute(f"SELECT count(*) FROM driving.edges WHERE edge_id = {lane}").fetchone()[0] == 0
    assert con.execute(f"SELECT count(*) FROM driving.edge_graph WHERE {lane} IN (from_edge, to_edge) AND uses = 'car'").fetchone()[0] == 0
    assert con.execute(f"SELECT count(*) FROM driving.edge_graph WHERE {lane} IN (from_edge, to_edge) AND uses = 'bus'").fetchone()[0] > 0   # the buses' turns
    assert con.execute(f"SELECT count(*) FROM cycling.edges WHERE edge_id = {lane}").fetchone()[0] == 1
    # `duckosm info` and `way --json`, the commands an agent reads first
    import json
    from click.testing import CliRunner
    from duckosm import __version__
    from duckosm.cli import main
    r = CliRunner().invoke(main, ["info", str(tmp_path / "monaco.duckdb"), "--json"])
    assert r.exit_code == 0, r.output
    d = json.loads(r.output)
    assert [m["mode"] for m in d["modes"]] == ["driving", "walking", "cycling"]
    assert d["modes"][0]["edges"] > 100 and d["modes"][0]["turn_restrictions"] > 0
    assert d["timezone"] == "Europe/Monaco" and d["duckosm_version"] == __version__ and d["built_at"]
    assert d["raw"] and d["boundary"] and len(d["features"]) >= 5
    text = CliRunner().invoke(main, ["info", str(tmp_path / "monaco.duckdb")]).output
    assert "driving" in text and "time zone Europe/Monaco" in text
    r = CliRunner().invoke(main, ["way", str(tmp_path / "monaco.duckdb"), "4230100", "--json"])
    w = json.loads(r.output)
    assert w["raw"]["tags"]["name"] == "Avenue Delphine" and {e["mode"] for e in w["edges"]} >= {"driving"}
    # every area db stores its time zone (required, not an option)
    # looked up on the network, not at the bbox centre (which for this extract is at sea, in France)
    assert con.execute("SELECT timezone FROM main.visualization_metadata").fetchone()[0] == "Europe/Monaco"
