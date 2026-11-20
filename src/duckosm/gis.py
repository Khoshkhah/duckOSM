"""``duckosm.gis``: GIS and data engineers (docs/proposals/python-sdk.md, section 5.2).

Today: ``to_gis`` (GeoPackage, GeoParquet, shapefiles), ``quack`` (the QuackOSM table, GDAL's layers, PMTiles) and
``admin`` (the table of administrative boundaries). Later: ``features``, ``to_osm_xml``, ``admin_boundaries``.
"""


def to_gis(net, out, modes=None, fmt="gpkg", boundary=True, name=None, srs="EPSG:4326", ogr2ogr_bin=None, mode=None):
    """The edges and nodes of the modes (default: all of the file; a ``ModeView`` or ``mode=``: one) as a GeoPackage
    (``fmt="gpkg"``), GeoParquet (``"geoparquet"``) or shapefiles (``"shp"``). Returns ``{path, dir, files, layers}``.

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
