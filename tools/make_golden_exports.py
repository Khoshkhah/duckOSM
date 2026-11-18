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


# the columns that gmns-viz reads (driving, with --meso --micro)
VIEWER = {"gmns_driving.lane": "link_id, lane_num, allowed_uses, turn, geom",
          "gmns_driving.node": "node_id, x_coord, y_coord",
          "gmns_driving.link": "link_id, to_node_id, lanes, length",
          "gmns_driving.movement": "node_id",
          "meso_driving.meso_link": "link_id, macro_link_id, lanes, facility_type, mvmt_txt_id, start_ib_lane, end_ib_lane, meso_type, geom",
          "meso_driving.meso_node": "geom",
          "micro_driving.micro_link": "cell_type, mvmt_txt_id, geom"}


def viewer(gmns_db, out, tmp):
    """The viewer fixture (tests/data/exports/gmns_viewer/: the columns that gmns-viz reads, as Parquet, from `gmns_db`) and
    monaco_gmns_viewer.tsv: the MD5 and the size of the page of Python's write_viewer on a db made from these files (the same
    SQL as `viewer_db` in golden.rs)."""
    import hashlib
    from duckosm.gmns_viewer import write_viewer
    d = out / "gmns_viewer"
    d.mkdir(exist_ok=True)
    con = duckdb.connect(str(gmns_db), read_only=True)
    con.execute("INSTALL spatial; LOAD spatial")
    for t, cols in VIEWER.items():
        con.execute(f"COPY (SELECT {cols} FROM {t}) TO '{d}/{t}.parquet' (FORMAT parquet, COMPRESSION zstd)")
    con.close()
    db = Path(tmp) / "viewer" / "monaco_gmns.duckdb"
    db.parent.mkdir()
    con = duckdb.connect(str(db))
    con.execute("INSTALL spatial; LOAD spatial; CREATE SCHEMA gmns_driving; CREATE SCHEMA meso_driving; CREATE SCHEMA micro_driving")
    for t in VIEWER:
        con.execute(f"CREATE TABLE {t} AS SELECT * FROM read_parquet('{d}/{t}.parquet')")
    con.close()
    write_viewer(db, Path(tmp) / "viewer" / "v.html")
    page = (Path(tmp) / "viewer" / "v.html").read_bytes()
    (out / "monaco_gmns_viewer.tsv").write_text(f"driving\t{hashlib.md5(page).hexdigest()}\t{len(page)}\n")


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


def gtfs_feed(db, path):
    """A small GTFS feed of Monaco for `--gtfs` (the same bytes on each run): each OpenStreetMap bus stop 2 m north of its node,
    served by a bus (route_type 3, or 700 for every second stop) or by no trip (every tenth stop); the railway stops (a train,
    route_type 2), the station entrances (location_type 2), the station (1) and one stop far from Monaco. Deflated."""
    import zipfile
    con = duckdb.connect(str(db), read_only=True)
    q = lambda w: con.execute(f"SELECT osm_id, tags['name'], lat, lon FROM raw.nodes WHERE {w} ORDER BY osm_id").fetchall()  # noqa: E731
    stops, service = [], {}
    for i, (osm_id, name, lat, lon) in enumerate(q("tags['highway'] = 'bus_stop'")):
        stops.append((f"B{osm_id}", name or "", round(lat + 0.00002, 7), lon, 0))
        if i % 10:
            service[f"B{osm_id}"] = 700 if i % 2 else 3
    for osm_id, name, lat, lon in q("tags['railway'] = 'stop'"):
        stops.append((f"R{osm_id}", name or "", lat, lon, 0))
        service[f"R{osm_id}"] = 2
    for osm_id, name, lat, lon in q("tags['railway'] = 'train_station_entrance'"):
        stops.append((f"E{osm_id}", name or "", lat, lon, 2))
        service[f"E{osm_id}"] = 2
    for osm_id, name, lat, lon in q("tags['railway'] = 'station'"):
        stops.append((f"S{osm_id}", name or "", lat, lon, 1))
    stops.append(("FAR", "Far away", 43.9, 7.5, 0))
    service["FAR"] = 3
    con.close()
    types = sorted(set(service.values()))
    files = {"stops.txt": [["stop_id", "stop_name", "stop_lat", "stop_lon", "location_type"], *stops],
             "routes.txt": [["route_id", "route_type"], *[[f"r{t}", t] for t in types]],
             "trips.txt": [["trip_id", "route_id"], *[[f"t{t}", f"r{t}"] for t in types]],
             "stop_times.txt": [["trip_id", "stop_id"], *[[f"t{t}", sid] for sid, t in service.items()]]}
    with zipfile.ZipFile(path, "w") as z:
        for n, rows in files.items():
            text = "".join(",".join(f'"{v}"' if "," in str(v) else str(v) for v in r) + "\n" for r in rows)
            z.writestr(zipfile.ZipInfo(n, (1980, 1, 1, 0, 0, 0)), text, compress_type=zipfile.ZIP_DEFLATED)


