"""The core of the duckOSM SDK (docs/proposals/python-sdk.md, section 4): ``Network``, ``ModeView``, ``network()`` and
``open()``.

A ``Network`` is a built ``.duckdb`` file with one or more modes. It is also the path of its file (``os.PathLike``), so
each function that takes a database path takes a ``Network``. ``net["walking"]`` (or ``net.walking``) is a ``ModeView``:
the same calls for one mode, with no ``mode=``. The methods call the functions of the modules (``duckosm.routing``,
``duckosm.sim``, ...): the logic exists once. Each read opens a read-only connection and closes it. geopandas, roadstyle
and matplotlib are imported in the methods. The maps come from roadstyle (``roadstyle.duckosm.page``).
"""
from __future__ import annotations

import os
from pathlib import Path
from typing import Any, Iterable

from duckosm.errors import DuckOSMError, require

HIGHLIGHT = "#e8590c"
ROUTE_COLORS = ("#e8590c", "#1c7ed6", "#2f9e44", "#ae3ec9", "#f08c00")
MODE_COLORS = {"driving": "#e8590c", "walking": "#2f9e44", "cycling": "#1c7ed6"}
MAP_COLUMNS = ("edge_id", "osm_id", "name", "highway", "oneway", "lanes", "length_m", "maxspeed_kmh", "layer", "bridge",
               "tunnel", "mode")
DEFAULT_MODES = ("driving", "walking", "cycling")


# ---- get a network ---------------------------------------------------------------------------------------------------

def open(path: str | os.PathLike) -> "Network":  # noqa: A001 -- duckosm.open, the name of the SDK
    """The :class:`Network` of a built ``.duckdb`` file.

    >>> net = duckosm.open("data/monaco.duckdb")"""
    path = Path(path)
    if not path.is_file():
        raise DuckOSMError(f"{path}: no such database file. Build one with duckosm.network(\"Monaco\") or "
                           f"duckosm.build(pbf=..., output=\"{path}\").")
    return Network(path)


def network(place: str | None = None, *, modes: Iterable[str] | None = DEFAULT_MODES, fixes: str | os.PathLike | None = None,
            out: str | os.PathLike = "data", name: str | None = None, pbf: str | os.PathLike | None = None,
            progress: Any = None, **area) -> "Network":
    """Get the OSM file of an area and build its network, in one call. The download stays in a cache.

    The area: ``place`` (a name), or one of ``bbox=(west, south, east, north)``, ``point=(lon, lat)`` with ``dist_m=``,
    ``osm_id=``, ``h3_cell=``, ``boundary=`` (a GeoJSON file, a dict or a GeoDataFrame), or ``pbf=`` (an ``.osm.pbf``
    file of your own; ``boundary=`` cuts it). ``modes``: the networks (default driving, walking and cycling). ``fixes``: an
    OSM fixes file for the cut and the build. The file is ``<out>/<name>.duckdb``. ``progress``: as :func:`duckosm.build`.

    >>> net = duckosm.network("Monaco")
    >>> net = duckosm.network(point=(7.4246, 43.7384), dist_m=500, modes=["walking"])"""
    from duckosm._api import build, get
    modes = None if modes is None else list(modes)
    if pbf is not None:
        stem = name or Path(pbf).name.split(".")[0]
        return build(pbf=pbf, boundary=area.pop("boundary", None), modes=modes, output=Path(out) / f"{stem}.duckdb",
                     fixes=fixes, progress=progress)
    return get(place, out=out, name=name, fixes=fixes, progress=progress, **area).build(modes=modes, fixes=fixes,
                                                                                         progress=progress)


def resolve(target, mode: str | None, default: str | None = "driving"):
    """``(database, mode)`` for a module function: a :class:`ModeView` gives its own; a :class:`Network` applies the mode
    rules (one mode: that one; more: ``mode`` must be given); a path or a connection keeps ``mode`` or ``default``."""
    if isinstance(target, ModeView):
        if mode is not None and mode != target.mode:
            raise DuckOSMError(f"this view is the mode {target.mode!r}, not {mode!r}")
        return target.network, target.mode
    if isinstance(target, Network):
        return target, target._one(mode)
    return target, mode or default


# ---- the network -----------------------------------------------------------------------------------------------------

