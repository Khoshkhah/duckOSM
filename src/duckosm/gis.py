"""``duckosm.gis``: GIS and data engineers (docs/proposals/python-sdk.md, section 5.2).

``to_gis`` (GeoPackage, GeoParquet, shapefiles), ``quack`` (the QuackOSM table, GDAL's layers, PMTiles), ``admin`` (the
table of administrative boundaries), ``features`` (OSM features as a GeoDataFrame), ``to_osm_xml`` (the network as OSM
XML) and ``admin_boundaries`` (the administrative areas as a GeoDataFrame). The work is in Rust (``duckosm quack``,
``duckosm admin``) and in DuckDB SQL; Python only makes the GeoDataFrame.
"""
from __future__ import annotations

import tempfile
from pathlib import Path

from duckosm._progress import shown
from duckosm.errors import DuckOSMError, require


def to_gis(net, out, modes=None, fmt="gpkg", boundary=True, name=None, srs="EPSG:4326", ogr2ogr_bin=None, mode=None):
    """The edges and nodes of the modes (default: all of the file; a ``ModeView`` or ``mode=``: one) as a GeoPackage
    (``fmt="gpkg"``), GeoParquet (``"geoparquet"``) or shapefiles (``"shp"``). Returns ``{path, dir, files, layers}``.
    The layers are ``edges_<mode>``, ``nodes_<mode>`` and ``boundary``. GeoParquet also writes one layer for each table
    of the base map (``features_<table>``, for example ``features_buildings``); the GeoPackage and the shapefiles do not.

    >>> duckosm.gis.to_gis(net, "out/monaco.gpkg")"""
    from duckosm import _api
    if hasattr(net, "network"):                       # a ModeView
        net, modes = net.network, [net.mode]
    elif mode is not None:
        modes = [mode]
    return _api.to_gis(net, out, modes=modes, fmt=fmt, boundary=boundary, name=name, srs=srs, ogr2ogr_bin=ogr2ogr_bin)


def quack(pbf, **kwargs):
    """``duckosm quack`` on an OSM file or an :class:`duckosm.Area`: the QuackOSM features table (``formats=``), GDAL's
    five layers (``gdal=True``); with ``fixes=`` the fixes are in it. Options as :func:`duckosm.quack`. Returns the paths.

    >>> duckosm.gis.quack(area, out="out", name="monaco", formats=["geoparquet"])"""
    from duckosm import _api
    return _api.quack(getattr(pbf, "pbf", pbf), **kwargs)


def admin(net, pbf, gpkg=None):
    """``duckosm admin``: add the table ``admin_boundaries`` (all OSM administrative levels, each with its parent) to the
    database, from the boundary relations of the OSM file ``pbf`` (or an :class:`duckosm.Area`). ``gpkg``: also a
    GeoPackage. The database is changed in place. Returns the database.

    >>> duckosm.gis.admin(net, area)"""
    from duckosm._api import _opt, _run
    _run("admin", "--pbf", getattr(pbf, "pbf", pbf), "--db", net, *_opt("--gpkg", gpkg))
    return net


# ---- new in the SDK ----------------------------------------------------------------------------------------------------

def _pbf(source) -> Path:
    """The OSM file of an :class:`duckosm.Area`, of a path, or of a Network (the ``.osm.pbf`` beside its file, where
    ``duckosm.network`` and ``Area.build`` put it)."""
    from duckosm.network import ModeView, Network
    if isinstance(source, ModeView):
        source = source.network
    if isinstance(source, Network):
        pbf = source.path.with_name(source.path.name.split(".")[0] + ".osm.pbf")
        if not pbf.is_file():
            raise DuckOSMError(f"{shown(source.path)}: no OSM file {pbf.name} beside it. Give the OSM file or the Area: "
                               f"duckosm.gis.features(duckosm.get(...), tags=...)")
        return pbf
    pbf = Path(getattr(source, "pbf", source))
    if not pbf.is_file():
        raise DuckOSMError(f"{shown(pbf)}: no such OSM file")
    return pbf


