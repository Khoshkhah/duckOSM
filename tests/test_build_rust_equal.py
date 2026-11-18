"""The Rust build (`duckosm --config`) gives the same database as `duckosm build` of the Python package (DuckOSM.run):
the same tables, the same columns and the same rows. Then the Python package can send `duckosm build` and
`duckosm.build()` to the Rust build (docs/reference/python.md)."""
from pathlib import Path

import duckdb
import yaml

from duckosm import Config, DuckOSM, _rust

ROOT = Path(__file__).resolve().parents[1]


def _config(tmp_path, out):
    c = yaml.safe_load((ROOT / "config" / "sample_monaco.yaml").read_text())
    c["output_path"] = str(tmp_path / out)
    c.setdefault("options", {})["sea"] = False      # the sea comes from the network (S3): not in a test
    c["validation"] = {"enabled": False}
    p = tmp_path / f"{out}.yaml"
    p.write_text(yaml.safe_dump(c))
    return p


def _tables(con, db):
    q = f"SELECT table_schema, table_name FROM information_schema.tables WHERE table_catalog = '{db}' ORDER BY 1, 2"
    return {f"{s}.{t}" for s, t in con.execute(q).fetchall()}


def _rows(con, db, table):
    desc = con.execute(f"DESCRIBE {db}.{table}").fetchall()
    cols = [(r[0], r[1]) for r in desc]
    sel = ", ".join(f"ST_AsWKB({r[0]})" if r[1].startswith("GEOMETRY") else f'"{r[0]}"' for r in desc)
    return cols, sorted(map(repr, con.execute(f"SELECT {sel} FROM {db}.{table}").fetchall()))


def test_rust_build_equals_python_build(tmp_path, monkeypatch):
    monkeypatch.chdir(ROOT)                      # the sample config has relative paths
    py = DuckOSM(Config.from_yaml(str(_config(tmp_path, "py")))).run()
    rs_dir = tmp_path / "rs"
    assert _rust.cli(["duckosm", "--config", str(_config(tmp_path, "rs"))]) == 0
    rs = rs_dir / "monaco.duckdb"
    con = duckdb.connect()
    con.execute("LOAD spatial")
    con.execute(f"ATTACH '{py}' AS py (READ_ONLY)")
    con.execute(f"ATTACH '{rs}' AS rs (READ_ONLY)")
    pt, rt = _tables(con, "py"), _tables(con, "rs")
    diffs = [f"only in the Python build: {t}" for t in sorted(pt - rt)]
    diffs += [f"only in the Rust build: {t}" for t in sorted(rt - pt)]
    for t in sorted(pt & rt):
        pc, pr = _rows(con, "py", t)
        rc, rr = _rows(con, "rs", t)
        if pc != rc:
            diffs.append(f"{t}: columns differ: {sorted(set(pc) ^ set(rc))}")
        elif pr != rr:
            diffs.append(f"{t}: {len(pr)} / {len(rr)} rows, {len(set(pr) ^ set(rr))} rows differ")
    assert not diffs, "\n".join(diffs)