class Network(os.PathLike):
    """A built duckOSM database with one or more modes. ``net.path`` is its file; ``os.fspath(net)`` and ``str(net)`` give
    the path, and a Network is equal to the ``Path`` of its file, so code that used the ``Path`` from ``duckosm.build``
    still works. ``net["walking"]`` or ``net.walking`` is the :class:`ModeView` of one mode.

    >>> net = duckosm.network("Monaco")
    >>> net.summary()
    >>> net.walking.map()"""

    def __init__(self, path: str | os.PathLike):
        self.path = Path(path)
        self._modes_at = None

    # ---- the path -----------------------------------------------------------------------------------------------------
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

    def __getitem__(self, mode: str) -> "ModeView":
        return ModeView(self, self._mode(mode))

    def __getattr__(self, name):
        """``net.walking``: the :class:`ModeView` of a mode. Else the attributes of the ``Path`` (``name``, ``parent``,
        ``exists()`` ...), as before ``build`` gave a Network."""
        if name.startswith("_") or name == "path":
            raise AttributeError(name)
        if self.path.is_file() and name in self.modes:
            return ModeView(self, name)
        return getattr(self.path, name)

    def __dir__(self):
        extra = self.modes if self.path.is_file() else []
        return sorted(set(super().__dir__()) | set(extra))

    # ---- what is in the file ------------------------------------------------------------------------------------------
    def _connect(self):
        """A read-only connection to the file, with the extension spatial loaded. The caller closes it."""
        import duckdb
        if not self.path.is_file():
            raise DuckOSMError(f"{self.path}: no such database file. Build it with duckosm.network or duckosm.build.")
        con = duckdb.connect(str(self.path), read_only=True)
        try:
            con.load_extension("spatial")
        except duckdb.Error:
            con.install_extension("spatial")
            con.load_extension("spatial")
        return con

    def sql(self, query: str, params: Any = None):
        """The result of a SQL query as a pandas DataFrame (read-only; the extension spatial is loaded). ``params``: the
        values of ``?`` or ``$name``. FLOAT columns come as float64 (44.2, not 44.200001).

        >>> net.sql("SELECT highway, count(*) AS n FROM driving.edges GROUP BY highway")"""
        con = self._connect()
        try:
            df = con.execute(query, params).df() if params is not None else con.sql(query).df()
        finally:
            con.close()
        for c in df.columns[df.dtypes == "float32"]:          # through the shortest text of the float32
            df[c] = df[c].to_numpy().astype(str).astype("float64")
        return df

    @property
    def modes(self) -> list[str]:
        """The modes in the file (the schemas with a road table ``edges``): driving, walking, cycling ... Not ``mm``: the
        multimodal graph joins the modes; its edges have no ``osm_id``."""
        at = self.path.stat().st_mtime if self.path.is_file() else None
        if self._modes_at is None or self._modes_at[0] != at:
            rows = self.sql("SELECT DISTINCT table_schema FROM information_schema.columns "
                            "WHERE table_name = 'edges' AND column_name = 'osm_id'").table_schema
            order = list(DEFAULT_MODES)
            self._modes_at = (at, sorted(rows, key=lambda m: (order.index(m) if m in order else len(order), m)))
        return list(self._modes_at[1])

    def tables(self) -> list[str]:
        """The tables of the file, as ``schema.table``.

        >>> net.tables()"""
        t = self.sql("SELECT table_schema || '.' || table_name AS t FROM information_schema.tables "
                     "WHERE table_schema NOT IN ('information_schema', 'pg_catalog') ORDER BY table_schema, table_name")
        return list(t.t)

    def columns(self, table: str = "driving.edges"):
        """The columns of a table (``schema.table``) and their types, as a DataFrame.

        >>> net.columns("walking.edges")"""
        if table not in self.tables():
            raise DuckOSMError(f"no table {table}; the tables: {', '.join(self.tables())}")
        schema, name = table.split(".", 1)
        return self.sql(f'DESCRIBE "{schema}"."{name}"')[["column_name", "column_type"]]

    def summary(self):
        """One row for each mode: ``mode``, ``edges``, ``nodes``, ``km`` and ``oneway_km`` (the km of the one-way edges).

        >>> net.summary()"""
        import pandas as pd
        rows = [self.sql(f"SELECT '{m}' AS mode, (SELECT count(*) FROM {m}.edges) AS edges, "
                         f"(SELECT count(*) FROM {m}.nodes) AS nodes, "
                         f"(SELECT round(sum(length_m) / 1000, 1) FROM {m}.edges) AS km, "
                         f"(SELECT round(coalesce(sum(length_m) FILTER (WHERE oneway), 0) / 1000, 1) FROM {m}.edges) AS oneway_km")
                for m in self.modes]
        return pd.concat(rows, ignore_index=True) if rows else pd.DataFrame(columns=["mode", "edges", "nodes", "km"])

    def _meta(self) -> dict:
        """The build time, the version and the fixes file of the build (``main.visualization_metadata``)."""
        if "main.visualization_metadata" not in self.tables():
            return {}
        m = self.sql("SELECT * FROM main.visualization_metadata")
        if not len(m):
            return {}
        out = {"built": str(m.built_at[0])[:19] if "built_at" in m else None,
               "version": m.duckosm_version[0] if "duckosm_version" in m else None, "fixes": None}
        if "build_config" in m and m.build_config[0]:
            import yaml
            try:
                out["fixes"] = (yaml.safe_load(m.build_config[0]) or {}).get("osm_overrides")
            except yaml.YAMLError:
                pass
        return out

    @property
    def boundary(self):
        """The boundary of the area as a GeoDataFrame (None when the file has none)."""
        if "main.boundary" not in self.tables():
            return None
        return self._frame("main.boundary", "geom", "true", None)

    # ---- the notebook view --------------------------------------------------------------------------------------------
    def __repr__(self) -> str:
        if not self.path.is_file():
            return f"Network({str(self.path)!r}, missing)"
        from duckosm._api import _size
        s = self.summary()
        modes = ", ".join(f"{r.mode} {r.edges} edges" for r in s.itertuples())
        return f"Network({str(self.path)!r}, {_size(self.path)}, {modes})"

    def _repr_html_(self) -> str:
        from html import escape
        from duckosm._api import _size
        if not self.path.is_file():
            return f"<b>Network</b> <code>{escape(str(self.path))}</code>: missing"
        meta = self._meta()
        facts = [f"<code>{escape(str(self.path))}</code>", _size(self.path)]
        if meta.get("built"):
            facts.append(f"built {escape(meta['built'])}")
        facts.append(f"fixes: <code>{escape(str(meta['fixes']))}</code>" if meta.get("fixes") else "no fixes file")
        return (f"<div><b>Network {escape(self.path.name.split('.')[0])}</b>: {', '.join(facts)}"
                f"{self.summary().to_html(index=False)}</div>")

    # ---- the data -----------------------------------------------------------------------------------------------------
    def _mode(self, mode):
        modes = self.modes
        if mode not in modes:
            raise DuckOSMError(f"no mode {mode!r} in {self.path}; the modes in the file: {', '.join(modes)}. "
                               f"Build it with modes=[..., {mode!r}].")
        return mode

    def _one(self, mode):
        """The one mode of a call that needs one: ``mode``, or the only mode of the file."""
        if mode is not None:
            return self._mode(mode)
        modes = self.modes
        if len(modes) == 1:
            return modes[0]
        raise DuckOSMError(f"{self.path} has the modes {', '.join(modes)}: choose one, for example "
                           f"net[{modes[0]!r}].route(...) or mode={modes[0]!r}")

    def _frame(self, table, geometry, where, crs):
        gpd = require("geopandas", "the GeoDataFrames of duckosm")
        df = self.sql(f"SELECT * EXCLUDE ({geometry}), ST_AsWKB({geometry}) AS _wkb FROM {table} WHERE {where}")
        gdf = gpd.GeoDataFrame(df, geometry=gpd.GeoSeries.from_wkb(df.pop("_wkb").map(bytes), crs="EPSG:4326"))
        if crs is None:
            return gdf
        return gdf.to_crs(gdf.estimate_utm_crs() if crs == "utm" else crs)

    def _each(self, mode, read):
        """``read(mode)`` for one mode; with ``mode=None`` and more modes, all of them, with a ``mode`` column."""
        import pandas as pd
        modes = self.modes
        if mode is not None or len(modes) == 1:
            return read(self._mode(mode or modes[0]))
        parts = [read(m).assign(mode=m) for m in modes]
        gdf = pd.concat(parts, ignore_index=True)
        return gdf.set_geometry(parts[0].geometry.name).set_crs(parts[0].crs, allow_override=True)

    def edges(self, mode: str | None = None, where: str | None = None, crs: Any = None):
        """The edges as a GeoDataFrame: all columns of ``<mode>.edges``, ``edge_id`` as ``int64``, floats as ``float64``,
        ordered by ``edge_id``. ``mode=None``: the one mode of the file, or all modes with a ``mode`` column. ``where``: a
        SQL filter on the columns; the text goes into the query as it is, so give only text that you trust. ``crs``: the
        CRS of the result (default EPSG:4326), for example ``3857`` or ``"utm"`` (the UTM zone of the area).

        >>> net.edges("driving", where="highway = 'primary'")"""
        return self._each(mode, lambda m: self._frame(f"{m}.edges", "geometry", f"({where or 'true'}) ORDER BY edge_id", crs))

    def nodes(self, mode: str | None = None, crs: Any = None):
        """The nodes (junctions) as a GeoDataFrame, ordered by ``node_id``; ``mode`` and ``crs`` as :meth:`edges`.

        >>> net.nodes("walking")"""
        return self._each(mode, lambda m: self._frame(f"{m}.nodes", "geom", "true ORDER BY node_id", crs))

    # ---- maps -------------------------------------------------------------------------------------------------------
    def map(self, mode: str | None = None, *, theme: str = "liberty", color_by=None, highlight=None, routes=None, **style):
        """The live roadstyle map in a notebook: all modes with the base map of the file (``mode=None``) or one mode.

        - ``theme``: a roadstyle theme (``"liberty"``, ``"osm"``, ``"google"``, ``"grey"``, ``"roadstyle"``).
        - ``color_by``: a column of the edges, or your own data: a Series keyed by ``edge_id``, or a DataFrame with
          ``edge_id`` and one value column. Numbers get a color ramp (``cmap=``); classes need ``colors={value: color}``.
        - ``highlight``: edge ids to draw in orange; the other edges are gray.
        - ``routes``: a route, a list of routes or ``{label: route}``: lines on top, each in its own color.
        - ``style``: other options of roadstyle.

        With ``DUCKOSM_STATIC_MAPS=1`` it draws :meth:`plot` in place of the live map (the saved outputs of the manual).

        >>> net.map()
        >>> net.map("driving", color_by="maxspeed_kmh", cmap="viridis")
        >>> net.map("walking", routes=net.walking.route_points(a, b))"""
        if os.environ.get("DUCKOSM_STATIC_MAPS") == "1":
            return self.plot(mode, color_by=color_by, highlight=highlight, routes=routes, **style)
        require("roadstyle", "Network.map")
        if mode is not None:
            self._mode(mode)
        if highlight is not None:
            style["color_table"] = {int(e): HIGHLIGHT for e in highlight}
        if color_by is None and routes is None:
            from roadstyle import duckosm as page
            return page.page(self.path, mode or "all", theme=theme, **style)
        # ponytail: roadstyle.duckosm.page reads its own road columns and takes no overlays yet (a roadstyle change):
        # until then a map with your own column or routes is roadstyle.render_edges on the edges, on its default base map
        import roadstyle as rs
        gdf, color_by = self._colored(mode, color_by, style)
        line = self._route_lines(routes)
        if line:
            style["overlays"] = [*style.get("overlays", []),
                                 *(rs.Overlay(g, kind="line", color=ROUTE_COLORS[i % len(ROUTE_COLORS)], width=5, label=k,
                                              popup=["route", "time_s", "length_m"]) for i, (k, g) in enumerate(line.items()))]
        if color_by is not None:
            style["color_by"] = color_by
        style.setdefault("name", self.path.name.split(".")[0])
        return rs.render_edges(gdf[[c for c in gdf.columns if c in MAP_COLUMNS or c == color_by or c == gdf.geometry.name]],
                               **style)

    def dashboard(self, mode: str | None = None, **style):
        """roadstyle's dashboard of the file: the map, the panel with the road card and the color menu (road class,
        modes), docked.

        With ``DUCKOSM_STATIC_MAPS=1`` it draws :meth:`plot` in its place.

        >>> net.dashboard()"""
        if os.environ.get("DUCKOSM_STATIC_MAPS") == "1":
            return self.plot(mode)
        require("roadstyle", "Network.dashboard")
        from roadstyle import duckosm as page
        return page.page(self.path, "all" if mode is None else self._mode(mode), dashboard=True, **style)

    def save_map(self, path: str | os.PathLike, mode: str | None = None, *, dashboard: bool = False, **options) -> Path:
        """The map of :meth:`map` (or of :meth:`dashboard`) as one offline HTML file, with the same options. Returns the path.

        >>> net.save_map("monaco.html")"""
        Path(path).parent.mkdir(parents=True, exist_ok=True)
        static = os.environ.pop("DUCKOSM_STATIC_MAPS", None)       # a file is always the live map
        try:
            m = self.dashboard(mode, **options) if dashboard else self.map(mode, **options)
            m.save(str(path))
        finally:
            if static is not None:
                os.environ["DUCKOSM_STATIC_MAPS"] = static
        return Path(path)

    def plot(self, mode: str | None = None, *, color_by=None, highlight=None, routes=None, ax=None, **style):
        """A static matplotlib picture (for papers and saved notebook outputs), with the options of :meth:`map`. All modes:
        one color for each mode; one mode: one color for each road class. Returns the axis.

        >>> net.plot("driving", color_by="maxspeed_kmh")"""
        require("matplotlib", "Network.plot")
        import matplotlib.pyplot as plt
        from matplotlib.lines import Line2D
        gdf, color_by = self._colored(mode, color_by, style)
        line = self._route_lines(routes)
        own_axis = ax is None
        if own_axis:
            _, ax = plt.subplots(figsize=(7, 7), dpi=72)
        handles, colors = [], style.get("colors")
        plain = highlight is not None or bool(line)
        if colors:                                   # classes: gray edges, then each class in its color
            gdf.plot(ax=ax, color="#cccccc", linewidth=0.5)
            for value, color in colors.items():
                gdf[gdf[color_by] == value].plot(ax=ax, color=color, linewidth=3)
                handles.append(Line2D([], [], color=color, linewidth=3, label=value))
        elif color_by is not None:                   # numbers: a color ramp, gray where there is no value
            gdf.plot(ax=ax, column=color_by, cmap=style.get("cmap", "viridis"), linewidth=1.2, legend=True,
                     legend_kwds={"label": color_by, "shrink": 0.6}, missing_kwds={"color": "#cccccc", "linewidth": 0.5})
        elif "mode" in gdf and not plain:            # all modes: one color for each
            for m, part in gdf.groupby("mode", sort=False):
                part.plot(ax=ax, color=MODE_COLORS.get(m, "#555555"), linewidth=0.7)
                handles.append(Line2D([], [], color=MODE_COLORS.get(m, "#555555"), linewidth=2, label=m))
        elif "highway" in gdf and not plain:         # one mode: the road classes
            gdf.plot(ax=ax, column="highway", cmap="tab20", linewidth=0.8, legend=True,
                     legend_kwds={"loc": "lower right", "fontsize": 7})
        else:
            gdf.plot(ax=ax, color="#bbbbbb", linewidth=0.6)
        if highlight is not None:
            gdf[gdf.edge_id.isin(list(highlight))].plot(ax=ax, color=HIGHLIGHT, linewidth=3)
        for i, (label, g) in enumerate(line.items()):
            color = ROUTE_COLORS[i % len(ROUTE_COLORS)]
            g.plot(ax=ax, color=color, linewidth=3.5)
            handles.append(Line2D([], [], color=color, linewidth=3.5, label=label))
        if handles:
            ax.legend(handles=handles, loc="lower right")
        ax.set_axis_off()
        if own_axis:
            plt.show()
        return ax if not own_axis else None

    def _colored(self, mode, color_by, style):
        """The edges to draw, and the column to color by: a column of the edges, or your own data joined on ``edge_id``."""
        import pandas as pd
        gdf = self.edges(mode)
        if "mode" in gdf and mode is None:           # all modes on one map: each road once, with its first mode
            gdf = gdf.drop_duplicates("edge_id", keep="first")
        if color_by is None:
            return gdf, None
        if isinstance(color_by, pd.Series):
            name = color_by.name or "value"
            gdf = gdf.merge(color_by.rename(name).rename_axis("edge_id").reset_index(), on="edge_id", how="left")
            color_by = name
        elif isinstance(color_by, pd.DataFrame):
            values = [c for c in color_by.columns if c != "edge_id"]
            if "edge_id" not in color_by or len(values) != 1:
                raise DuckOSMError("color_by as a DataFrame needs the column edge_id and one value column")
            gdf, color_by = gdf.merge(color_by[["edge_id", values[0]]], on="edge_id", how="left"), values[0]
        if color_by not in gdf.columns:
            raise DuckOSMError(f"no column {color_by!r} to color by; the columns: {', '.join(map(str, gdf.columns))}")
        if "colors" not in style and gdf[color_by].dtype.kind not in "iuf":
            raise DuckOSMError(f"{color_by!r} has classes: give colors={{value: color}}, or color by a number column")
        return gdf, color_by

    def _route_lines(self, routes) -> dict:
        """``{label: GeoDataFrame of one line}`` of the routes; the edge geometries come from the file."""
        if routes is None:
            return {}
        if isinstance(routes, dict) and "edges" in routes:
            routes = [routes]
        items = routes.items() if isinstance(routes, dict) else ((f"route {i + 1}", r) for i, r in enumerate(routes))
        gpd = require("geopandas", "the routes of a map")
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

    # ---- analysis, routes, graphs and exports: the functions of the modules -----------------------------------------
    def stats(self, mode: str | None = None, area_m2: float | None = None):
        """:func:`duckosm.analysis.stats`: the numbers of OSMnx ``basic_stats`` in two views (``osmnx``, ``duckosm``).

        >>> net.driving.stats()"""
        from duckosm import analysis
        return analysis.stats(self, area_m2=area_m2, mode=mode)

    def route(self, a, b, mode: str | None = None, weight: str = "time") -> dict | None:
        """The fastest route (``weight="length"``: the shortest) from ``a`` to ``b``: two edge ids, or two ``(lon, lat)``
        points. :func:`duckosm.routing.route`.

        >>> r = net.walking.route((7.4155, 43.7285), (7.4335, 43.7468))"""
        from duckosm import routing
        return routing.route(self, a, b, mode=mode, weight=weight)

    def route_points(self, a: tuple, b: tuple, mode: str | None = None, **kwargs) -> dict | None:
        """:func:`duckosm.routing.route_points`: the route between two ``(lon, lat)`` points.

        >>> r = net.driving.route_points((7.4155, 43.7285), (7.4335, 43.7468))"""
        from duckosm import routing
        return routing.route_points(self, a, b, mode=mode, **kwargs)

    def route_multimodal_points(self, a: tuple, b: tuple, **kwargs) -> dict:
        """:func:`duckosm.routing.route_multimodal_points`: a route across the modes (needs the multimodal graph ``mm``).

        >>> m = net.route_multimodal_points((7.4155, 43.7285), (7.4335, 43.7468))"""
        from duckosm import routing
        return routing.route_multimodal_points(self, a, b, **kwargs)

    def directions(self, route: dict, mode: str | None = None) -> list[dict]:
        """:func:`duckosm.routing.directions`: the turn-by-turn steps of a route.

        >>> for step in net.driving.directions(r): print(step["text"])"""
        from duckosm import routing
        return routing.directions(self, route, mode=mode)

    def to_networkx(self, mode: str | None = None, **kwargs):
        """:func:`duckosm.ml.to_networkx`: the turn graph of one mode (a node is an ``edge_id``).

        >>> g = net.driving.to_networkx()"""
        from duckosm import ml
        return ml.to_networkx(self, mode=mode, **kwargs)

    def to_rustworkx(self, mode: str | None = None, **kwargs):
        """:func:`duckosm.ml.to_rustworkx`: the graph of one mode as a rustworkx graph.

        >>> h = net.driving.to_rustworkx()"""
        from duckosm import ml
        return ml.to_rustworkx(self, mode=mode, **kwargs)

    def to_sumo(self, out_dir, mode: str | None = None, progress: Any = None, **kwargs) -> dict:
        """:func:`duckosm.sim.to_sumo`: the SUMO network of one mode.

        >>> net.driving.to_sumo("out/sumo")"""
        from duckosm import sim
        from duckosm._progress import timed
        return timed(progress, f"SUMO {out_dir}", lambda: sim.to_sumo(self, out_dir, mode=mode, **kwargs))

    def to_matsim(self, out_path, mode: str | None = None, progress: Any = None, **kwargs) -> dict:
        """:func:`duckosm.sim.to_matsim`: the MATSim network.

        >>> net.driving.to_matsim("out/network.xml.gz")"""
        from duckosm import sim
        from duckosm._progress import timed
        return timed(progress, f"MATSim {out_path}", lambda: sim.to_matsim(self, out_path, mode=mode, **kwargs))

    def to_gmns(self, out_path, mode: str | None = None, progress: Any = None, **kwargs) -> dict:
        """:func:`duckosm.planning.to_gmns`: the GMNS network (``mode=None``: all modes).

        >>> net.to_gmns("out/gmns.duckdb", to_csv="out/gmns")"""
        from duckosm import planning
        from duckosm._progress import timed
        return timed(progress, f"GMNS {out_path}", lambda: planning.to_gmns(self, out_path, mode=mode, **kwargs))

    def to_opendrive(self, out_path, mode: str | None = None, progress: Any = None, **kwargs) -> dict:
        """:func:`duckosm.hdmap.to_opendrive`: the OpenDRIVE file (experimental).

        >>> net.driving.to_opendrive("out/monaco.xodr")"""
        from duckosm import hdmap
        from duckosm._progress import timed
        return timed(progress, f"OpenDRIVE {out_path}", lambda: hdmap.to_opendrive(self, out_path, mode=mode, **kwargs))

    def to_gis(self, out, mode: str | None = None, progress: Any = None, **kwargs) -> dict:
        """:func:`duckosm.gis.to_gis`: a GeoPackage, GeoParquet or shapefiles (``mode=None``: all modes).

        >>> net.to_gis("out/monaco.gpkg")"""
        from duckosm import gis
        from duckosm._progress import timed
        return timed(progress, f"GIS {out}", lambda: gis.to_gis(self, out, mode=mode, **kwargs))


