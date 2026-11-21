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


def _tag_filter(tags) -> str:
    """The SQL condition of OSMnx's ``tags``: ``{key: True}`` any value, ``{key: "v"}`` one value, ``{key: [..]}`` one of
    the values. The keys are joined with OR, as in OSMnx. ``None``: all features."""
    if not tags:
        return "true"

    def q(s):
        return "'" + str(s).replace("'", "''") + "'"
    parts = []
    for key, value in tags.items():
        v = f"tags[{q(key)}]"
        if value is True:
            parts.append(f"{v} IS NOT NULL")
        elif isinstance(value, str):
            parts.append(f"{v} = {q(value)}")
        else:
            parts.append(f"{v} IN ({', '.join(q(x) for x in value)})")
    return " OR ".join(parts)


def features(source, tags=None, *, fixes=None, threads=None):
    """The OSM features of an area as a GeoDataFrame (EPSG:4326), as OSMnx's ``features_from_*`` gives them. ``source``:
    an :class:`duckosm.Area`, an OSM file or a Network (its OSM file beside it). ``tags`` as in OSMnx: ``{"building":
    True}`` (any value), ``{"amenity": "cafe"}``, ``{"amenity": ["cafe", "bar"]}``; the keys are joined with OR. Columns:
    ``feature_id`` (``way/123``), one column for each key of ``tags``, ``tags`` (all tags, a dict) and ``geometry``.
    ``fixes``: an OSM fixes file, applied first. The table comes from ``duckosm quack`` (the QuackOSM rows).

    >>> cafes = duckosm.gis.features(area, tags={"amenity": "cafe"})"""
    require("geopandas", "duckosm.gis.features")
    import duckdb
    import geopandas as gpd
    with tempfile.TemporaryDirectory() as tmp:
        files = quack(_pbf(source), out=tmp, name="features", formats=["geoparquet"], fixes=fixes, threads=threads,
                      progress=False)
        con = duckdb.connect()
        try:
            con.load_extension("spatial")
        except duckdb.Error:
            con.install_extension("spatial")
            con.load_extension("spatial")
        keys = "".join(f", tags['{k.replace(chr(39), chr(39) * 2)}'] AS \"{k.replace(chr(34), chr(34) * 2)}\""
                       for k in (tags or {}))
        df = con.execute(f"SELECT feature_id{keys}, map_from_entries(map_entries(tags)) AS tags, "
                         f"ST_AsWKB(geometry) AS wkb FROM read_parquet('{files[0]}') "
                         f"WHERE {_tag_filter(tags)} ORDER BY feature_id").df()
        con.close()
    geometry = gpd.GeoSeries.from_wkb(df.pop("wkb").map(bytes), crs="EPSG:4326")
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


# the tags of to_osm_xml that are columns of the edges with the same name
_XML_TAGS = ("highway", "name", "junction", "layer", "bridge", "tunnel", "service", "surface", "access")


