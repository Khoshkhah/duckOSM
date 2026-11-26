"""
Tests for the Workstream-A pipeline: config schema, the duckdb clip,
and the validation of a build. Run: pytest tests/ -q
"""
from pathlib import Path

import duckdb
import pytest

from cli_run import invoke

from duckosm import DuckOSM, Config

# Clip a sub-area (Sodermalm) out of the country parent (Sweden); both must be built.
PARENT = Path(__file__).resolve().parents[1] / "data" / "db" / "sweden.duckdb"
BOUND = Path(__file__).resolve().parents[1] / "data" / "boundaries" / "sodermalm.geojson"


# ---- config schema -----------------------------------------------------------------
def test_config_duckdb_source(tmp_path):
    y = tmp_path / "c.yaml"
    y.write_text(
        "name: a\noutput_path: /tmp/a.duckdb\n"
        "source: {type: duckdb, source_db: /x/p.duckdb}\n"
        "boundary: {path: /x/b.geojson}\n"
        "clip: {predicate: within, keep_largest_component: false}\n")
    c = Config.from_yaml(str(y))
    assert c.source_type == "duckdb"
    assert c.source_db == "/x/p.duckdb"
    assert c.effective_boundary_path == "/x/b.geojson"
    assert c.clip.predicate == "within"
    assert c.clip.keep_largest_component is False


def test_config_backcompat_flat(tmp_path):
    y = tmp_path / "c.yaml"
    y.write_text(
        "name: b\npbf_path: /x/in.osm.pbf\noutput_path: /tmp/b\n"
        "boundary_path: /x/bnd.geojson\n")
    c = Config.from_yaml(str(y))
    assert c.source_type == "pbf"
    assert c.effective_pbf_path == "/x/in.osm.pbf"
    assert c.effective_boundary_path == "/x/bnd.geojson"


# ---- validation in a build ---------------------------------------------------------
def test_validation_layer_without_structure_warns_not_fails(tmp_path):
    """The warn-only check is flagged in the build output and does not fail the build, even with fail_on_error."""
    pbf = Path(__file__).resolve().parents[1] / "data" / "sample" / "monaco.osm.pbf"
    cfg = tmp_path / "c.yaml"
    cfg.write_text(
        f"name: m\noutput_path: {tmp_path / 'out'}\nsource: {{type: pbf, pbf_path: {pbf}}}\nmodes: [driving]\n"
        "options: {build_features: false, h3_indexing: false, bus_routes: false, levels: false}\n"
        "validation: {enabled: true, fail_on_error: true, warn_layer_without_structure: true}\n")
    r = invoke(["-c", cfg])
    assert r.exit_code == 0, r.output
    assert "validate[driving] layer_without_structure: WARN" in r.output   # Monaco has layer tags with no bridge or tunnel


# ---- duckdb clip integration (skips if the parent isn't built) ---------------------
@pytest.mark.skipif(not (PARENT.exists() and BOUND.exists()), reason="parent db not built")
def test_clip_preserves_edge_ids(tmp_path):
    out = tmp_path / "clip.duckdb"
    cfg = Config.from_args(pbf_path="", output_path=str(out), name="t",
                           boundary_path=str(BOUND), source_db=str(PARENT),
                           build_graph=True, h3_indexing=False)
    DuckOSM(cfg).run()
    con = duckdb.connect()
    con.execute(f"ATTACH '{PARENT}' AS p (READ_ONLY)")
    con.execute(f"ATTACH '{out}' AS c (READ_ONLY)")
    n_clip = con.execute("SELECT count(*) FROM c.driving.edges").fetchone()[0]
    preserved = con.execute(
        "SELECT count(*) FROM c.driving.edges x JOIN p.driving.edges y "
        "ON x.edge_id=y.edge_id AND x.source=y.source AND x.target=y.target").fetchone()[0]
    assert n_clip > 1000 and preserved == n_clip       # every clipped edge_id verbatim in parent
    # fewer edges than the parent (the disconnected fragments were dropped)
    n_parent = con.execute("SELECT count(*) FROM p.driving.edges").fetchone()[0]
    assert n_clip < n_parent
