"""Make the golden file of the Rust `elevation` command (rust/duckosm-tools/tests/golden.rs, `elevation_equals_duckosm`):
the output of the Python `to_elevation` on the Monaco database of that test.

Run the Rust test once (it builds the database and keeps it), then this tool, from the root of the repository:
    cargo test --release -p duckosm-tools --manifest-path rust/Cargo.toml elevation
    python tools/make_golden_elevation.py rust/target/tmp/duckosm-tools-elevation/monaco.duckdb
The golden file is the output of the Python command before the port. Do not change it to make a test pass.
"""
import shutil
import sys
import tempfile
from pathlib import Path

import duckdb

from duckosm.elevation import to_elevation

ROOT = Path(__file__).resolve().parent.parent
DEM = ROOT / "rust/duckosm-build/tests/data/monaco-copernicus-glo30.tif"
CROP = ROOT / "tests/data/golden_tools/monaco_dem_crop.tif"   # covers a part of Monaco: the other nodes get the fill value


def run(db):
    """The two runs of the test: the whole DEM as the reference ground in `ele`, then the crop in `ele_dsm` of two modes."""
    to_elevation(db, dem=str(DEM), product="DTM", vertical_datum="EGM2008", license="Copernicus open (attribution)")
    to_elevation(db, dem=str(CROP), suffix="dsm", nodata_fill=-5.5, modes=["walking", "driving"])


def summary(db):
    """For each table, its height columns, its rows and the MD5 of its rows as text; then the rows of main.elevation_metadata
    without the path, the time and the version. The same SQL as `elevation_summary` in golden.rs: if you change one, change the other."""
    con = duckdb.connect(str(db), read_only=True)
    lines = []
    for m in ("cycling", "driving", "walking"):
        for t, key in (("nodes", "node_id"), ("edges", "edge_id")):
            cols = [r[0] for r in con.execute(
                "SELECT column_name FROM duckdb_columns() WHERE schema_name = ? AND table_name = ? "
                "AND (column_name LIKE 'ele%' OR column_name LIKE 'z\\_%' ESCAPE '\\') ORDER BY column_index", [m, t]).fetchall()]
            n, h = con.execute(f"SELECT count(*), md5(string_agg(t::VARCHAR, chr(10) ORDER BY t::VARCHAR)) "
                               f"FROM (SELECT {', '.join([key, *cols])} FROM {m}.{t}) t").fetchone()
            lines.append(f"{m}.{t}\t{','.join(cols)}\t{n}\t{h}")
    lines += [r[0] for r in con.execute(
        "SELECT t::VARCHAR FROM (SELECT * EXCLUDE (uri, sampled_at, duckosm_version) FROM main.elevation_metadata) t ORDER BY ele_column").fetchall()]
    return "\n".join(lines) + "\n"


def main(db):
    with tempfile.TemporaryDirectory() as tmp:
        copy = Path(tmp) / "monaco.duckdb"
        shutil.copy(db, copy)
        run(copy)
        out = ROOT / "tests/data/golden_tools/monaco_elevation.tsv"
        out.write_text(summary(copy))
        print(out.read_text(), end="")


if __name__ == "__main__":
    main(*sys.argv[1:])
