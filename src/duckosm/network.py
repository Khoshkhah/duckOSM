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

from duckosm._progress import shown
from duckosm.errors import DuckOSMError, require

HIGHLIGHT = "#e8590c"
MAP_SHOWS = ("live", "link", "picture", "plot")
# the base-map layers that plot draws: roadstyle's layer name -> (the table in features, its color)
PLOT_LAYERS = {"ocean": ("ocean", "#aad3df"), "landcover": ("land", "#dde8cf"), "water": ("water_polygons", "#aad3df"),
               "waterways": ("water_lines", "#7fb6cc"), "buildings": ("buildings", "#d9d0c9")}
_MAP_COUNT = 0          # the <n> of the default file names of show="link" and show="picture"


def _modes_label(modes):
    """The dashboard's name of the modes that can use a road: "all modes", "driving + walking", ... or "driving"."""
    if set(DEFAULT_MODES) <= modes:
        return "all modes"
    return " + ".join(m for m in DEFAULT_MODES if m in modes) or " + ".join(sorted(modes))


def _show(show):
    show = show or os.environ.get("DUCKOSM_MAP_SHOW") or "live"
    if show not in MAP_SHOWS:
        raise DuckOSMError(f"show must be one of {', '.join(MAP_SHOWS)}, got {show!r}")
    return show


class MapLink(os.PathLike):
    """A saved live map: a link in a notebook, the path of the HTML file everywhere else."""

    def __init__(self, path: Path):
        self.path = path

    def __fspath__(self):
        return str(self.path)

    def __repr__(self):
        return f"MapLink({str(self.path)!r})"

    def _repr_html_(self):
        mb = self.path.stat().st_size / 1e6
        return f'<a href="{self.path.as_posix()}" target="_blank">Open the live map</a> ({mb:.1f} MB)'

    def _repr_markdown_(self):
        return f"[Open the live map]({self.path.as_posix()}) ({self.path.stat().st_size / 1e6:.1f} MB)"


class MapPicture(os.PathLike):
    """A PNG of a live map: the picture in a notebook, the path of the PNG file everywhere else."""

    def __init__(self, path: Path):
        self.path = path

    def __fspath__(self):
        return str(self.path)

    def __repr__(self):
        return f"MapPicture({str(self.path)!r})"

    def _repr_png_(self):
        return self.path.read_bytes()
ROUTE_COLORS = ("#e8590c", "#1c7ed6", "#2f9e44", "#ae3ec9", "#f08c00")
# the colors of the modes that can use a road: the "Modes" colors of roadstyle's dashboard (roadstyle
# data/basemap/modes.yaml, mode_colors), so that plot and the dashboard agree
MODES_COLORS = {"all modes": "#555555", "driving + walking": "#e67e22", "driving + cycling": "#9b59b6",
                "walking + cycling": "#16a085", "driving": "#e74c3c", "walking": "#27ae60", "cycling": "#2b6cb0"}
MAP_COLUMNS = ("edge_id", "osm_id", "name", "highway", "oneway", "lanes", "length_m", "maxspeed_kmh", "layer", "bridge",
               "tunnel", "mode")
DEFAULT_MODES = ("driving", "walking", "cycling")


# ---- get a network ---------------------------------------------------------------------------------------------------

def open(path: str | os.PathLike) -> "Network":  # noqa: A001 -- duckosm.open, the name of the SDK
    """The :class:`Network` of a built ``.duckdb`` file.

    >>> net = duckosm.open("data/monaco.duckdb")"""
    path = Path(path)
    if not path.is_file():
        raise DuckOSMError(f"{shown(path)}: no such database file. Build one with duckosm.network(\"Monaco\") or "
                           f"duckosm.build(pbf=..., output=\"{shown(path)}\").")
    return Network(path)


