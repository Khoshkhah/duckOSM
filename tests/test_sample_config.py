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
    cfg.output_path = str(tmp_path)
    monkeypatch.chdir(tmp_path)                                   # keep reports etc. out of the repo
    DuckOSM(cfg).run()

    con = duckdb.connect(str(tmp_path / "monaco.duckdb"), read_only=True)
    for mode in ("driving", "walking", "cycling"):
        assert con.execute(f"SELECT count(*) FROM {mode}.edges").fetchone()[0] > 100
    assert con.execute("SELECT count(*) FROM driving.edge_graph").fetchone()[0] > 0
