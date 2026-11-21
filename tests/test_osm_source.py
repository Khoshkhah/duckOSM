"""main.osm_source on Monaco: the version of the OSM data of a build (docs/reference/database.md, osm_source)."""
import hashlib
import json
import shutil
from pathlib import Path

import duckdb

import duckosm
from duckosm._api import _stream

DATA = Path(__file__).parent / "data"


def test_own_pbf_and_an_older_file(tmp_path):
    net = duckosm.build(pbf=DATA / "monaco.osm.pbf", boundary=DATA / "monaco.geojson", modes=["driving"],
                        output=tmp_path / "monaco.duckdb", progress=False)
    v = net.osm_version
    assert net.has_osm_source() and v["osm_timestamp"] is not None
    assert v["extract"] is None and v["extract_url"] is None, "an own OSM file has no extract"
    assert v["pbf_sha256"] == hashlib.sha256((DATA / "monaco.osm.pbf").read_bytes()).hexdigest()
    assert f"<td>OSM data</td><td>{str(v['osm_timestamp'])[:10]} (your own OSM file)</td>" in net._repr_html_()
    # a file from an older build has no main.osm_source
    old = tmp_path / "old.duckdb"
    shutil.copy(net.path, old)
    con = duckdb.connect(str(old))
    con.execute("DROP TABLE main.osm_source")
    con.close()
    old = duckosm.Network(old)
    assert old.osm_version is None and not old.has_osm_source() and "unknown" in old._repr_html_()


def test_get_names_the_extract(tmp_path):
    """duckosm get with a local list of extracts and the Monaco extract in its cache: the build names the extract."""
    cache = tmp_path / "cache"
    (cache / "geofabrik" / "europe").mkdir(parents=True)
    shutil.copy(DATA / "monaco.osm.pbf", cache / "geofabrik" / "europe" / "monaco-latest.osm.pbf")
    w, s, e, n = 7.38, 43.70, 7.47, 43.77
    index = tmp_path / "index-v1.json"
    index.write_text(json.dumps({"type": "FeatureCollection", "features": [{"type": "Feature", "geometry": {
        "type": "Polygon", "coordinates": [[[w, s], [e, s], [e, n], [w, n], [w, s]]]}, "properties": {
        "id": "monaco", "name": "Monaco", "urls": {"pbf": "https://download.geofabrik.de/europe/monaco-latest.osm.pbf"}}}]}))
    _stream(["get", "--boundary", str(DATA / "monaco.geojson"), "--cache", str(cache), "--index", str(index),
             "-o", str(tmp_path / "area"), "--name", "monaco"])
    pbf = tmp_path / "area" / "monaco.osm.pbf"
    net = duckosm.build(pbf=pbf, boundary=DATA / "monaco.geojson", modes=["driving"], output=tmp_path / "m.duckdb", progress=False)
    v = net.osm_version
    assert v["extract"] == "monaco" and v["extract_url"].endswith("monaco-latest.osm.pbf") and v["downloaded_at"] is not None
    assert v["pbf_sha256"] == hashlib.sha256(pbf.read_bytes()).hexdigest()
    assert "(Geofabrik monaco)" in net._repr_html_()
    assert not any(str(Path.home()) in str(x) for x in v.values()), "no path of the home folder in the table"
