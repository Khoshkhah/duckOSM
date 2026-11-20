"""``duckosm.Network``: a built database file, with its tables, its maps, its routes and its exports as methods.

``duckosm.network("Monaco")`` gets and builds an area in one call; ``duckosm.build`` returns a Network; ``duckosm.open``
opens a built file. A Network is also the path of its file (``os.PathLike``), so each function that takes a database
path takes a Network too. The methods call the functions of the package: they hold no logic of their own. Each read
opens a read-only connection and closes it, so the file is not held open. geopandas, roadstyle and matplotlib are
imported in the methods. The maps use only the public API of roadstyle.
"""
from __future__ import annotations

import os
from pathlib import Path
from typing import Any, Iterable

from duckosm.errors import DuckOSMError, require

HIGHLIGHT = "#e8590c"
ROUTE_COLORS = ("#e8590c", "#1c7ed6", "#2f9e44", "#ae3ec9", "#f08c00")
MAP_COLUMNS = ("edge_id", "osm_id", "name", "highway", "oneway", "lanes", "length_m", "maxspeed_kmh", "layer", "bridge",
               "tunnel")
THEMES = {"light": "ofm_positron", "dark": "ofm_dark", "satellite": "satellite"}     # roadstyle base maps
_ORDER = ("driving", "walking", "cycling")


def open(path: str | os.PathLike) -> "Network":  # noqa: A001 -- duckosm.open, the name of the SDK
    """The :class:`Network` of the built database file ``path``.

    >>> net = duckosm.open("data/monaco.duckdb")"""
    path = Path(path)
    if not path.is_file():
        raise DuckOSMError(f"{path}: no such database file. Build one with duckosm.network(\"Monaco\") or "
                           f"duckosm.build(pbf=..., output=\"{path}\").")
    return Network(path)


def network(place: str | None = None, *, modes: Iterable[str] = ("driving",), out: str | os.PathLike = "data",
            name: str | None = None, fixes: str | os.PathLike | None = None, progress: Any = None, **area) -> "Network":
    """Get and build an area in one call: :func:`duckosm.get` (the extract stays in the download cache), then
    :func:`duckosm.build` into ``<out>/<name>.duckdb``. The area: ``place``, or one of ``bbox=``, ``point=`` with
    ``dist_m=``, ``osm_id=``, ``h3_cell=`` and ``boundary=`` (as :func:`duckosm.get`). ``fixes``: an OSM fixes file
    for the cut and the build. ``progress``: as :func:`duckosm.build`.

    >>> net = duckosm.network("Monaco")
    >>> net = duckosm.network(point=(7.4246, 43.7384), dist_m=500, modes=["driving", "walking"])"""
    from duckosm._api import build, get
    a = get(place, out=out, name=name, fixes=fixes, progress=progress, **area)
    db = Path(out) / f"{a.pbf.name.split('.')[0]}.duckdb"
    return build(pbf=a.pbf, boundary=a.boundary, modes=list(modes), output=db, fixes=fixes, progress=progress)


