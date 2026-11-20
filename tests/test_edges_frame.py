"""duckosm.edges and duckosm.show_edges on Monaco (src/duckosm/maps.py)."""
from pathlib import Path

import duckdb
import pytest

import duckosm

DATA = Path(__file__).parent / "data"
pytest.importorskip("geopandas")


@pytest.fixture(scope="module")
def db(tmp_path_factory):
    out = tmp_path_factory.mktemp("maps") / "monaco.duckdb"
    return duckosm.build(pbf=DATA / "monaco.osm.pbf", boundary=DATA / "monaco.geojson", output=out)


def test_edges_are_the_rows_of_the_table(db):
    gdf = duckosm.edges(db)
    con = duckdb.connect(str(db), read_only=True)
    assert len(gdf) == con.execute("SELECT count(*) FROM driving.edges").fetchone()[0]
    assert str(gdf.crs) == "EPSG:4326" and gdf.geometry.geom_type.eq("LineString").all()
    assert str(gdf.edge_id.dtype) == "int64"                    # the BIGINT hash stays exact
    main = duckosm.edges(con, where="highway = 'primary'")    # a connection and a SQL filter
    assert 0 < len(main) < len(gdf) and set(main.highway) == {"primary"}
    with pytest.raises(ValueError):
        duckosm.edges(db, mode="driving; DROP TABLE x")


def test_show_edges_live_map_highlights(db, monkeypatch):
    pytest.importorskip("roadstyle")
    monkeypatch.delenv("DUCKOSM_STATIC_MAPS", raising=False)
    gdf = duckosm.edges(db)
    m = duckosm.show_edges(gdf, highlight=gdf.edge_id[:3])
    assert hasattr(m, "_repr_html_")


def test_show_edges_picture(db, monkeypatch):
    pytest.importorskip("matplotlib")
    import matplotlib
    matplotlib.use("Agg")
    monkeypatch.setenv("DUCKOSM_STATIC_MAPS", "1")
    gdf = duckosm.edges(db)
    assert duckosm.show_edges(gdf, highlight=gdf.edge_id[:3], color_by="maxspeed_kmh") is None