def gmns(db, out):
    """duckosm gmns on Monaco (every mode, lane geometry, with --combined and --to-csv) -> one line for each column of the GMNS
    db, the MD5 of each CSV file, the rows of gmns_check (check_gmns, graph_report), one line for each column of the meso
    and micro networks, one line for each column of `location` with the stops of a GTFS feed, and one line for each column
    of gmns_walking with --walk-frame, and the gmns-viz fixture and page (`viewer`)."""
    import hashlib
    from duckosm.gmns import to_meso, to_micro
    from duckosm.gmns_check import check_gmns, graph_report
    with tempfile.TemporaryDirectory() as tmp:
        to_gmns(db, f"{tmp}/monaco_gmns.duckdb", combined=True, to_csv=f"{tmp}/csv")
        (out / "monaco_gmns_columns.tsv").write_text("\n".join(columns(f"{tmp}/monaco_gmns.duckdb")) + "\n")
        csv = sorted(Path(f"{tmp}/csv").rglob("*.csv"))

        def md5(p):                       # the header and the sorted rows: the order of some tables is that of a parallel query
            head, *rows = p.read_text().split("\n")
            return hashlib.md5("\n".join([head, *sorted(r for r in rows if r)]).encode()).hexdigest()
        (out / "monaco_gmns_csv.tsv").write_text("".join(f"{p.relative_to(f'{tmp}/csv')}\t{md5(p)}\n" for p in csv))
        rows = [f"{s}\t{n}\t{bad}\t{total}" for s, n, bad, total in check_gmns(f"{tmp}/monaco_gmns.duckdb")]
        rows += [f"graph\t{s}\t{n}\t{links}\t{big}\t{parts}\t{dead}" for s, n, links, big, _, parts, dead in graph_report(f"{tmp}/monaco_gmns.duckdb")]
        (out / "monaco_gmns_check.tsv").write_text("\n".join(rows) + "\n")
        # meso and micro (`--meso --micro`) of driving and cycling
        to_meso(f"{tmp}/monaco_gmns.duckdb", modes=["driving", "cycling"])
        to_micro(f"{tmp}/monaco_gmns.duckdb", modes=["driving", "cycling"])
        lines = columns(f"{tmp}/monaco_gmns.duckdb", "meso\\_%") + columns(f"{tmp}/monaco_gmns.duckdb", "micro\\_%")[1:]
        (out / "monaco_gmns_meso_columns.tsv").write_text("\n".join(lines) + "\n")
        viewer(f"{tmp}/monaco_gmns.duckdb", out, tmp)      # gmns-viz
        # --gtfs: the stops of a feed as location rows
        gtfs_feed(db, out / "monaco_gtfs.zip")
        to_gmns(db, f"{tmp}/gtfs.duckdb", gtfs=[str(out / "monaco_gtfs.zip")])
        lines = [x for x in columns(f"{tmp}/gtfs.duckdb") if x.startswith("table\t") or ".location\t" in x]
        (out / "monaco_gmns_gtfs_location.tsv").write_text("\n".join(lines) + "\n")
        # --walk-frame: the columns of gmns_walking (its lane lines are checked by their properties)
        to_gmns(db, f"{tmp}/wf/monaco_gmns.duckdb", modes=["driving", "walking"], walk_frame=True)
        (out / "monaco_gmns_walk_frame_columns.tsv").write_text("\n".join(columns(f"{tmp}/wf/monaco_gmns.duckdb", "gmns\\_walking")) + "\n")


def lane_diff_lines(rows, n):
    """The lines of monaco_sumo_lane_diff.tsv: the number compared, then one line for each difference."""
    t = lambda x: ",".join(map(str, x))  # noqa: E731
    return [f"compared\t{n}"] + [f"{r['way']}\t{r['dir']}\t{t(r['duckosm'])}\t{t(r['sumo_osm'])}\t{','.join(r['sources'])}\t{r['cause']}" for r in rows]


def sumo_lane_diff(db, out):
    """sumo_compare.lane_diff of the golden db and its PBF (tests/data/monaco_sample.osm.pbf; needs netconvert and osmium)."""
    from duckosm.sumo_compare import lane_diff
    with tempfile.TemporaryDirectory() as tmp:
        rows, n = lane_diff(db, Path(__file__).parent.parent / "tests" / "data" / "monaco_sample.osm.pbf", tmp)
    (out / "monaco_sumo_lane_diff.tsv").write_text("\n".join(lane_diff_lines(rows, n)) + "\n")


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
    sumo_lane_diff(db, out)
    for p in sorted(x for x in out.iterdir() if x.is_file()):
        n = len(gzip.open(p).read()) if p.suffix == ".gz" else p.stat().st_size
        print(f"{p}: {n} bytes")


if __name__ == "__main__":
    main(*sys.argv[1:])