def features(source, tags=None, *, fixes=None, threads=None):
    """The OSM features of an area as a GeoDataFrame (EPSG:4326), as OSMnx's ``features_from_*`` gives them. ``source``:
    an :class:`duckosm.Area`, an OSM file or a Network (its OSM file beside it). ``tags`` as in OSMnx: ``{"building":
    True}`` (any value), ``{"amenity": "cafe"}``, ``{"amenity": ["cafe", "bar"]}``; the keys are joined with OR. Columns:
    ``feature_id`` (``way/123``), one column for each key of ``tags``, ``tags`` (all tags, a dict) and ``geometry``.
    ``fixes``: an OSM fixes file, applied first. The table comes from ``duckosm quack`` (the QuackOSM rows).

    >>> cafes = duckosm.gis.features(area, tags={"amenity": "cafe"})"""
    require("geopandas", "duckosm.gis.features")
    import geopandas as gpd
    import pandas as pd

    from duckosm import _rust
    asked = [(str(k), None if v is True else [str(v)] if isinstance(v, str) else [str(x) for x in v]) for k, v in (tags or {}).items()]
    with tempfile.TemporaryDirectory() as tmp:
        files = quack(_pbf(source), out=tmp, name="features", formats=["geoparquet"], fixes=fixes, threads=threads,
                      progress=False)
        rows = _rust.gis_features(str(files[0]), asked)
    df = pd.DataFrame({"feature_id": [r[0] for r in rows],
                       **{k: [r[1][i] for r in rows] for i, (k, _) in enumerate(asked)},
                       "tags": [dict(r[2]) for r in rows]})
    geometry = gpd.GeoSeries.from_wkb([bytes(r[3]) for r in rows], crs="EPSG:4326")
    return gpd.GeoDataFrame(df, geometry=geometry, crs="EPSG:4326")


def admin_boundaries(net, pbf=None):
    """The administrative areas of the network's area as a GeoDataFrame (EPSG:4326): ``osm_id``, ``name``,
    ``admin_level``, the parent and the geometry, one row for each OSM boundary relation. When the database has no table
    ``admin_boundaries``, it is added first with :func:`admin` from ``pbf`` (default: the OSM file beside the database).

    >>> duckosm.gis.admin_boundaries(net)"""
    require("geopandas", "duckosm.gis.admin_boundaries")
    import geopandas as gpd
    from duckosm.network import ModeView, Network
    net = net.network if isinstance(net, ModeView) else Network(net)
    if "main.admin_boundaries" not in net.tables():
        admin(net, _pbf(pbf if pbf is not None else net))
    df = net.sql("SELECT * EXCLUDE (geometry), ST_AsWKB(geometry) AS wkb FROM main.admin_boundaries "
                 "ORDER BY admin_level, name")
    geometry = gpd.GeoSeries.from_wkb(df.pop("wkb").map(bytes), crs="EPSG:4326")
    return gpd.GeoDataFrame(df, geometry=geometry, crs="EPSG:4326")


def to_osm_xml(net, path, mode=None):
    """The network of one mode as an OSM XML file (``.osm``), as OSMnx's ``save_graph_xml`` writes it: a ``<node>`` for
    each node of the roads (``source``, the OSM nodes of ``refs``, ``target``), a ``<way>`` for each road, with the id ``edge_id`` of its
    forward edge and the tags ``highway``, ``name``, ``oneway``, ``lanes``, ``maxspeed``, ``layer``, ``bridge``,
    ``tunnel`` ..., ``duckosm:osm_id`` (the OSM way) and ``duckosm:edge_id``. A two-way road is one way. Rust (DuckDB) writes the
    file. Returns ``{path, nodes, ways}``.

    >>> duckosm.gis.to_osm_xml(net.driving, "out/monaco.osm")"""
    from duckosm import _api
    from duckosm.network import resolve
    db, mode = resolve(net, mode)
    path = Path(path)
    with _api._file(db) as f:
        nodes, ways = _api._rust().to_osm_xml(f, str(path), mode)
    return {"path": path, "nodes": nodes, "ways": ways}
