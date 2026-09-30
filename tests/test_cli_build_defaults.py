"""Regression: a CLI-args build (no YAML) must produce the same shape as a config build.

``duckosm build --pbf X --output Y --graph`` goes through ``Config.from_args``, which fills
``Options`` from dataclass defaults instead of a YAML file. It once died in ``extract_restrictions``
("Table e does not have a column named refs") because that path skipped the simplifier, which
writes ``refs``. The simplifier now always runs (the ``simplify`` option was removed).
"""
from pathlib import Path

import duckdb
import pytest

from duckosm import Config, DuckOSM

PBF = Path(__file__).resolve().parents[1] / "pbf" / "sodermalm.complete_ways.osm.pbf"


def test_simplify_is_not_an_option_any_more(tmp_path, caplog):
    """The simplifier always runs: `simplify` is gone, and an old config naming it only warns."""
    from dataclasses import fields
    from duckosm.config import Options
    assert "simplify" not in {f.name for f in fields(Options)}
    y = tmp_path / "c.yaml"
    y.write_text("name: a\noptions:\n  simplify: false\n")
    with caplog.at_level("WARNING", logger="duckosm"):
        Config.from_yaml(str(y))
    assert "simplify" in caplog.text


@pytest.mark.skipif(not PBF.exists(), reason="needs pbf/sodermalm.complete_ways.osm.pbf")
def test_cli_args_build_extracts_restrictions(tmp_path):
    """End-to-end on the CLI path: the build completes and turn restrictions land."""
    out = tmp_path / "cli.duckdb"
    cfg = Config.from_args(pbf_path=str(PBF), output_path=str(out), name="cli",
                           modes=["driving"], build_graph=True, h3_indexing=False)
    DuckOSM(cfg).run()

    con = duckdb.connect(str(out), read_only=True)
    assert con.execute("SELECT count(*) FROM driving.edges").fetchone()[0] > 1000
    assert con.execute("SELECT count(*) FROM driving.turn_restrictions").fetchone()[0] > 0
    # refs is what restrictions needs; simplify is what writes it.
    assert con.execute(
        "SELECT count(*) FROM driving.edges WHERE refs IS NULL OR len(refs) < 2").fetchone()[0] == 0
