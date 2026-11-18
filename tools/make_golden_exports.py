"""Make the golden files of the Rust exports (rust/duckosm-export/tests/golden.rs): the output of the Python exports
on the Monaco database of that test.

Run the Rust test once (it builds the database), then this tool, from the root of the repository:
    cargo test --release -p duckosm-export --manifest-path rust/Cargo.toml
    python tools/make_golden_exports.py rust/target/tmp/duckosm-exports/golden.duckdb tests/data/exports [GMNS_DB]
The exports of a GMNS db read the tables of tests/data/exports/gmns_driving/ (the columns that they read, as Parquet).
GMNS_DB (`duckosm gmns` of the golden db) makes these files again.
Each golden file is the output of the Python export before the port. Do not change a golden file to make a test pass.
"""
import gzip
import shutil
import sys
import tempfile
from pathlib import Path

import duckdb

from duckosm.gis import to_gis
from duckosm.gmns import to_gmns
from duckosm.gis_debug import write_debug
from duckosm.lanelet2 import to_lanelet2
from duckosm.matsim import to_matsim
from duckosm.matsim_lanes import to_matsim_lanes
from duckosm.opendrive import to_opendrive
from duckosm.railml import to_railml
from duckosm.sumo import to_sumo


def summary(src):
    """One line of a layer: its columns, its number of rows and the MD5 of its rows as text. `src`: a FROM item.
    The same SQL as `summary` in golden.rs: if you change one, change the other."""
    con = duckdb.connect()
    con.execute("INSTALL spatial; LOAD spatial")
    cols = con.execute(f"SELECT string_agg(column_name || ':' || column_type, ',') FROM (DESCRIBE SELECT * FROM {src})").fetchone()[0]
    n, h = con.execute(f"SELECT count(*), md5(coalesce(string_agg(t::VARCHAR, chr(10) ORDER BY t::VARCHAR), '')) "
                       f"FROM {src} t").fetchone()
    return f"{cols}\t{n}\t{h}"


# the columns of gmns_driving that the exports read: OpenDRIVE --junctions, MATSim lanes, Lanelet2
GMNS = {"link": "link_id, name, from_node_id, to_node_id, length, capacity, free_speed, lanes, geom",
        "movement": "node_id, ib_link_id, start_ib_lane, end_ib_lane, ob_link_id, mvmt_code, geometry",
        "node": "node_id, ctrl_type",
        "lane": "link_id, allowed_uses, width, geom"}


def gmns_fixture(gmns_db, out):
    con = duckdb.connect(gmns_db, read_only=True)
    con.execute("INSTALL spatial; LOAD spatial")
    (out / "gmns_driving").mkdir(exist_ok=True)
    for t, cols in GMNS.items():
        con.execute(f"COPY (SELECT {cols} FROM gmns_driving.{t}) TO '{out}/gmns_driving/{t}.parquet' (FORMAT parquet, COMPRESSION zstd)")


def gmns_db(fixture, path):
    """A GMNS db from the Parquet files: the same SQL as `gmns_db` in golden.rs."""
    con = duckdb.connect(str(path))
    con.execute("INSTALL spatial; LOAD spatial; CREATE SCHEMA gmns_driving")
    for t in GMNS:
        con.execute(f"CREATE TABLE gmns_driving.{t} AS SELECT * FROM read_parquet('{fixture}/gmns_driving/{t}.parquet')")
    con.close()


def bad_files(con, d):
    """Shapefiles that gis-debug must fail: the same SQL as `bad_files` in golden.rs."""
    Path(d).mkdir()
    con.execute(f"COPY (SELECT edge_id::DOUBLE AS edge_id, highway, geometry FROM driving.edges) TO '{d}/m_edges_driving.shp' "
                f"(FORMAT gdal, DRIVER 'ESRI Shapefile', SRS 'EPSG:4326')")
    con.execute(f"COPY (SELECT CASE WHEN edge_id % 7 = 0 THEN NULL ELSE edge_id END AS edge_id, highway, geometry FROM walking.edges) "
                f"TO '{d}/m_edges_walking.shp' (FORMAT gdal, DRIVER 'ESRI Shapefile', SRS 'EPSG:3857')")


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
        # gis-debug: the GeoPackage, the shapefiles, and shapefiles with a float edge_id, NULLs and another CRS
        bad_files(src, f"{tmp}/bad")
        for x in ("monaco.gpkg", "shp", "bad"):
            write_debug(f"{tmp}/{x}", source_db=db, out=f"{tmp}/{x}.html")
            gz(Path(f"{tmp}/{x}.html"), out / f"gis_debug_{x.split('.')[-1]}.html.gz")
        src.close()
    (out / "monaco_gis_summary.tsv").write_text("\n".join(lines) + "\n")


