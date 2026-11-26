"""A pip install has no repo: the config template ships inside the package (`duckosm init-config`)."""

from cli_run import invoke
from duckosm import Config


def test_init_config_writes_a_loadable_template(tmp_path):
    out = tmp_path / "area.yaml"
    r = invoke(["init-config", str(out)])
    assert r.exit_code == 0, r.output
    cfg = Config.from_yaml(str(out))
    assert cfg.name and cfg.modes

    r = invoke(["init-config", str(out)])       # no silent overwrite
    assert r.exit_code != 0 and "already exists" in r.output