# ---- one mode ----------------------------------------------------------------------------------------------------------

class ModeView:
    """One mode of a :class:`Network`: the same calls with no ``mode=``. ``net["walking"]`` or ``net.walking``.

    >>> walk = net.walking
    >>> walk.edges(), walk.map(), walk.route((7.4155, 43.7285), (7.4335, 43.7468))"""

    _CALLS = ("edges", "nodes", "map", "plot", "dashboard", "stats", "route", "route_points", "directions", "to_networkx",
              "to_rustworkx", "to_sumo", "to_matsim", "to_gmns", "to_opendrive", "to_gis")

    def __init__(self, network: Network, mode: str):
        self.network, self.mode = network, mode

    def __getattr__(self, name):
        if name in ModeView._CALLS:
            call = getattr(self.network, name)

            def bound(*args, **kwargs):
                return call(*args, mode=self.mode, **kwargs)
            bound.__doc__ = call.__doc__
            bound.__name__ = name
            return bound
        raise AttributeError(f"ModeView has no attribute {name!r}")

    def __dir__(self):
        return sorted(set(super().__dir__()) | set(ModeView._CALLS))

    def save_map(self, path, **options) -> Path:
        """:meth:`Network.save_map` of this mode."""
        return self.network.save_map(path, self.mode, **options)

    def summary(self):
        """The row of this mode of :meth:`Network.summary`."""
        s = self.network.summary()
        return s[s["mode"] == self.mode].reset_index(drop=True)

    def __repr__(self):
        r = self.summary().iloc[0]
        return f"ModeView({str(self.network.path)!r}, {self.mode}: {r.edges} edges, {r.nodes} nodes, {r.km} km)"

    def _repr_html_(self):
        from html import escape
        return (f"<div><b>{escape(self.mode)}</b> of <code>{escape(str(self.network.path))}</code>"
                f"{self.summary().to_html(index=False)}</div>")


# ---- duckosm.network is a function and a module -------------------------------------------------------------------------

class _CallableModule(type(os)):
    """When a module imports ``duckosm.network`` (the module), Python sets ``duckosm.network`` to the module, not to the
    function :func:`network`. A call of the module calls the function, so ``duckosm.network("Monaco")`` always works."""

    def __call__(self, *args, **kwargs) -> Network:
        return network(*args, **kwargs)


import sys  # noqa: E402

sys.modules[__name__].__class__ = _CallableModule
