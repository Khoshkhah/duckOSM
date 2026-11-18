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


# the time of the build, and where it ran: not the same in two builds
VOLATILE = {"created", "built_at", "area", "build_dir", "build_config", "roadstyle_version"}


def _rows(con, db, table):
    desc = [r for r in con.execute(f"DESCRIBE {db}.{table}").fetchall() if r[0] not in VOLATILE]
    cols = [(r[0], r[1]) for r in desc]
    sel = ", ".join(f"ST_AsWKB({r[0]})" if r[1].startswith("GEOMETRY") else f'"{r[0]}"' for r in desc)
    return cols, sorted(map(repr, con.execute(f"SELECT {sel} FROM {db}.{table}").fetchall()))


def test_rust_build_equals_python_build(tmp_path, monkeypatch):
    monkeypatch.chdir(ROOT)                      # the sample config has relative paths
    py = DuckOSM(Config.from_yaml(str(_config(tmp_path, "py")))).run()
    rs_dir = tmp_path / "rs"
    assert _rust.cli(["duckosm", "build", "-c", str(_config(tmp_path, "rs"))]) == 0
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
            # one row that differs, by the first column (the id): the values of the columns that differ
            key = pc[0][0]
            skip = VOLATILE & {d[0] for d in con.execute(f"DESCRIBE py.{t}").fetchall()}
            ex = f" EXCLUDE ({', '.join(skip)})" if skip else ""
            q = (f'SELECT * FROM {{}}.{t} WHERE "{key}" IN (SELECT "{key}" FROM (SELECT *{ex} FROM py.{t} EXCEPT SELECT *{ex} FROM rs.{t}) '
                 'LIMIT 1)')
            try:
                cols = [d[0] for d in con.execute(f"DESCRIBE py.{t}").fetchall()]
                a, b = con.execute(q.format("py")).fetchone(), con.execute(q.format("rs")).fetchone()
                for c, x, y in zip(cols, a or [], b or []):
                    if c not in VOLATILE and x != y:
                        diffs.append(f"    {key}={a[0]} {c}: py {str(x)[:1500]} | rs {str(y)[:1500]}")
            except Exception as e:                     # noqa: BLE001 (only the message of the test)
                diffs.append(f"    (no example row: {e})")
    assert not diffs, "\n".join(diffs)