def network(place: str | None = None, *, modes: Iterable[str] | None = DEFAULT_MODES, fixes: str | os.PathLike | None = None,
            out: str | os.PathLike | None = None, name: str | None = None, pbf: str | os.PathLike | None = None,
            path: str | os.PathLike | None = None,
            progress: Any = None, **area) -> "Network":
    """Get the OSM file of an area and build its network, in one call. The download stays in a cache.

    The area: ``place`` (a name), or one of ``bbox=(west, south, east, north)``, ``point=(lon, lat)`` with ``dist_m=``,
    ``osm_id=``, ``h3_cell=``, ``boundary=`` (a GeoJSON file, a dict or a GeoDataFrame), or ``pbf=`` (an ``.osm.pbf``
    file of your own; ``boundary=`` cuts it, without it the whole file is the area). ``modes``: the networks (default
    driving, walking and cycling). ``fixes``: an OSM fixes file for the cut and the build. The file is
    The file: ``path`` (for example ``"monaco.duckdb"``) when you give it; else a new folder in duckOSM's work
    folder (:func:`duckosm.work_dir`), and ``net.save(path)`` keeps it. ``out`` and ``name``: the old way,
    ``<out>/<name>.duckdb``. The build options of :func:`duckosm.build` work here too: ``config=`` (a YAML or JSON file,
    or a dict), ``speeds``, ``costs``, ``restrictions``, ``bus_routes``, ``connect_paths``, ``snap_m``, ``dem``,
    ``features`` and ``levels``. ``progress``: as :func:`duckosm.build`.

    >>> net = duckosm.network("Monaco")
    >>> net = duckosm.network(point=(7.4246, 43.7384), dist_m=500, modes=["walking"])
    >>> net = duckosm.network(pbf="my.osm.pbf", boundary="area.geojson", bus_routes=False)"""
    from duckosm._api import build, get
    from duckosm._api import BUILD_OPTIONS
    modes = None if modes is None else list(modes)
    built = {k: area.pop(k) for k in list(area) if k in BUILD_OPTIONS or k in ("config", "dem", "features", "levels")}
    area_args = ("osm_id", "bbox", "h3_cell", "boundary", "point", "dist_m", "cache", "refresh")
    for k in area:                   # a wrong name: the nearest of the area arguments and the build options
        if k not in area_args:
            import difflib
            names = [*area_args, *BUILD_OPTIONS, "config", "dem", "features", "levels"]
            near = difflib.get_close_matches(k, names, n=1)
            raise DuckOSMError(f"network has no argument {k!r}" + (f": did you mean {near[0]!r}?" if near else "") +
                               f" The area: {', '.join(area_args)}. The build options: {', '.join(names[len(area_args):])}.")
    if pbf is not None:
        stem = name or Path(pbf).name.split(".")[0]
        output = Path(path) if path else Path(out) / f"{stem}.duckdb" if out else None
        return build(pbf=pbf, boundary=area.pop("boundary", None), modes=modes, output=output,
                     fixes=fixes, progress=progress, **area, **built)
    got = get(place, out=out, name=name, fixes=fixes, progress=progress, **area)
    return got.build(modes=modes, output=Path(path) if path else None, fixes=fixes, progress=progress, **built)


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
    def _connect(self, own: bool = False):
        """A read-only connection to the file, with the extension spatial loaded. The caller closes it. ``own``: the
        connection has its own DuckDB instance (the file is attached to an in-memory database), so a result that the
        caller does not read to the end does not lock the shared instance of the other calls."""
        import duckdb
        if not self.path.is_file():
            raise DuckOSMError(f"{shown(self.path)}: no such database file. Build it with duckosm.network or duckosm.build.")
        if own:
            con = duckdb.connect()
            name = self.path.name.split(".")[0].replace('"', '""')
            con.execute(f"ATTACH '{str(self.path).replace(chr(39), chr(39) * 2)}' AS \"{name}\" (READ_ONLY); USE \"{name}\"")
        else:
            con = duckdb.connect(str(self.path), read_only=True)
        try:
            con.load_extension("spatial")
        except duckdb.Error:
            con.install_extension("spatial")
            con.load_extension("spatial")
        return con

    def connect(self):
        """A read-only DuckDB connection to the file, with the extension spatial loaded: for Arrow
        (``.to_arrow_table()``), Polars (``.pl()``) or DuckDB relations. Close it after use, or use it in a ``with``
        block. In DuckDB 1.5, ``.arrow()`` gives a lazy ``RecordBatchReader``, not a table. The connection has its own
        DuckDB instance, so a reader that you do not read to the end does not stop the other calls of ``net``.

        >>> with net.connect() as con:
        ...     t = con.sql("SELECT * FROM driving.edges").to_arrow_table()"""
        return self._connect(own=True)

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

    def schema(self, schema: str | None = None):
        """One row for each table of the file: ``schema``, ``table``, ``rows``, ``columns`` and ``description`` (the
        comment that the build writes; empty in a file of an older duckOSM). ``schema``: only the tables of that schema,
        for example ``"driving"`` or ``"main"``.

        >>> net.schema()
        >>> net.schema("driving")"""
        where = "" if schema is None else " AND schema_name = $schema"
        df = self.sql("SELECT schema_name AS schema, table_name AS \"table\", column_count AS columns, "
                      "coalesce(comment, '') AS description FROM duckdb_tables() "
                      f"WHERE database_name = current_database(){where} ORDER BY schema, \"table\"",
                      None if schema is None else {"schema": schema})
        if schema is not None and not len(df):
            raise DuckOSMError(f"no schema {schema!r} in {shown(self.path)}; the schemas: "
                               f"{', '.join(sorted({t.split('.')[0] for t in self.tables()}))}")
        if len(df):                                  # exact counts: duckdb_tables().estimated_size is an estimate
            counts = self.sql(" UNION ALL ".join(f"SELECT {i} AS i, count(*) AS n FROM \"{s}\".\"{n}\""
                                                 for i, (s, n) in enumerate(zip(df.schema, df.table))))
            df.insert(2, "rows", counts.sort_values("i").n.to_numpy())
        return df

    def _table(self, table):
        """``(schema, name)`` of ``schema.table``, or an error that names the tables of the file."""
        if table not in self.tables():
            raise DuckOSMError(f"no table {table}; the tables: {', '.join(self.tables())}")
        return table.split(".", 1)

    def columns(self, table: str = "driving.edges"):
        """The columns of a table (``schema.table``): ``column_name``, ``column_type`` and ``description`` (the comment
        that the build writes; empty in a file of an older duckOSM), as a DataFrame.

        >>> net.columns("walking.edges")"""
        schema, name = self._table(table)
        return self.sql("SELECT column_name, data_type AS column_type, coalesce(comment, '') AS description "
                        "FROM duckdb_columns() WHERE database_name = current_database() AND schema_name = $s "
                        "AND table_name = $t ORDER BY column_index", {"s": schema, "t": name})

    def summarize(self, table: str = "driving.edges"):
        """DuckDB's ``SUMMARIZE`` of a table: for each column the type, the minimum, the maximum, the number of distinct
        values, the average, the quartiles, the count and the percentage of NULL values. Geometry columns are left out.

        >>> net.summarize("driving.edges")"""
        schema, name = self._table(table)
        cols = self.columns(table)
        keep = [c for c, ty in zip(cols.column_name, cols.column_type) if not ty.upper().startswith("GEOMETRY")]
        return self.sql(f'SUMMARIZE SELECT {", ".join(chr(34) + c + chr(34) for c in keep)} FROM "{schema}"."{name}"')

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
            return f"Network({shown(self.path)!r}, missing)"
        from duckosm._api import _size
        s = self.summary()
        modes = ", ".join(f"{r.mode} {r.edges} edges" for r in s.itertuples())
        return f"Network({shown(self.path)!r}, {_size(self.path)}, {modes})"

    def _repr_html_(self) -> str:
        from html import escape
        from duckosm._api import _size
        if not self.path.is_file():
            return f"<b>Network</b> <code>{escape(shown(self.path))}</code>: missing"
        meta = self._meta()
        fixes = f"<code>{escape(str(meta['fixes']))}</code>" if meta.get("fixes") else "none"
        saved = "yes" if self.saved else 'no, it is in the work folder: <code>net.save("name.duckdb")</code> keeps it'
        rows = "".join(f"<tr><td>{k}</td><td>{v}</td></tr>" for k, v in (
            ("file", f"<code>{escape(shown(self.path))}</code> ({_size(self.path)})"), ("fixes", fixes), ("saved", saved)))
        return (f"<div><b>Network {escape(self.path.name.split('.')[0])}</b><table>{rows}</table>"
                f"{self.summary().to_html(index=False)}</div>")

    # ---- the data -----------------------------------------------------------------------------------------------------
    def _mode(self, mode):
        modes = self.modes
        if mode not in modes:
            raise DuckOSMError(f"no mode {mode!r} in {shown(self.path)}; the modes in the file: {', '.join(modes)}. "
                               f"Build it with modes=[..., {mode!r}].")
        return mode

    def _one(self, mode):
        """The one mode of a call that needs one: ``mode``, or the only mode of the file."""
        if mode is not None:
            return self._mode(mode)
        modes = self.modes
        if len(modes) == 1:
            return modes[0]
        raise DuckOSMError(f"{shown(self.path)} has the modes {', '.join(modes)}: choose one, for example "
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
    def map(self, mode: str | None = None, *, show: str | None = None, path=None, layers=None, theme: str = "liberty",
            color_by=None, highlight=None, routes=None, **style):
        """The roadstyle map of the file: all modes with the base map of the file (``mode=None``) or one mode.

        - ``show``: how the notebook shows the map (default: ``DUCKOSM_MAP_SHOW``, else ``"live"``):

          - ``"live"``: the live map in the output (about 6 MB for Monaco);
          - ``"link"``: the live map saved as an HTML file (``path``, default ``maps/<file>-<n>.html``) and a link to it;
          - ``"picture"``: a PNG of the live map (``path``, default ``maps/<file>-<n>.png``), taken in headless Chromium
            (needs ``pip install playwright`` and ``playwright install chromium``);
          - ``"plot"``: :meth:`plot`, a matplotlib picture with no roadstyle.

        - ``layers``: the base map of the file under the roads: True (all layers), False (the roads only) or a list of
          roadstyle layer names, for example ``["water", "buildings"]``. Default: all layers on the live map
          (``"live"``, ``"link"``, ``"picture"``), the roads only with ``"plot"``. The live map's layers button turns them on
          and off. With ``color_by`` or ``routes`` the live map has roadstyle's default base map, and ``layers`` has no
          effect (a roadstyle change).

        - ``theme``: a roadstyle theme (``"liberty"``, ``"osm"``, ``"google"``, ``"grey"``, ``"roadstyle"``).
        - ``color_by``: a column of the edges, or your own data: a Series keyed by ``edge_id``, or a DataFrame with
          ``edge_id`` and one value column. Numbers get a color ramp (``cmap=``); classes need ``colors={value: color}``.
        - ``highlight``: edge ids to draw in orange; the other edges are gray.
        - ``routes``: a route, a list of routes or ``{label: route}``: lines on top, each in its own color.
        - ``style``: other options of roadstyle.

        >>> net.map()
        >>> net.map("driving", color_by="maxspeed_kmh", cmap="viridis", show="picture")
        >>> net.map("walking", routes=net.walking.route_points(a, b), show="link")"""
        show = _show(show)
        if show == "plot":
            return self.plot(mode, layers=bool(layers) and layers, color_by=color_by, highlight=highlight, routes=routes,
                             **style)
        return self._present(self._live_map(mode, theme, color_by, highlight, routes,
                                            {"layers": True if layers is None else layers, **style}), show, path)

    def _live_map(self, mode, theme, color_by, highlight, routes, style):
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
        style.pop("layers", None)
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

    def dashboard(self, mode: str | None = None, *, show: str | None = None, path=None, layers=None, color_by=None,
                  highlight=None, routes=None, **style):
        """roadstyle's dashboard of the file: the map, the panel with the road card and the color menu (road class,
        modes), docked. ``show``, ``path``, ``layers``, ``color_by``, ``highlight`` and ``routes``: as in :meth:`map`.
        With ``color_by``, the roads have the colors of your data, and the road card of a clicked road shows its value.

        >>> net.dashboard()
        >>> net.dashboard("driving", color_by=counts.set_index("edge_id").vehicles_per_hour, cmap="viridis")"""
        show = _show(show)
        if show == "plot":
            return self.plot(mode, layers=bool(layers) and layers, color_by=color_by, highlight=highlight, routes=routes,
                             **style)
        return self._present(self._live_dashboard(mode, layers, color_by, highlight, routes, style), show, path)

    def _live_dashboard(self, mode, layers, color_by, highlight, routes, style):
        require("roadstyle", "Network.dashboard")
        if color_by is not None or routes is not None:
            # your data or routes: the map of map(), in the dashboard template (as roadstyle.duckosm.page does)
            style = {"name": f"{self.path.name.split('.')[0]}: dashboard", "tooltip": None, "template": "dashboard",
                     "layout": "dock", "dashboard": {"panel": {"sections": [{"id": "road", "part": "road_card"}]}},
                     **style}
            return self._live_map(mode, "liberty", color_by, highlight, routes, style)
        from roadstyle import duckosm as page
        style = {"layers": True if layers is None else layers, **style}
        if highlight is not None:
            style["color_table"] = {int(e): HIGHLIGHT for e in highlight}
        return page.page(self.path, "all" if mode is None else self._mode(mode), dashboard=True, **style)

    def _present(self, live, show, path):
        """The live map as ``show`` asks: itself, a link to its saved HTML file or a PNG of it."""
        if show == "live":
            return live
        global _MAP_COUNT
        _MAP_COUNT += 1
        out = Path(path or f"maps/{self.path.name.split('.')[0]}-{_MAP_COUNT}.{'html' if show == 'link' else 'png'}")
        out.parent.mkdir(parents=True, exist_ok=True)
        if show == "link":
            live.save(str(out))
            return MapLink(out)
        require("playwright", "show='picture'")
        import roadstyle as rs
        rs.snapshot(live, out, controls=False)      # the map only: a picture cannot use the toolbox
        return MapPicture(out)

    def save_map(self, path: str | os.PathLike, mode: str | None = None, *, dashboard: bool = False, **options) -> Path:
        """The map of :meth:`map` (or of :meth:`dashboard`) as one offline HTML file, with the same options. Returns the path.

        >>> net.save_map("monaco.html")"""
        Path(path).parent.mkdir(parents=True, exist_ok=True)
        if dashboard:
            m = self._live_dashboard(mode, options.pop("layers", None), options.pop("color_by", None),
                                     options.pop("highlight", None), options.pop("routes", None), options)
        else:
            theme = options.pop("theme", "liberty")
            m = self._live_map(mode, theme, options.pop("color_by", None), options.pop("highlight", None),
                               options.pop("routes", None), options)
        m.save(str(path))
        return Path(path)

    def plot(self, mode: str | None = None, *, layers=False, color_by=None, highlight=None, routes=None, ax=None, **style):
        """A static matplotlib picture (for papers and saved notebook outputs), with the options of :meth:`map`. All modes:
        one color for each set of modes that can use a road (``"all modes"``, ``"driving + walking"``, ... ``"cycling"``),
        the colors of the "Modes" option of :meth:`dashboard`; one mode: one color for each road class. Returns the axis.

        ``layers``: the base map of the file under the roads: False (the default: the roads only), True (the layers below)
        or a list of these names:
        ``ocean``, ``landcover``, ``water``, ``waterways``, ``buildings`` (the names of roadstyle's layers).

        >>> net.plot("driving", color_by="maxspeed_kmh")"""
        require("matplotlib", "Network.plot")
        import matplotlib.pyplot as plt
        from matplotlib.lines import Line2D
        gdf, color_by = self._colored(mode, color_by, style)
        line = self._route_lines(routes)
        own_axis = ax is None
        if own_axis:
            _, ax = plt.subplots(figsize=(7, 7), dpi=72)
        x0, y0, x1, y1 = gdf.total_bounds            # the view: the roads and a small margin, as the live map
        pad = 0.03 * max(x1 - x0, y1 - y0)
        view = (x0 - pad, y0 - pad, x1 + pad, y1 + pad)
        self._plot_layers(ax, layers, view)
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
        elif "modes" in gdf and not plain:           # all modes: one color for each set of modes, as the dashboard
            for label, color in MODES_COLORS.items():
                part = gdf[gdf.modes == label]
                if len(part):
                    part.plot(ax=ax, color=color, linewidth=0.7)
                    handles.append(Line2D([], [], color=color, linewidth=2, label=label))
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
        ax.set_xlim(view[0], view[2])
        ax.set_ylim(view[1], view[3])
        ax.set_axis_off()
        if own_axis:
            plt.show()
        return ax if not own_axis else None

    def _plot_layers(self, ax, layers, view):
        """Draws the base-map layers of ``features.*`` on ``ax``, in roadstyle's draw order, cut to ``view``
        (``x0, y0, x1, y1``): the ocean of a file can reach far past the roads."""
        if layers is False or layers is None:
            return
        names = list(PLOT_LAYERS) if layers is True else list(layers)
        wrong = [n for n in names if n not in PLOT_LAYERS]
        if wrong:
            raise DuckOSMError(f"plot draws the layers {', '.join(PLOT_LAYERS)}, not {', '.join(wrong)}; "
                               "the live map (show='live' or 'picture') draws all of roadstyle's layers")
        gpd = require("geopandas", "Network.plot")
        have = set(self.tables())
        for name in PLOT_LAYERS:                     # draw order: ocean, land, water, buildings
            table, color = PLOT_LAYERS[name]
            if name in names and f"features.{table}" in have:
                box = "ST_MakeEnvelope({}, {}, {}, {})".format(*(float(v) for v in view))
                df = self.sql(f"SELECT ST_AsWKB(ST_Intersection(geom, {box})) AS wkb FROM features.{table} "
                              f"WHERE ST_Intersects(geom, {box})")
                gdf = gpd.GeoDataFrame(geometry=gpd.GeoSeries.from_wkb(df.wkb.map(bytes), crs="EPSG:4326"))
                if len(gdf):
                    kw = {"linewidth": 0.6} if name == "waterways" else {"linewidth": 0}
                    gdf.plot(ax=ax, color=color, zorder=0, **kw)

    def _colored(self, mode, color_by, style):
        """The edges to draw, and the column to color by: a column of the edges, or your own data joined on ``edge_id``."""
        import pandas as pd
        gdf = self.edges(mode)
        if "mode" in gdf and mode is None:           # all modes on one map: each road once, with all its modes
            used = gdf.groupby("edge_id")["mode"].agg(frozenset)
            gdf = gdf.drop_duplicates("edge_id", keep="first").copy()
            gdf["modes"] = gdf.edge_id.map(used).map(_modes_label)
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
        """The fastest route (``weight="length"``: the shortest) from ``a`` to ``b``: edge ids, ``(lon, lat)`` points or
        place names. A :class:`duckosm.routing.Route`. :func:`duckosm.routing.route`.

        >>> r = net.walking.route((7.4155, 43.7285), (7.4335, 43.7468))"""
        from duckosm import routing
        return routing.route(self, a, b, mode=mode, weight=weight)

    def route_points(self, a: tuple, b: tuple, mode: str | None = None, **kwargs) -> dict | None:
        """:func:`duckosm.routing.route_points`: the route between two ``(lon, lat)`` points.

        >>> r = net.driving.route_points((7.4155, 43.7285), (7.4335, 43.7468))"""
        from duckosm import routing
        return routing.route_points(self, a, b, mode=mode, **kwargs)

    def routes(self, origins, destinations, mode: str | None = None, **kwargs) -> list:
        """:func:`duckosm.routing.routes`: many routes, in parallel.

        >>> rs = net.driving.routes([e1, e2], [e3, e4])"""
        from duckosm import routing
        return routing.routes(self, origins, destinations, mode=mode, **kwargs)

    def k_shortest_paths(self, a, b, k: int = 3, mode: str | None = None, **kwargs) -> list:
        """:func:`duckosm.routing.k_shortest_paths`: the ``k`` best routes (Yen).

        >>> rs = net.driving.k_shortest_paths(e1, e2, k=3)"""
        from duckosm import routing
        return routing.k_shortest_paths(self, a, b, k=k, mode=mode, **kwargs)

    def matrix(self, origins, destinations=None, mode: str | None = None, **kwargs):
        """:func:`duckosm.routing.matrix`: a travel time (or distance) matrix, a DataFrame.

        >>> m = net.driving.matrix(edge_ids)"""
        from duckosm import routing
        return routing.matrix(self, origins, destinations, mode=mode, **kwargs)

    def isochrones(self, origins, minutes=(5, 10, 15), mode: str | None = None, **kwargs):
        """:func:`duckosm.routing.isochrones`: the polygons reached within each time.

        >>> iso = net.walking.isochrones([(7.4246, 43.7384)])"""
        from duckosm import routing
        return routing.isochrones(self, origins, minutes, mode=mode, **kwargs)

    def accessibility(self, origins, targets, minutes=15, mode: str | None = None):
        """:func:`duckosm.routing.accessibility`: the targets reached from each origin, and the nearest.

        >>> acc = net.walking.accessibility(homes, shops, minutes=[5, 10])"""
        from duckosm import routing
        return routing.accessibility(self, origins, targets, minutes, mode=mode)

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

    # ---- the OSM fixes: duckosm.fixes ------------------------------------------------------------------------------
    @property
    def fixes(self):
        """The :class:`duckosm.fixes.Fixes` of the rules that the network was built with (the table ``main.fixes``), or None.
        The rules are in memory, with no save file: ``net.fixes.save("name.yaml")`` writes them to a file that you choose.
        A file from an older build has no ``main.fixes``: then the rules come from the file at the path of the build.

        >>> net.fixes"""
        from duckosm.fixes import Fixes
        record = self._fixes_record()
        if record is not None:
            return Fixes.from_text(record["yaml"])
        path = self._meta().get("fixes")
        return Fixes(path) if path else None

    def _fixes_record(self) -> dict | None:
        """The row of ``main.fixes`` (``yaml``, ``path``, ``sha256``), or None."""
        if "main.fixes" not in self.tables():
            return None
        rows = self.sql("SELECT yaml, path, sha256 FROM main.fixes LIMIT 1")
        return rows.iloc[0].to_dict() if len(rows) else None

    def has_fixes(self) -> bool:
        """True when the network was built with a fixes file (``main.fixes`` has a row, or, in a file from an older build,
        the build names a fixes file).

        >>> net.has_fixes()"""
        return self._fixes_record() is not None or bool(self._meta().get("fixes"))

    def fixes_changed(self) -> bool:
        """True when the fixes file at the path of the build is now different from the rules that the network was built
        with (``main.fixes``): then build again. False when they are the same. Stops with a message when the network has
        no fixes, or when the file is not there.

        >>> net.fixes_changed()"""
        import hashlib
        record = self._fixes_record()
        if record is None:
            raise DuckOSMError("fixes_changed: this network has no record of its fixes (main.fixes): "
                               "it was built with no fixes file, or by an older duckOSM")
        path = Path(record["path"])
        if not path.is_file():
            raise DuckOSMError(f"fixes_changed: the fixes file of the build is not there: {path}. "
                               "net.fixes has the rules of the build: net.fixes.save('name.yaml') writes them again")
        return hashlib.sha256(path.read_text().encode()).hexdigest() != record["sha256"]

    @property
    def saved(self) -> bool:
        """False while the file is in duckOSM's work folder (:func:`duckosm.work_dir`); ``net.save(path)`` keeps it."""
        from duckosm._api import work_dir
        try:
            self.path.resolve().relative_to(work_dir().resolve())
            return False
        except ValueError:
            return True

    def save(self, path: str | os.PathLike, overwrite: bool = False) -> "Network":
        """Copy the network to ``path`` (a ``.duckdb`` file, or a folder: then ``<folder>/<name>.duckdb``), with its
        companion files beside it under the new name: the cut OSM file (``.osm.pbf``), the boundary (``.geojson``)
        and the drawing-order edits (``.levels``), so that :meth:`rebuild` and :meth:`edit` still work. Returns the
        :class:`Network` of the copy. ``overwrite=False``: an error when ``path`` exists.

        >>> net = duckosm.network(osm_id=2220322, modes=["driving"])
        >>> net = net.save("monaco.duckdb")"""
        import shutil
        dst = Path(path)
        if dst.is_dir() or str(path).endswith(("/", os.sep)):
            dst = dst / self.path.name
        if dst.suffix != ".duckdb":
            dst = dst.with_suffix(".duckdb")
        if dst.exists() and dst.resolve() == self.path.resolve():
            return self
        if dst.exists() and not overwrite:
            raise DuckOSMError(f"{shown(dst)} exists: give another path, or overwrite=True")
        dst.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(self.path, dst)
        old, new = self.path.name.split(".")[0], dst.name.split(".")[0]
        for ext in (".osm.pbf", ".geojson", ".levels"):
            src = self.path.with_name(old + ext)
            if src.is_dir():
                shutil.copytree(src, dst.with_name(new + ext), dirs_exist_ok=True)
            elif src.is_file():
                shutil.copy2(src, dst.with_name(new + ext))
        return Network(dst)

    def rebuild(self, fixes=None, out: str | os.PathLike | None = None, progress: Any = None) -> "Network":
        """Build the area again, with the same modes, from the OSM file and the boundary beside the file (where
        ``duckosm.network`` and ``Area.build`` put them), with ``fixes`` (a :class:`duckosm.fixes.Fixes` or a path).
        ``out``: the new file (default: ``<name>_fixed.duckdb`` beside this one). Returns the new Network.

        >>> fixed = net.rebuild(fixes=fixes)"""
        from duckosm._api import build
        pbf = self.path.with_name(self.path.name.split(".")[0] + ".osm.pbf")
        if not pbf.is_file():
            raise DuckOSMError(f"rebuild: no OSM file {pbf.name} beside {self.path.name}. Build with the Area: "
                               f"duckosm.get(...).build(output=..., fixes=...)")
        boundary = pbf.with_name(pbf.name.split(".")[0] + ".geojson")
        out = Path(out) if out else self.path.with_name(self.path.name.split(".")[0] + "_fixed.duckdb")
        if out.resolve() == self.path.resolve():
            raise DuckOSMError("rebuild: give a new file in out=, not the file of this network")
        return build(pbf=pbf, boundary=boundary if boundary.is_file() else None, modes=self.modes, output=out,
                     fixes=None if fixes is None else os.fspath(fixes), progress=progress)

    def diff(self, other, mode: str | None = None):
        """:func:`duckosm.fixes.diff`: the edges that were added, removed or changed in ``other``, by ``edge_id``.

        >>> net.diff(fixed)"""
        from duckosm.fixes import diff
        return diff(self, other, mode=mode)

    def edit(self, load=None, save=None, port: int = 8790, *, fixes=None) -> str:
        """Open ``duckosm edit`` on the file: the editor of the fixes, on a map. The server runs in the background while
        Python runs. Returns the URL of the page.

        ``load``: a fixes file that the page reads into its list. The page does not write it. ``save``: the fixes file
        that the page writes, each change at once. It can be the ``load`` file, and it need not exist. With no ``save``,
        the first Save on the page asks for the file. A relative path is relative to the folder where Python runs.
        The library never chooses a fixes file: with neither, the list is empty. ``fixes``: old name, the same as
        ``load=fixes, save=fixes``.

        >>> net.edit(load="data/osm_fixes.yaml", save="data/osm_fixes_v2.yaml")"""
        from duckosm import _rust
        from duckosm.edit import _Db
        if fixes is not None:
            import warnings
            warnings.warn("net.edit(fixes=F) is an old name: use net.edit(load=F, save=F)", DeprecationWarning, stacklevel=2)
            load = save = fixes
        path = lambda f: None if f is None else str(Path(f).resolve())  # noqa: E731
        return _rust.edit(str(self.path.resolve()), path(load), path(save), port, _Db(self.path.resolve()))

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

    _CALLS = ("edges", "nodes", "map", "plot", "dashboard", "stats", "route", "route_points", "routes", "k_shortest_paths", "matrix",
              "isochrones", "accessibility", "directions", "to_networkx",
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
        return f"ModeView({shown(self.network.path)!r}, {self.mode}: {r.edges} edges, {r.nodes} nodes, {r.km} km)"

    def _repr_html_(self):
        from html import escape
        return (f"<div><b>{escape(self.mode)}</b> of <code>{escape(shown(self.network.path))}</code>"
                f"{self.summary().to_html(index=False)}</div>")


# ---- duckosm.network is a function and a module -------------------------------------------------------------------------

class _CallableModule(type(os)):
    """When a module imports ``duckosm.network`` (the module), Python sets ``duckosm.network`` to the module, not to the
    function :func:`network`. A call of the module calls the function, so ``duckosm.network("Monaco")`` always works."""

    def __call__(self, *args, **kwargs) -> Network:
        return network(*args, **kwargs)


import sys  # noqa: E402

sys.modules[__name__].__class__ = _CallableModule
