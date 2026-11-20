"""duckosm.gis on Monaco: features, to_osm_xml, admin_boundaries (docs/proposals/python-sdk.md section 5.2)."""
import shutil
import xml.etree.ElementTree as ET
from pathlib import Path

import pytest

import duckosm

DATA = Path(__file__).parent / "data"
gpd = pytest.importorskip("geopandas")


@pytest.fixture(scope="module")
def net(tmp_path_factory):
    folder = tmp_path_factory.mktemp("gis")
    shutil.copy(DATA / "monaco.osm.pbf", folder / "monaco.osm.pbf")      # the OSM file beside the database
    return duckosm.build(pbf=folder / "monaco.osm.pbf", boundary=DATA / "monaco.geojson", modes=["driving", "walking"],
                         output=folder / "monaco.duckdb", progress=False)


def test_features_tags_as_osmnx():
    cafes = duckosm.gis.features(DATA / "monaco.osm.pbf", tags={"amenity": "cafe"})
    assert len(cafes) > 0 and set(cafes.amenity) == {"cafe"} and cafes.crs.to_epsg() == 4326
    assert all(t["amenity"] == "cafe" for t in cafes.tags)
    both = duckosm.gis.features(DATA / "monaco.osm.pbf", tags={"amenity": ["cafe", "bar"], "building": True})
    assert set(both.amenity.dropna()) >= {"cafe", "bar"} and both.building.notna().any()
    assert len(both) > len(cafes)


def test_features_of_a_network(net):
    assert len(duckosm.gis.features(net, tags={"building": True})) > 1000


def test_to_osm_xml(net, tmp_path):
    out = duckosm.gis.to_osm_xml(net.driving, tmp_path / "monaco.osm")
    root = ET.parse(out["path"]).getroot()
    nodes = {n.get("id") for n in root.iter("node")}
    ways = root.findall("way")
    assert len(ways) == out["ways"] == net.sql("SELECT count(*) n FROM driving.edges WHERE NOT is_reverse").n[0]
    assert len(nodes) == out["nodes"]
    assert all(nd.get("ref") in nodes for w in ways for nd in w.iter("nd"))       # each node of a way is in the file
    tags = {t.get("k"): t.get("v") for t in ways[0].iter("tag")}
    assert {"highway", "oneway", "maxspeed", "duckosm:edge_id", "duckosm:osm_id"} <= set(tags)
    assert tags["duckosm:edge_id"] == ways[0].get("id")


def test_admin_boundaries(net):
    a = duckosm.gis.admin_boundaries(net)
    assert len(a) > 0 and {"osm_id", "name", "admin_level"} <= set(a.columns) and a.crs.to_epsg() == 4326
    assert "Monaco" in set(a.name)


def test_osmium_reads_the_osm_xml(net, tmp_path):
    if not shutil.which("osmium"):
        pytest.skip("no osmium")
    import subprocess
    out = duckosm.gis.to_osm_xml(net.driving, tmp_path / "monaco.osm")
    info = subprocess.run(["osmium", "fileinfo", "-e", "-g", "data.count.ways", str(out["path"])], capture_output=True,
                          text=True, check=True).stdout
    assert int(info) == out["ways"]
