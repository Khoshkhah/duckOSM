"""Regression: a CLI-args build (no YAML) must produce the same shape as a config build.

``duckosm build --pbf X --output Y --graph`` goes through ``Config.from_args``, which fills
``Options`` from dataclass defaults instead of a YAML file. When ``simplify`` defaulted to False
there, the run died in ``extract_restrictions`` ("Table e does not have a column named refs"):
``refs`` is only written by the simplifier, and every shipped config sets ``simplify: true``, so
the CLI path was the one route that hit it. The unsimplified ``edges`` table is not a supported
product anyway — its geometry is a straight LINESTRING between way endpoints — so the default is
aligned with the configs rather than teaching restrictions to cope without ``refs``.
"""
from pathlib import Path

import duckdb
import pytest

from duckosm import Config, DuckOSM

PBF = Path(__file__).resolve().parents[1] / "pbf" / "sodermalm.complete_ways.osm.pbf"


def test_from_args_defaults_simplify_on():
    """The CLI path must default to the same simplify setting every YAML config ships."""
    cfg = Config.from_args(pbf_path="/x/in.osm.pbf", output_path="/tmp/x.duckdb")
    assert cfg.options.simplify is True
    assert cfg.options.extract_restrictions is True   # the pair that used to conflict


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