def to_osm_xml(net, path, mode=None):
    """The network of one mode as an OSM XML file (``.osm``), as OSMnx's ``save_graph_xml`` writes it: a ``<node>`` for
    each node of the roads (``source``, the OSM nodes of ``refs``, ``target``), a ``<way>`` for each road, with the id ``edge_id`` of its
    forward edge and the tags ``highway``, ``name``, ``oneway``, ``lanes``, ``maxspeed``, ``layer``, ``bridge``,
    ``tunnel`` ..., ``duckosm:osm_id`` (the OSM way) and ``duckosm:edge_id``. A two-way road is one way. DuckDB writes the
    file. Returns ``{path, nodes, ways}``.

    >>> duckosm.gis.to_osm_xml(net.driving, "out/monaco.osm")"""
    from duckosm.network import resolve
    db, mode = resolve(net, mode)
    from duckosm.network import Network
    db = db if isinstance(db, Network) else Network(db)
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    e = f'"{mode}".edges'

    def attr(expr):     # an XML attribute value: & < " escaped
        return f"replace(replace(replace({expr}, '&', '&amp;'), '<', '&lt;'), '\"', '&quot;')"

    def tag(k, expr):
        return f"""CASE WHEN {expr} IS NOT NULL THEN '  <tag k="{k}" v="' || {attr(f"CAST({expr} AS VARCHAR)")} || '"/>' || chr(10) ELSE '' END"""
    tags = " || ".join([tag(k, k) for k in _XML_TAGS] + [
        tag("oneway", "CASE WHEN oneway THEN 'yes' ELSE 'no' END"),
        tag("lanes", "CASE WHEN oneway THEN lanes ELSE lanes + coalesce(twin_lanes, lanes) END"),
        tag("lanes:forward", "CASE WHEN NOT oneway THEN lanes END"),
        tag("lanes:backward", "CASE WHEN NOT oneway THEN coalesce(twin_lanes, lanes) END"),
        tag("maxspeed", "CAST(round(maxspeed_kmh) AS INTEGER)"),
        tag("duckosm:osm_id", "osm_id"), tag("duckosm:edge_id", "edge_id")])
    # the nodes of each road: source, the OSM nodes of refs, target; where refs and the vertices differ (an edge cut at a
    # virtual node), each inner vertex gets a hashed negative id
    ids = ("CASE WHEN len(e.refs) = ST_NPoints(e.geometry) THEN [e.source] || e.refs[2:len(e.refs) - 1] || [e.target] "
           "ELSE [e.source] || list_transform(range(2, ST_NPoints(e.geometry)), i -> -(hash(e.edge_id, i) >> 2)::BIGINT) "
           "|| [e.target] END")
    roads = (f"SELECT e.*, {ids} AS ids, t.lanes AS twin_lanes FROM {e} e LEFT JOIN {e} t ON t.osm_id = e.osm_id AND "
             f"t.source = e.target AND t.target = e.source AND t.is_reverse WHERE NOT e.is_reverse")
    sql = f"""
        WITH r AS ({roads}),
             p AS (SELECT unnest(ids) AS id, unnest(generate_series(1, len(ids))) AS i, geometry FROM r),
             n AS (SELECT DISTINCT ON (id) id, ST_Y(ST_PointN(geometry, CAST(i AS INTEGER))) AS lat,
                          ST_X(ST_PointN(geometry, CAST(i AS INTEGER))) AS lon FROM p ORDER BY id),
             body AS (
               SELECT 0 AS part, id AS k, '<node id="' || id || '" version="1" lat="' || printf('%.7f', lat) ||
                      '" lon="' || printf('%.7f', lon) || '"/>' AS line FROM n
               UNION ALL
               SELECT 1, edge_id, '<way id="' || edge_id || '" version="1">' || chr(10) ||
                      list_aggr(list_transform(ids, x -> '  <nd ref="' || x || '"/>' || chr(10)), 'string_agg', '') ||
                      {tags} || '</way>' FROM r)
        SELECT line FROM (
            SELECT -1 AS part, 0 AS k, '<?xml version="1.0" encoding="UTF-8"?>' || chr(10) ||
                   '<osm version="0.6" generator="duckOSM">' AS line
            UNION ALL SELECT part, k, line FROM body
            UNION ALL SELECT 2, 0, '</osm>') ORDER BY part, k"""
    con = db._connect()
    try:
        out = str(path).replace("'", "''")
        con.execute(f"COPY ({sql}) TO '{out}' (FORMAT csv, HEADER false, QUOTE '', ESCAPE '', DELIMITER '\t')")
        nodes, ways = con.execute(f"SELECT count(DISTINCT x), count(DISTINCT edge_id) FROM "
                                  f"(SELECT unnest(ids) x, edge_id FROM ({roads}))").fetchone()
    finally:
        con.close()
    return {"path": path, "nodes": nodes, "ways": ways}
