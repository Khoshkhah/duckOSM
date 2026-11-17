"""Make the golden files of the Rust exports (rust/duckosm-export/tests/golden.rs): the output of the Python exports
on the Monaco database of that test.

Run the Rust test once (it builds the database), then this tool, from the root of the repository:
    cargo test --release -p duckosm-export --manifest-path rust/Cargo.toml
    python tools/make_golden_exports.py rust/target/tmp/duckosm-exports/golden.duckdb tests/data/exports
Each golden file is the output of the Python export before the port. Do not change a golden file to make a test pass.
"""
import gzip
import shutil
import sys
import tempfile
from pathlib import Path

import duckdb

from duckosm.gis import to_gis
from duckosm.matsim import to_matsim
from duckosm.railml import to_railml


def summary(src):
    """One line of a layer: its columns, its number of rows and the MD5 of its rows as text. `src`: a FROM item.
    The same SQL as `summary` in golden.rs: if you change one, change the other."""
    con = duckdb.connect()
    con.execute("INSTALL spatial; LOAD spatial")
    cols = con.execute(f"SELECT string_agg(column_name || ':' || column_type, ',') FROM (DESCRIBE SELECT * FROM {src})").fetchone()[0]
    n, h = con.execute(f"SELECT count(*), md5(coalesce(string_agg(t::VARCHAR, chr(10) ORDER BY t::VARCHAR), '')) "
                       f"FROM {src} t").fetchone()
    return f"{cols}\t{n}\t{h}"


def gis(db, out):
    """export-gis on Monaco: GeoParquet, GeoPackage (one file, with ogr2ogr) and shapefile -> a summary of each layer."""
    lines = ["export\tlayer\tcolumns\trows\tmd5"]
    with tempfile.TemporaryDirectory() as tmp:
        src = duckdb.connect(db, read_only=True)
        res = to_gis(src, f"{tmp}/gp", fmt="geoparquet")
        for lyr in res["layers"]:
            lines.append(f"geoparquet\t{lyr}\t" + summary(f"read_parquet('{tmp}/gp/{lyr}.parquet')"))
        shutil.copy(f"{tmp}/gp/layers.json", out / "monaco_geoparquet_layers.json")
        res = to_gis(src, f"{tmp}/monaco.gpkg", fmt="gpkg", name="monaco")
        assert res["path"], "the golden GeoPackage needs ogr2ogr"
        for lyr in res["layers"]:
            lines.append(f"gpkg\t{lyr}\t" + summary(f"ST_Read('{tmp}/monaco.gpkg', layer = '{lyr}')"))
        res = to_gis(src, f"{tmp}/shp", fmt="shp", modes=["driving"], name="monaco")
        for lyr in res["layers"]:
            lines.append(f"shp\t{lyr}\t" + summary(f"ST_Read('{tmp}/shp/monaco_{lyr}.shp')"))
            lines.append(f"shp\t{lyr}.prj\t" + Path(f"{tmp}/shp/monaco_{lyr}.prj").read_text().strip() + "\t\t")
        src.close()
    (out / "monaco_gis_summary.tsv").write_text("\n".join(lines) + "\n")


def main(db, out):
    out = Path(out)
    out.mkdir(parents=True, exist_ok=True)
    to_matsim(db, out / "monaco_network.xml.gz", mode="driving")
    to_matsim(db, out / "monaco_all_network.xml.gz", mode="all", crs="EPSG:3035")
    to_railml(db, out / "monaco.railml.xml")
    gis(db, out)
    for p in sorted(out.iterdir()):
        n = len(gzip.open(p).read()) if p.suffix == ".gz" else p.stat().st_size
        print(f"{p}: {n} bytes")


if __name__ == "__main__":
    main(*sys.argv[1:])