def columns(db, schema_like="gmns\\_%"):
    """One line for each column of each table of the schemas `schema_like`: its type, the rows and the MD5 of its values as
    text, sorted (NULL is the empty text after a mark). The same SQL as `columns` in golden_gmns.rs: if you change one,
    change the other."""
    con = duckdb.connect(str(db), read_only=True)
    con.execute("INSTALL spatial; LOAD spatial")
    lines = ["table\tcolumn\ttype\trows\tmd5"]
    cols = con.execute("SELECT table_schema || '.' || table_name, column_name, data_type FROM information_schema.columns "
                       f"WHERE table_schema LIKE '{schema_like}' ESCAPE '\\' ORDER BY 1, ordinal_position").fetchall()
    for t, c, typ in cols:
        v = f'ST_AsText("{c}")' if typ.startswith("GEOMETRY") else f'"{c}"::VARCHAR'
        n, h = con.execute(f"SELECT count(*), md5(coalesce(string_agg(coalesce('=' || {v}, '-'), chr(10) "
                           f"ORDER BY coalesce('=' || {v}, '-')), '')) FROM {t}").fetchone()
        lines.append(f"{t}\t{c}\t{typ}\t{n}\t{h}")
    con.close()
    return lines


def gmns(db, out):
    """duckosm gmns on Monaco (its defaults: every mode, lane geometry) -> one line for each column of the GMNS db."""
    with tempfile.TemporaryDirectory() as tmp:
        to_gmns(db, f"{tmp}/monaco_gmns.duckdb")
        (out / "monaco_gmns_columns.tsv").write_text("\n".join(columns(f"{tmp}/monaco_gmns.duckdb")) + "\n")


def gz(src, dst):
    """Keep the large Python output gzip-compressed, with the time 0 in its header (the same bytes on each run)."""
    with open(src, "rb") as f, open(dst, "wb") as o:
        o.write(gzip.compress(f.read(), mtime=0))


def main(db, out, gmns=None):
    out = Path(out)
    out.mkdir(parents=True, exist_ok=True)
    if gmns:
        gmns_fixture(gmns, out)
    with tempfile.TemporaryDirectory() as tmp:
        tmp = Path(tmp)
        g = tmp / "monaco_gmns.duckdb"
        gmns_db(out, g)
        to_opendrive(g, tmp / "monaco_junctions.xodr", junctions=True)
        to_matsim_lanes(g, tmp / "ml")
        to_lanelet2(g, tmp / "monaco.lanelet2.osm")
        gz(tmp / "monaco.lanelet2.osm", out / "monaco.lanelet2.osm.gz")
        to_opendrive(db, tmp / "monaco.xodr")
        for f in ("monaco_junctions.xodr", "monaco.xodr"):
            gz(tmp / f, out / f"{f}.gz")
        for f in ("lanes", "signalSystems", "signalGroups", "signalControl"):
            gz(tmp / "ml" / f"{f}.xml", out / f"matsim_lanes_{f}.xml.gz")
        src = duckdb.connect(db, read_only=True)
        for m in ("driving", "walking"):                 # the plain XML: the CI has no netconvert
            to_sumo(src, tmp / m, mode=m, net_name="monaco", run_netconvert=False)
            for f in ("nod", "edg", "con"):
                gz(tmp / m / f"monaco.{f}.xml", out / f"sumo_{m}.{f}.xml.gz")
        src.close()
    to_matsim(db, out / "monaco_network.xml.gz", mode="driving")
    to_matsim(db, out / "monaco_all_network.xml.gz", mode="all", crs="EPSG:3035")
    to_railml(db, out / "monaco.railml.xml")
    gis(db, out)
    gmns(db, out)
    for p in sorted(x for x in out.iterdir() if x.is_file()):
        n = len(gzip.open(p).read()) if p.suffix == ".gz" else p.stat().st_size
        print(f"{p}: {n} bytes")


if __name__ == "__main__":
    main(*sys.argv[1:])
