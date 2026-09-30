"""A pip install has no repo: the config template ships inside the package (`duckosm init-config`),
and `duckosm build` names its output after the input in the current folder."""
from click.testing import CliRunner

from duckosm import Config
from duckosm.cli import _stem, main


def test_init_config_writes_a_loadable_template(tmp_path):
    out = tmp_path / "area.yaml"
    r = CliRunner().invoke(main, ["init-config", str(out)])
    assert r.exit_code == 0, r.output
    cfg = Config.from_yaml(str(out))
    assert cfg.name and cfg.modes

    r = CliRunner().invoke(main, ["init-config", str(out)])       # no silent overwrite
    assert r.exit_code != 0 and "already exists" in r.output


def test_output_named_after_input():
    assert _stem("maps/monaco-latest.osm.pbf") == "monaco-latest"
    assert _stem("sodermalm.geojson") == "sodermalm"
    assert _stem("out/area.duckdb") == "area"
