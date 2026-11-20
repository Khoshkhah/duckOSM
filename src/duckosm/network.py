"""``duckosm.Network``: a built database file, with its tables, its maps, its routes and its exports as methods.

``duckosm.build`` returns a Network; ``duckosm.open`` opens a built file. A Network is also the path of its file
(``os.PathLike``), so each function that takes a database path takes a Network too. The methods call the functions of
the package: they hold no logic of their own. Each read opens a read-only connection and closes it, so the file is not
held open. geopandas, roadstyle and matplotlib are imported in the methods.
"""
import os
from pathlib import Path

HIGHLIGHT = "#e8590c"
MAP_COLUMNS = ("edge_id", "osm_id", "name", "highway", "oneway", "lanes", "length_m", "maxspeed_kmh", "layer", "bridge",
               "tunnel")
_ORDER = ("driving", "walking", "cycling")


def open(path):  # noqa: A001 -- duckosm.open, as the name of the SDK
    """The :class:`Network` of the built database file ``path``. A missing file raises FileNotFoundError."""
    path = Path(path)
    if not path.is_file():
        raise FileNotFoundError(f"{path}: no such database file")
    return Network(path)


class Network(os.PathLike):
    """A built duckOSM database. ``net.path`` is its file. ``os.fspath(net)`` and ``str(net)`` give the path, and a
    Network is equal to the ``Path`` of its file, so code that used the ``Path`` from ``duckosm.build`` still works."""

    def __init__(self, path):
        self.path = Path(path)

    # ---- the path ---------------------------------------------------------------------------------------------------
    def __fspath__(self):
        return str(self.path)

    def __str__(self):
        return str(self.path)

    def __repr__(self):
        return f"Network({str(self.path)!r})"

    def __eq__(self, other):
        if isinstance(other, Network):
            return self.path == other.path
        if isinstance(other, (str, os.PathLike)):
            return self.path == Path(other)
        return NotImplemented

    def __hash__(self):
        return hash(self.path)

    def __getattr__(self, name):
        """The attributes of the ``Path`` (``name``, ``parent``, ``exists()`` ...), as before ``build`` gave a Network."""
        if name.startswith("__") or name == "path":
            raise AttributeError(name)
        return getattr(self.path, name)

    # ---- the tables -------------------------------------------------------------------------------------------------
    def _connect(self):
        """A read-only connection to the file, with the extension spatial loaded. The caller closes it."""
        import duckdb
        con = duckdb.connect(str(self.path), read_only=True)
        try:
            con.load_extension("spatial")
        except duckdb.Error:
            con.install_extension("spatial")
            con.load_extension("spatial")
        return con

    def sql(self, query, params=None):
        """The result of the SQL ``query`` as a pandas DataFrame. ``params``: the values of ``?`` or ``$name``."""
        con = self._connect()
        try:
            return con.execute(query, params).df() if params is not None else con.sql(query).df()
        finally:
            con.close()

    @property
    def modes(self):
        """The networks in the file: the schemas with a road table ``edges`` (driving, walking, cycling ...). Not ``mm``:
        the multimodal graph joins the networks; its edges have no ``osm_id``."""
        rows = self.sql("SELECT DISTINCT table_schema FROM information_schema.columns "
                        "WHERE table_name = 'edges' AND column_name = 'osm_id'").table_schema
        return sorted(rows, key=lambda m: (_ORDER.index(m) if m in _ORDER else len(_ORDER), m))

    def _mode(self, mode):
        if mode not in self.modes:
            raise ValueError(f"no network {mode!r} in {self.path}: {self.modes}")
        return mode

    def _frame(self, table, geometry, where):
        import geopandas as gpd
        df = self.sql(f"SELECT * EXCLUDE ({geometry}), ST_AsWKB({geometry}) AS _wkb FROM {table} WHERE {where or 'true'}")
        return gpd.GeoDataFrame(df, geometry=gpd.GeoSeries.from_wkb(df.pop("_wkb").map(bytes), crs="EPSG:4326"))

    def edges(self, mode="driving", where=None):
        """The edges of ``mode`` as a ``geopandas.GeoDataFrame`` in EPSG:4326: all columns of ``<mode>.edges``
        (``edge_id`` as ``int64``, ``osm_id``, ``name``, ``highway``, ``length_m``, ``maxspeed_kmh`` ...), ordered by
        ``edge_id``. ``where``: a SQL filter on these columns, for example ``"highway = 'primary'"``. The text goes into
        the query as it is: give only text that you trust."""
        return self._frame(f"{self._mode(mode)}.edges", "geometry", f"({where or 'true'}) ORDER BY edge_id")

    def nodes(self, mode="driving"):
        """The nodes (junctions) of ``mode`` as a ``geopandas.GeoDataFrame`` in EPSG:4326, ordered by ``node_id``."""
        return self._frame(f"{self._mode(mode)}.nodes", "geom", "true ORDER BY node_id")

    # ---- the map ----------------------------------------------------------------------------------------------------
    def map(self, mode="driving", highlight=None, data=None, **style):
        """A map of the edges of ``mode``, or of ``data`` (a GeoDataFrame from :meth:`edges` with columns of your own).
        ``highlight``: edge ids to draw in orange, for example the ``edges`` of a route; the other edges are gray.
        ``style``: the options of ``roadstyle.render_edges``, for example ``color_by="maxspeed_kmh", cmap="viridis"``
        (a number column) or ``color_by="road", colors={value: color}`` (a class column).

        Returns the live roadstyle map (it shows in Jupyter; needs roadstyle). When ``DUCKOSM_STATIC_MAPS=1``, it draws
        a small matplotlib picture with the same ``highlight``, ``color_by``, ``cmap`` and ``colors`` and returns None:
        the notebooks of the manual save this picture, not megabytes of map HTML."""
        gdf = self.edges(mode) if data is None else data
        if os.environ.get("DUCKOSM_STATIC_MAPS") == "1":
            return _picture(gdf, highlight, **style)
        try:
            import roadstyle
        except ImportError as e:
            raise ImportError("Network.map needs roadstyle: pip install roadstyle") from e
        if highlight is not None:
            style["color_table"] = {int(e): HIGHLIGHT for e in highlight}
        # the map gets the road columns of MAP_COLUMNS and the columns that you added; not the other columns of the
        # table (lists such as refs, and int8 columns, which roadstyle does not read)
        table = set(self.sql(f"DESCRIBE {self._mode(mode)}.edges").column_name)
        keep = [c for c in gdf.columns if c in MAP_COLUMNS or c not in table or c == gdf.geometry.name]
        return roadstyle.render_edges(gdf[keep], **style)

    # ---- routes and exports: the functions of duckosm, with this file -----------------------------------------------
    def route(self, from_edge, to_edge, mode="driving", weight="time"):
        """:func:`duckosm.route` on this network."""
        from duckosm._api import route
        return route(self, from_edge, to_edge, mode=mode, weight=weight)

    def route_points(self, a, b, mode="driving", **kwargs):
        """:func:`duckosm.route_points` on this network."""
        from duckosm._api import route_points
        return route_points(self, a, b, mode=mode, **kwargs)

    def route_multimodal_points(self, a, b, **kwargs):
        """:func:`duckosm.route_multimodal_points` on this network (needs the ``mm`` schema)."""
        from duckosm._api import route_multimodal_points
        return route_multimodal_points(self, a, b, **kwargs)

    def directions(self, route, mode="driving"):
        """:func:`duckosm.directions` of a route of this network."""
        from duckosm._api import directions
        return directions(self, route, mode=mode)

    def to_networkx(self, mode="driving", weight="time"):
        """:func:`duckosm.to_networkx`: the turn graph of ``mode`` as a networkx graph."""
        from duckosm.routing import to_networkx
        con = self._connect()
        try:
            return to_networkx(con, mode=mode, weight=weight)
        finally:
            con.close()

    def to_rustworkx(self, mode="driving", graph="turns", weight="time"):
        """:func:`duckosm.to_rustworkx` on this network."""
        from duckosm._api import to_rustworkx
        return to_rustworkx(self, mode=mode, graph=graph, weight=weight)

    def to_sumo(self, out_dir, **kwargs):
        """:func:`duckosm.to_sumo` of this network."""
        from duckosm._api import to_sumo
        return to_sumo(self, out_dir, **kwargs)

    def to_gmns(self, out_path, **kwargs):
        """:func:`duckosm.to_gmns` of this network."""
        from duckosm.gmns import to_gmns
        return to_gmns(self, out_path, **kwargs)

    def to_gis(self, out, **kwargs):
        """:func:`duckosm.to_gis` of this network."""
        from duckosm._api import to_gis
        return to_gis(self, out, **kwargs)

    def to_matsim(self, out_path, **kwargs):
        """:func:`duckosm.to_matsim` of this network."""
        from duckosm._api import to_matsim
        return to_matsim(self, out_path, **kwargs)

    def to_opendrive(self, out_path, **kwargs):
        """:func:`duckosm.to_opendrive` of this network."""
        from duckosm._api import to_opendrive
        return to_opendrive(self, out_path, **kwargs)


def _picture(gdf, highlight=None, color_by=None, cmap="viridis", colors=None, **_):
    """The matplotlib picture of :meth:`Network.map`."""
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
