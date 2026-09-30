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
    # every area db stores its time zone (required, not an option)
    # looked up on the network, not at the bbox centre (which for this extract is at sea, in France)
    assert con.execute("SELECT timezone FROM main.visualization_metadata").fetchone()[0] == "Europe/Monaco"
