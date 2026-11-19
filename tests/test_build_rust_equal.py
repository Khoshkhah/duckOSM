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


# the time of the build, and where it ran: not the same in two builds. The heads of edge_levels: roadstyle's Rust level editor makes the
# two heads of a road shorter than 10 m shorter (newer than the roadstyle of the Python build), so only the numbers are compared.
VOLATILE = {"created", "built_at", "area", "build_dir", "build_config", "roadstyle_version", "head_start_m", "head_end_m"}


def _col(name, typ):
    """A column as compared: the geometry as WKB; a map with its entries in key order (the Rust build writes the tags that the
    network reads first, the Python build in the order of the file: the same keys and values)."""
    if typ.startswith("GEOMETRY"):
        return f"ST_AsWKB({name})"
    if typ.startswith("MAP"):
        return f'list_sort(map_entries("{name}"))'
    if name == "boundary_geojson":               # Rust keeps the boundary file's GeoJSON, Python writes ST_AsGeoJSON of it: the same shape
        return f'ST_AsWKB(ST_Normalize(ST_GeomFromGeoJSON("{name}"::VARCHAR)))'
    return f'"{name}"'


def _rows(con, db, table):
    desc = [r for r in con.execute(f"DESCRIBE {db}.{table}").fetchall() if r[0] not in VOLATILE]
    cols = [(r[0], r[1]) for r in desc]
    sel = ", ".join(_col(r[0], r[1]) for r in desc)
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
        if not pr and not rr:                    # an empty table (level_edits): Python's types come from an empty DataFrame, so only the names
            pc, rc = [c for c, _ in pc], [c for c, _ in rc]
        if pc != rc:
            diffs.append(f"{t}: columns differ: {sorted(set(pc) ^ set(rc))}")
        elif pr != rr:
            diffs.append(f"{t}: {len(pr)} / {len(rr)} rows, {len(set(pr) ^ set(rr))} rows differ")
            # one row that differs, by the first column (the id): the values of the columns that differ
            key = pc[0][0]
            skip = VOLATILE & {d[0] for d in con.execute(f"DESCRIBE py.{t}").fetchall()}
            ex = f" EXCLUDE ({', '.join(skip)})" if skip else ""
            try:
                k = con.execute(f'SELECT "{key}" FROM (SELECT *{ex} FROM py.{t} EXCEPT SELECT *{ex} FROM rs.{t}) ORDER BY 1 LIMIT 1').fetchone()[0]
                cols = [d[0] for d in con.execute(f"DESCRIBE py.{t}").fetchall()]
                a, b = (con.execute(f'SELECT * FROM {db}.{t} WHERE "{key}" = ? LIMIT 1', [k]).fetchone() for db in ("py", "rs"))
                for c, x, y in zip(cols, a or [], b or [None] * len(cols)):
                    if c in VOLATILE or repr(x) == repr(y):
                        continue
                    if x == y:                         # equal values in another order (a map): the order differs
                        diffs.append(f"    {key}={k} {c}: the same values in another order: py {repr(x)[:400]} | rs {repr(y)[:400]}")
                        continue
                    sx, sy = str(x), str(y)
                    i = next((i for i, (p, q) in enumerate(zip(sx, sy)) if p != q), min(len(sx), len(sy)))
                    diffs.append(f"    {key}={k} {c} (lengths {len(sx)} / {len(sy)}, first difference at {i}): "
                                 f"py ...{sx[max(0, i - 80):i + 200]} | rs ...{sy[max(0, i - 80):i + 200]}")
            except Exception as e:                     # noqa: BLE001 (only the message of the test)
                diffs.append(f"    (no example row: {e})")
    assert not diffs, "\n".join(diffs)
