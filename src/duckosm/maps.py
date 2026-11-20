"""The edges of a built database as a GeoDataFrame, and a quick map of them (the notebooks of docs/notebooks/).

``edges`` needs geopandas; ``show_edges`` needs roadstyle (``pip install roadstyle``), or matplotlib when the
environment variable ``DUCKOSM_STATIC_MAPS`` is ``1``. All three are imported in the functions.
"""
import os

_COLUMNS = "edge_id, osm_id, name, highway, length_m, maxspeed_kmh"
HIGHLIGHT = "#e8590c"


def edges(db, mode="driving", where=None):
    """The edges of ``mode`` in the database ``db`` (a path or a DuckDB connection to a file) as a
    ``geopandas.GeoDataFrame`` in EPSG:4326, with the columns ``edge_id``, ``osm_id``, ``name``, ``highway``,
    ``length_m``, ``maxspeed_kmh`` and ``geometry``.

    ``where``: a SQL filter on the columns of ``<mode>.edges``, for example ``"highway = 'primary'"``. The text goes
    into the query as it is: give only text that you trust."""
    import duckdb
    import geopandas as gpd

    if not str(mode).isidentifier():
        raise ValueError(f"mode {mode!r} is not a schema name")
    own = not isinstance(db, duckdb.DuckDBPyConnection)      # a path: open it here, and close it at the end
    con = duckdb.connect(str(db), read_only=True) if own else db
    try:
        con.load_extension("spatial")
    except duckdb.Error:
        con.install_extension("spatial")
        con.load_extension("spatial")
    try:
        df = con.sql(f"SELECT {_COLUMNS}, ST_AsWKB(geometry) AS wkb FROM {mode}.edges"
                     f" WHERE {where or 'true'} ORDER BY edge_id").df()
    finally:
        if own:
            con.close()
    geometry = gpd.GeoSeries.from_wkb(df.pop("wkb").map(bytes), crs="EPSG:4326")
    return gpd.GeoDataFrame(df, geometry=geometry)


def show_edges(gdf, highlight=None, **style):
    """A map of the edges ``gdf`` (from :func:`edges`). ``highlight``: edge ids to draw in orange (for example the
    ``edges`` of a route); the other edges are gray. ``style``: the options of ``roadstyle.render_edges``, for example
    ``color_by="maxspeed_kmh", cmap="viridis"`` (a number column) or ``color_by="road", colors={value: color}``
    (a class column).

    Returns the live roadstyle map (it shows in Jupyter). When ``DUCKOSM_STATIC_MAPS=1``, it draws a small
    matplotlib picture in its place, with the same ``highlight``, ``color_by``, ``cmap`` and ``colors``, and returns
    None: the notebooks of the manual save this picture, not megabytes of map HTML."""
    if os.environ.get("DUCKOSM_STATIC_MAPS") == "1":
        return _picture(gdf, highlight, **style)
    try:
        import roadstyle
    except ImportError as e:
        raise ImportError("show_edges needs roadstyle: pip install roadstyle") from e
    if highlight is not None:
        style["color_table"] = {int(e): HIGHLIGHT for e in highlight}
    return roadstyle.render_edges(gdf, **style)


def _picture(gdf, highlight=None, color_by=None, cmap="viridis", colors=None, **_):
    """The matplotlib picture of :func:`show_edges`."""
    import matplotlib.pyplot as plt
    from matplotlib.lines import Line2D

    _, ax = plt.subplots(figsize=(7, 7), dpi=72)
    if colors:                                   # classes: gray edges, then each class in its color
        gdf.plot(ax=ax, color="#cccccc", linewidth=0.5)
        for value, color in colors.items():
            gdf[gdf[color_by] == value].plot(ax=ax, color=color, linewidth=3)
        ax.legend(handles=[Line2D([], [], color=c, linewidth=3, label=v) for v, c in colors.items()], loc="lower right")
    elif color_by:                               # numbers: a color ramp, gray where there is no value
        gdf.plot(ax=ax, column=color_by, cmap=cmap, linewidth=1.2, legend=True,
                 legend_kwds={"label": color_by, "shrink": 0.6}, missing_kwds={"color": "#cccccc", "linewidth": 0.5})
    else:
        gdf.plot(ax=ax, color="#555555", linewidth=0.6)
    if highlight is not None:
        gdf[gdf.edge_id.isin(list(highlight))].plot(ax=ax, color=HIGHLIGHT, linewidth=3)
    ax.set_axis_off()
    plt.show()