class Network(os.PathLike):
    """A built duckOSM database. ``net.path`` is its file. ``os.fspath(net)`` and ``str(net)`` give the path, and a
    Network is equal to the ``Path`` of its file, so code that used the ``Path`` from ``duckosm.build`` still works.

    >>> net = duckosm.network("Monaco")
    >>> net.modes, len(net.edges())
    >>> net.map(theme="dark")"""

    def __init__(self, path: str | os.PathLike):
        self.path = Path(path)

    # ---- the path ---------------------------------------------------------------------------------------------------
    def __fspath__(self) -> str:
        return str(self.path)

    def __str__(self) -> str:
        return str(self.path)

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

    # ---- the notebook view ------------------------------------------------------------------------------------------
    def summary(self) -> dict:
        """The file, its size, the build time and, for each mode, the edges, the nodes and the km.

        >>> net.summary()["modes"]["driving"]["km"]"""
        from duckosm._api import _size
        built = None
        if "visualization_metadata" in self.tables("main"):
            meta = self.sql("SELECT * FROM main.visualization_metadata")
            if "built_at" in meta and len(meta):
                built = str(meta.built_at[0])[:19]
        modes = {}
        for m in self.modes:
            r = self.sql(f"SELECT (SELECT count(*) FROM {m}.edges) AS edges, (SELECT count(*) FROM {m}.nodes) AS nodes, "
                         f"(SELECT round(sum(length_m) / 1000, 1) FROM {m}.edges) AS km").iloc[0]
            modes[m] = {"edges": int(r.edges), "nodes": int(r.nodes), "km": float(r.km or 0)}
        return {"name": self.path.name.split(".")[0], "path": str(self.path), "size": _size(self.path), "built": built,
                "modes": modes}

    def __repr__(self) -> str:
        if not self.path.is_file():
            return f"Network({str(self.path)!r}, missing)"
        s = self.summary()
        modes = ", ".join(f"{m} {v['edges']} edges" for m, v in s["modes"].items())
        return f"Network({s['path']!r}, {s['size']}, {modes})"

    def _repr_html_(self) -> str:
        from html import escape
        if not self.path.is_file():
            return f"<b>Network</b> <code>{escape(str(self.path))}</code>: missing"
        s = self.summary()
        rows = "".join(f"<tr><td>{m}</td><td>{v['edges']:,}</td><td>{v['nodes']:,}</td><td>{v['km']:,}</td></tr>"
                       for m, v in s["modes"].items())
        built = f", built {escape(s['built'])}" if s["built"] else ""
        return (f"<div><b>Network {escape(s['name'])}</b>: <code>{escape(s['path'])}</code>, {s['size']}{built}"
                f"<table><tr><th>mode</th><th>edges</th><th>nodes</th><th>km</th></tr>{rows}</table></div>")

    # ---- the tables -------------------------------------------------------------------------------------------------
    def _connect(self):
        """A read-only connection to the file, with the extension spatial loaded. The caller closes it."""
        import duckdb
        if not self.path.is_file():
            raise DuckOSMError(f"{self.path}: no such database file. Build it with duckosm.build or duckosm.network.")
        con = duckdb.connect(str(self.path), read_only=True)
        try:
            con.load_extension("spatial")
        except duckdb.Error:
            con.install_extension("spatial")
            con.load_extension("spatial")
        return con

    def sql(self, query: str, params: Any = None):
        """The result of the SQL ``query`` as a pandas DataFrame. ``params``: the values of ``?`` or ``$name``.

        >>> net.sql("SELECT highway, count(*) AS n FROM driving.edges GROUP BY highway")"""
        con = self._connect()
        try:
            df = con.execute(query, params).df() if params is not None else con.sql(query).df()
        finally:
            con.close()
        # a DuckDB FLOAT is a float32: as float64 through its shortest text, so that 44.2 shows as 44.2, not 44.200001
        for c in df.columns[df.dtypes == "float32"]:
            df[c] = df[c].to_numpy().astype(str).astype("float64")
        return df

    @property
    def modes(self) -> list[str]:
        """The networks in the file: the schemas with a road table ``edges`` (driving, walking, cycling ...). Not ``mm``:
        the multimodal graph joins the networks; its edges have no ``osm_id``."""
        rows = self.sql("SELECT DISTINCT table_schema FROM information_schema.columns "
                        "WHERE table_name = 'edges' AND column_name = 'osm_id'").table_schema
        return sorted(rows, key=lambda m: (_ORDER.index(m) if m in _ORDER else len(_ORDER), m))

    def tables(self, mode: str = "driving") -> list[str]:
        """The tables of a schema: a mode (``"driving"``), or ``"main"``, ``"mm"``, ``"features"`` ...

        >>> net.tables("driving")"""
        return list(self.sql("SELECT table_name FROM information_schema.tables WHERE table_schema = ? "
                             "ORDER BY table_name", [mode]).table_name)

    def columns(self, mode: str = "driving", table: str = "edges"):
        """The columns of a table and their types, as a DataFrame.

        >>> net.columns("driving")"""
        if table not in self.tables(mode):
            raise DuckOSMError(f"no table {mode}.{table}; the tables of {mode}: {', '.join(self.tables(mode)) or 'none'}")
        return self.sql(f'DESCRIBE "{mode}"."{table}"')[["column_name", "column_type"]]

    def _mode(self, mode):
        modes = self.modes
        if mode not in modes:
            raise DuckOSMError(f"no network {mode!r} in {self.path}; the networks in the file: {', '.join(modes)}. "
                               f"Build it with modes=[..., {mode!r}].")
        return mode

    def _frame(self, table, geometry, where, crs):
        gpd = require("geopandas", "Network.edges and Network.nodes")
        df = self.sql(f"SELECT * EXCLUDE ({geometry}), ST_AsWKB({geometry}) AS _wkb FROM {table} WHERE {where}")
        gdf = gpd.GeoDataFrame(df, geometry=gpd.GeoSeries.from_wkb(df.pop("_wkb").map(bytes), crs="EPSG:4326"))
        return gdf if crs is None else gdf.to_crs(crs)

    def edges(self, mode: str = "driving", where: str | None = None, crs: Any = None):
        """The edges of ``mode`` as a ``geopandas.GeoDataFrame``: all columns of ``<mode>.edges`` (``edge_id`` as
        ``int64``, ``osm_id``, ``name``, ``highway``, ``length_m``, ``maxspeed_kmh`` ...), ordered by ``edge_id``.
        ``where``: a SQL filter on these columns. The text goes into the query as it is: give only text that you trust.
        ``crs``: the CRS of the result (default EPSG:4326), for example ``3857`` or ``"EPSG:32632"``.

        >>> net.edges(where="highway = 'primary'")"""
        return self._frame(f"{self._mode(mode)}.edges", "geometry", f"({where or 'true'}) ORDER BY edge_id", crs)

    def nodes(self, mode: str = "driving", crs: Any = None):
        """The nodes (junctions) of ``mode`` as a ``geopandas.GeoDataFrame``, ordered by ``node_id``. ``crs``: as
        :meth:`edges`.

        >>> net.nodes()"""
        return self._frame(f"{self._mode(mode)}.nodes", "geom", "true ORDER BY node_id", crs)

    # ---- the map ----------------------------------------------------------------------------------------------------
    def map(self, mode: str = "driving", *, theme: str = "light", color_by: str | None = None, highlight=None,
            route=None, data=None, dashboard: bool = False, **style):
        """A roadstyle map of the edges of ``mode``: the road look of roadstyle (casing and fill, bridges and tunnels
        on top), one color for each road class, a legend and a popup.

        - ``theme``: ``"light"``, ``"dark"`` or ``"satellite"`` (the base map).
        - ``color_by``: a column. Numbers get a color ramp (``cmap=``, ``vmin=``, ``vmax=``); classes need
          ``colors={value: color}``.
        - ``highlight``: edge ids to draw in orange; the other edges are gray.
        - ``route``: a route result (``net.route_points(...)``), a list of them, or ``{label: route}``. Each is a line on
          top, in its own color, with a check box in the Layers box.
        - ``data``: a GeoDataFrame from :meth:`edges` with columns of your own (for ``color_by``).
        - ``dashboard``: roadstyle's dashboard page: the map, number tiles, a query box, a chart of km by road class
          and the card of the clicked road.
        - ``style``: other options of ``roadstyle.render_edges`` (``cmap``, ``vmin``, ``labels``, ``arrows`` ...).

        Returns the roadstyle map: it shows in a notebook, ``.save(path)`` writes it. When ``DUCKOSM_STATIC_MAPS=1``, it
        draws a small matplotlib picture in its place and returns None: the notebooks of the manual save this picture.

        >>> net.map(theme="dark")
        >>> net.map(color_by="maxspeed_kmh", cmap="viridis")
        >>> net.map(route={"car": net.route_points(a, b), "foot": net.route_points(a, b, mode="walking")})"""
        gdf = self.edges(mode) if data is None else data
        routes = self._routes(route)
        if theme not in THEMES:
            raise DuckOSMError(f"theme {theme!r}: use one of {', '.join(THEMES)}")
        if color_by is not None:
            if color_by not in gdf.columns:
                raise DuckOSMError(f"no column {color_by!r} to color by; the columns: {', '.join(map(str, gdf.columns))}")
            if "colors" not in style and gdf[color_by].dtype.kind not in "iuf":
                raise DuckOSMError(f"{color_by!r} has classes: give colors={{value: color}}, or color a number column")
        if os.environ.get("DUCKOSM_STATIC_MAPS") == "1":
            return _picture(gdf, highlight, routes, color_by=color_by, theme=theme, **style)
        rs = require("roadstyle", "Network.map")
        if color_by is not None:
            style["color_by"] = color_by
        if highlight is not None:
            style["color_table"] = {int(e): HIGHLIGHT for e in highlight}
        if routes:
            style["overlays"] = [*style.get("overlays", []),
                                 *(rs.Overlay(line, kind="line", color=ROUTE_COLORS[i % len(ROUTE_COLORS)], width=5,
                                              label=label, placement="over", popup=["route", "time_s", "length_m"])
                                   for i, (label, line) in enumerate(routes.items()))]
        style.setdefault("basemap", THEMES[theme])
        style.setdefault("name", self.path.name.split(".")[0])
        # the road columns of MAP_COLUMNS and the columns that you added; not the other columns of the table (lists such
        # as refs, and int8 columns, which roadstyle does not read)
        table = set(self.columns(mode).column_name)
        gdf = gdf[[c for c in gdf.columns if c in MAP_COLUMNS or c not in table or c == gdf.geometry.name]]
        if not dashboard:
            return rs.render_edges(gdf, **style)
        km = round(float(gdf.length_m.sum()) / 1000, 1) if "length_m" in gdf else None
        style.setdefault("dashboard", {"panel": {"sections": [
            {"id": "tiles", "kpis": [{"title": "Edges", "value": len(gdf)}, {"title": "km", "value": km}]},
            {"id": "q", "label": "Query", "part": "query", "text": "p.highway == 'primary'"},
            {"id": "km", "label": "km by road class", "part": "chart",
             "chart": {"kind": "bar", "x": "highway", "y": "length_m", "scale": 0.001}},
            {"id": "road", "label": "Selected road", "part": "road_card",
             "fields": ["name", "highway", "maxspeed_kmh", "lanes", "length_m", "edge_id"]},
        ]}})
        style.setdefault("road_popup", False)
        return rs.render_dashboard(gdf, **style)

    def save_map(self, path: str | os.PathLike, mode: str = "driving", **options) -> Path:
        """The map of :meth:`map` (the same options) as one HTML file, with its data in it. Returns the path.

        >>> net.save_map("monaco.html", theme="dark", dashboard=True)"""
        Path(path).parent.mkdir(parents=True, exist_ok=True)
        static = os.environ.pop("DUCKOSM_STATIC_MAPS", None)       # a file is always the live map
        try:
            self.map(mode, **options).save(str(path))
        finally:
            if static is not None:
                os.environ["DUCKOSM_STATIC_MAPS"] = static
        return Path(path)

    def _routes(self, route) -> dict:
        """``{label: GeoDataFrame of one line}`` of the routes for :meth:`map`; the edge geometries come from the file."""
        if route is None:
            return {}
        if isinstance(route, dict) and "edges" in route:
            route = [route]
        items = route.items() if isinstance(route, dict) else ((f"Route {i + 1}", r) for i, r in enumerate(route))
        gpd = require("geopandas", "Network.map(route=...)")
        union = " UNION ALL ".join(f"SELECT edge_id, geometry FROM {m}.edges" for m in self.modes)
        out = {}
        for label, r in items:
            if r is None:
                continue
            ids = [int(e[1] if isinstance(e, (list, tuple)) else e) for e in r["edges"]]   # [mode, id] in multimodal
            df = self.sql(f"SELECT DISTINCT ON (edge_id) edge_id, ST_AsWKB(geometry) AS wkb FROM ({union}) "
                          "WHERE list_contains($ids, edge_id)", {"ids": ids})
            if df.empty:
                continue
            geom = gpd.GeoSeries.from_wkb(df.wkb.map(bytes), crs="EPSG:4326").union_all()
            out[str(label)] = gpd.GeoDataFrame({"route": [str(label)], "time_s": [round(r.get("time_s", 0), 1)],
                                                "length_m": [round(r.get("length_m", 0))]}, geometry=[geom], crs="EPSG:4326")
        return out

    # ---- routes and exports: the functions of duckosm, with this file -----------------------------------------------
    def route(self, from_edge: int, to_edge: int, mode: str = "driving", weight: str = "time") -> dict | None:
        """:func:`duckosm.route` on this network: the fastest route between two edge ids, or None.

        >>> r = net.route(start_edge, end_edge)"""
        from duckosm._api import route
        return route(self, from_edge, to_edge, mode=self._mode(mode), weight=weight)

    def route_points(self, a: tuple, b: tuple, mode: str = "driving", **kwargs) -> dict | None:
        """:func:`duckosm.route_points` on this network: the fastest route between two ``(lon, lat)`` points, or None.

        >>> r = net.route_points((7.4155, 43.7285), (7.4335, 43.7468))"""
        from duckosm._api import route_points
        return route_points(self, a, b, mode=self._mode(mode), **kwargs)

    def route_multimodal_points(self, a: tuple, b: tuple, **kwargs) -> dict:
        """:func:`duckosm.route_multimodal_points` on this network (needs the ``mm`` schema of a multimodal build).

        >>> m = net.route_multimodal_points((7.4155, 43.7285), (7.4335, 43.7468))"""
        from duckosm._api import route_multimodal_points
        if not self.tables("mm"):
            raise DuckOSMError(f"{self.path} has no multimodal graph: build it with a config file that has "
                               "multimodal: {enabled: true}")
        return route_multimodal_points(self, a, b, **kwargs)

    def directions(self, route: dict, mode: str = "driving") -> list[dict]:
        """:func:`duckosm.directions` of a route of this network: the turn-by-turn steps.

        >>> for step in net.directions(r): print(step["text"])"""
        from duckosm._api import directions
        return directions(self, route, mode=mode)

    def to_networkx(self, mode: str = "driving", weight: str = "time"):
        """:func:`duckosm.to_networkx`: the turn graph of ``mode`` as a networkx graph (a node is an ``edge_id``).

        >>> g = net.to_networkx()"""
        require("networkx", "Network.to_networkx")
        from duckosm.routing import to_networkx
        con = self._connect()
        try:
            return to_networkx(con, mode=self._mode(mode), weight=weight)
        finally:
            con.close()

    def to_rustworkx(self, mode: str = "driving", graph: str = "turns", weight: str = "time"):
        """:func:`duckosm.to_rustworkx` on this network.

        >>> h = net.to_rustworkx(); h.attrs["index"][edge_id]"""
        require("rustworkx", "Network.to_rustworkx")
        from duckosm._api import to_rustworkx
        return to_rustworkx(self, mode=self._mode(mode), graph=graph, weight=weight)

    def to_sumo(self, out_dir: str | os.PathLike, progress: Any = None, **kwargs) -> dict:
        """:func:`duckosm.to_sumo` of this network. ``progress``: as :func:`duckosm.build`.

        >>> net.to_sumo("out/sumo")"""
        from duckosm._api import to_sumo
        from duckosm._progress import timed
        return timed(progress, f"SUMO {out_dir}", lambda: to_sumo(self, out_dir, **kwargs))

    def to_gmns(self, out_path: str | os.PathLike, progress: Any = None, **kwargs) -> dict:
        """:func:`duckosm.to_gmns` of this network.

        >>> net.to_gmns("out/gmns.duckdb", to_csv="out/gmns")"""
        from duckosm._progress import timed
        from duckosm.gmns import to_gmns
        return timed(progress, f"GMNS {out_path}", lambda: to_gmns(self, out_path, **kwargs))

    def to_gis(self, out: str | os.PathLike, progress: Any = None, **kwargs) -> dict:
        """:func:`duckosm.to_gis` of this network (a GeoPackage, GeoParquet or shapefiles).

        >>> net.to_gis("out/monaco.gpkg")"""
        from duckosm._api import to_gis
        from duckosm._progress import timed
        return timed(progress, f"GIS {out}", lambda: to_gis(self, out, **kwargs))

    def to_matsim(self, out_path: str | os.PathLike, progress: Any = None, **kwargs) -> dict:
        """:func:`duckosm.to_matsim` of this network.

        >>> net.to_matsim("out/network.xml.gz")"""
        from duckosm._api import to_matsim
        from duckosm._progress import timed
        return timed(progress, f"MATSim {out_path}", lambda: to_matsim(self, out_path, **kwargs))

    def to_opendrive(self, out_path: str | os.PathLike, progress: Any = None, **kwargs) -> dict:
        """:func:`duckosm.to_opendrive` of this network (experimental).

        >>> net.to_opendrive("out/monaco.xodr")"""
        from duckosm._api import to_opendrive
        from duckosm._progress import timed
        return timed(progress, f"OpenDRIVE {out_path}", lambda: to_opendrive(self, out_path, **kwargs))


def _picture(gdf, highlight=None, routes=None, color_by=None, cmap="viridis", colors=None, theme="light", **_):
    """The matplotlib picture of :meth:`Network.map`."""
    require("matplotlib", "the picture of Network.map")
    import matplotlib.pyplot as plt
    from matplotlib.lines import Line2D

    _, ax = plt.subplots(figsize=(7, 7), dpi=72)
    if theme == "dark":
        ax.figure.set_facecolor("#222222")
    handles = []
    if colors:                                   # classes: gray edges, then each class in its color
        gdf.plot(ax=ax, color="#cccccc", linewidth=0.5)
        for value, color in colors.items():
            gdf[gdf[color_by] == value].plot(ax=ax, color=color, linewidth=3)
            handles.append(Line2D([], [], color=color, linewidth=3, label=value))
    elif color_by:                               # numbers: a color ramp, gray where there is no value
        gdf.plot(ax=ax, column=color_by, cmap=cmap, linewidth=1.2, legend=True,
                 legend_kwds={"label": color_by, "shrink": 0.6}, missing_kwds={"color": "#cccccc", "linewidth": 0.5})
    elif "highway" in gdf and not routes and highlight is None:    # the road classes (gray under routes)
        gdf.plot(ax=ax, column="highway", cmap="tab20", linewidth=0.8, legend=True,
                 legend_kwds={"loc": "lower right", "fontsize": 7})
    else:
        gdf.plot(ax=ax, color="#555555", linewidth=0.6)
    if highlight is not None:
        gdf[gdf.edge_id.isin(list(highlight))].plot(ax=ax, color=HIGHLIGHT, linewidth=3)
    for i, (label, line) in enumerate((routes or {}).items()):
        color = ROUTE_COLORS[i % len(ROUTE_COLORS)]
        line.plot(ax=ax, color=color, linewidth=3.5)
        handles.append(Line2D([], [], color=color, linewidth=3.5, label=label))
    if handles:
        ax.legend(handles=handles, loc="lower right")
    ax.set_axis_off()
    plt.show()
