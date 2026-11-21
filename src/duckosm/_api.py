"""The Python API of duckOSM on the Rust code (the module ``duckosm._rust``). docs/design/all_in_rust.md, step 6.

The functions keep the names, the arguments and the results of the Python code. Each one calls Rust, except for the
cases that the Rust code does not have; these call the Python code (the module named in each function).

TODO (step 6, before the Python engine code goes):
- The build: ``DuckOSM.run()`` runs the Python steps with the Rust network step, as before. The whole build in Rust
  (``duckosm --config``) is not yet equal to it: no ``features.ocean`` (``options.sea``), no
  ``visualization.level_heads`` and the other tables of the level edits, no ``main.building_faces`` in some builds,
  other tables with a fixes file of the page (tests/test_page_fixes.py), and ``main.visualization_metadata`` gives
  ``duckosm 0.7.0`` as the version. tests/test_engine_rust.py, test_levels_in_build.py, test_ask_schema.py,
  test_editor_changes.py and test_page_fixes.py fail with it.
- GMNS (group C): the command ``duckosm gmns`` is Rust; the functions ``to_gmns``, ``to_meso``, ``to_micro``,
  ``check_gmns`` and ``to_sumo`` with ``bus_edges``, ``edge_attrs`` or ``lanes`` still call the Python code.
- The commands with no Rust port yet: ``levels``, ``clip-pbf`` (see ``duckosm/_main.py``).
- ``to_networkx``, ``to_networkx_nodes`` and ``write_graph`` give networkx graphs: they stay Python.
- ``to_sumo`` with a ``dict`` config or ``netconvert_bin``, ``to_railml`` with other ``rail_types``,
  ``route`` with a ``graph`` and ``route_multimodal`` with ``_points``.
"""

import json
import os
from pathlib import Path

from duckosm._progress import shown


def _rust():
    from duckosm import _rust
    return _rust


def _db(con):
    """The file of a database: a path, or a DuckDB connection to a database file. None: an in-memory database (the Rust
    code cannot read it, so the Python code runs)."""
    if isinstance(con, (str, os.PathLike)):
        return str(con)
    row = con.execute("SELECT path FROM duckdb_databases() WHERE database_name = current_database()").fetchone()
    return row[0] if row and row[0] and row[0] != ":memory:" else None


def _paths(d, keys):
    for k in keys:
        if d.get(k) is not None:
            d[k] = Path(d[k])
    return d


# ---- exports -----------------------------------------------------------------------------------------------------

def to_sumo(con, out_dir, mode: str = "driving", net_name: str = "network", run_netconvert: bool = True,
            netconvert_bin: str = None, connections: bool = True, config=None, bus_edges: bool = False,
            edge_attrs: dict = None, lanes: dict = None):
    """See :func:`duckosm.sumo.to_sumo`."""
    if bus_edges or edge_attrs or lanes or isinstance(config, dict) or netconvert_bin or _db(con) is None:
        from duckosm import sumo
        return sumo.to_sumo(con, out_dir, mode=mode, net_name=net_name, run_netconvert=run_netconvert,
                            netconvert_bin=netconvert_bin, connections=connections, config=config,
                            bus_edges=bus_edges, edge_attrs=edge_attrs, lanes=lanes)
    r = json.loads(_rust().to_sumo(_db(con), str(out_dir), mode, net_name, connections,
                                   None if config is None else str(config), run_netconvert))
    return _paths(r, ("nod", "edg", "con", "netccfg", "net"))


def to_gis(con, out, modes=None, fmt: str = "gpkg", boundary: bool = True, name: str = None,
           srs: str = "EPSG:4326", ogr2ogr_bin: str = None):
    """See :func:`duckosm._gis_py.to_gis`. ``ogr2ogr_bin`` is not used: Rust writes the GeoPackage through DuckDB."""
    if _db(con) is None:
        from duckosm import _gis_py as gis
        return gis.to_gis(con, out, modes, fmt, boundary, name, srs, ogr2ogr_bin)
    modes = [modes] if isinstance(modes, str) else list(modes or [])
    r = json.loads(_rust().to_gis(_db(con), str(out), modes, fmt, boundary, name, srs))
    r["files"] = [Path(f) for f in r["files"]]
    return _paths(r, ("path", "dir"))


def to_matsim(source, out_path, mode="driving", crs=None, gzip=True):
    """See :func:`duckosm.matsim.to_matsim`."""
    if not isinstance(mode, str):
        mode = ",".join(mode)
    return json.loads(_rust().to_matsim(_db(source), str(out_path), mode, crs, gzip))


def to_matsim_lanes(gmns_db, out_dir=".", mode="driving", signals=True, cycle_s=90):
    """See :func:`duckosm.matsim_lanes.to_matsim_lanes`."""
    return json.loads(_rust().to_matsim_lanes(_db(gmns_db), str(out_dir), mode, signals, int(cycle_s)))


def to_opendrive(source, out_path, mode="driving", crs=None, junctions=False):
    """See :func:`duckosm.opendrive.to_opendrive`."""
    return json.loads(_rust().to_opendrive(_db(source), str(out_path), mode, crs, junctions))


_RAIL = ("rail", "light_rail", "subway", "tram", "narrow_gauge", "funicular")


def to_railml(source, out_path, rail_types=_RAIL):
    """See :func:`duckosm.railml.to_railml`."""
    if tuple(rail_types) != _RAIL:
        from duckosm import railml
        return railml.to_railml(source, out_path, rail_types)
    return json.loads(_rust().to_railml(_db(source), str(out_path)))


def to_lanelet2(gmns_db, out_path, mode="driving", snap_m=0.01):
    """See :func:`duckosm.lanelet2.to_lanelet2`."""
    return json.loads(_rust().to_lanelet2(_db(gmns_db), str(out_path), mode, snap_m))


def to_elevation(db, dem=None, source="auto", modes=None, nodata_fill=0.0, suffix="",
                 vertical_datum=None, license=None, reference=True, product=None):
    """See :func:`duckosm.elevation.to_elevation`."""
    modes = [modes] if isinstance(modes, str) else list(modes or [])
    return json.loads(_rust().to_elevation(_db(db), None if dem is None else str(dem), source, modes, float(nodata_fill),
                                           suffix, product, vertical_datum, license, reference))


# ---- routing -----------------------------------------------------------------------------------------------------

class Router:
    """Build the routing graph of one mode once, then answer many shortest-route queries. See
    :class:`duckosm._routing_py.Router`; the graph is in Rust, so there is no ``.graph``."""

    def __init__(self, con, mode: str = "driving", weight: str = "time"):
        self.con, self.mode, self.weight = con, mode, weight
        db = _db(con)
        if db is None:
            from duckosm import _routing_py as routing
            self._py = routing.Router(con, mode, weight)
        else:
            self._py, self._r = None, _rust().Router(db, mode, weight)

    def route(self, from_edge, to_edge):
        """Shortest route between two edge ids: the dict of :func:`route`, or None."""
        if self._py is not None:
            return self._py.route(from_edge, to_edge)
        try:
            return json.loads(self._r.route(int(from_edge), int(to_edge)))
        except RuntimeError as e:
            if "not in the" in str(e):        # as networkx: a ValueError
                raise ValueError(str(e)) from None
            raise


def route(con, from_edge, to_edge, mode: str = "driving", weight: str = "time", graph=None):
    """See :func:`duckosm._routing_py.route`. With a networkx ``graph``, the Python code."""
    if graph is not None or _db(con) is None:
        from duckosm import _routing_py as routing
        return routing.route(con, from_edge, to_edge, mode=mode, weight=weight, graph=graph)
    return Router(con, mode, weight).route(from_edge, to_edge)


def route_multimodal(con, src_node, dst_node, schema: str = "mm", start_mode: str = "walking",
                     end_mode: str = "walking", enforce_sequence: bool = True, allowed_modes=None, _points=None):
    """See :func:`duckosm._routing_py.route_multimodal`."""
    if _points is not None or _db(con) is None:
        from duckosm import _routing_py as routing
        return routing.route_multimodal(con, src_node, dst_node, schema, start_mode, end_mode, enforce_sequence,
                                        allowed_modes, _points)
    modes = None if allowed_modes is None else list(allowed_modes)
    return json.loads(_rust().route_multimodal(_db(con), int(src_node), int(dst_node), schema, start_mode, end_mode,
                                               enforce_sequence, modes))


def route_points(con, a, b, mode="driving", weight="time", radius_m=50.0, access_kmh=4.5):
    """See :func:`duckosm.point_routing.route_points`."""
    if _db(con) is None:
        from duckosm import point_routing
        return point_routing.route_points(con, a, b, mode, weight, radius_m, access_kmh)
    return json.loads(_rust().route_points(_db(con), tuple(a), tuple(b), mode, weight, float(radius_m), float(access_kmh)))


def route_multimodal_points(con, a, b, radius_m=50.0, access_kmh=4.5, schema="mm", allowed_modes=None,
                            enforce_sequence=True):
    """See :func:`duckosm.point_routing.route_multimodal_points`."""
    if _db(con) is None:
        from duckosm import point_routing
        return point_routing.route_multimodal_points(con, a, b, radius_m, access_kmh, schema, allowed_modes, enforce_sequence)
    modes = None if allowed_modes is None else list(allowed_modes)
    return json.loads(_rust().route_multimodal_points(_db(con), tuple(a), tuple(b), float(radius_m), float(access_kmh),
                                                      schema, modes, enforce_sequence))


def directions(con, route, mode="driving"):
    """See :func:`duckosm.directions.directions`."""
    if _db(con) is None:
        from duckosm import directions as d
        return d.directions(con, route, mode)
    return json.loads(_rust().directions(_db(con), json.dumps(route), mode))


def build_lane_graph(gmns_db, mode="driving"):
    """See :func:`duckosm.lane_routing.build_lane_graph`."""
    lanes, lc, turn = _rust().build_lane_graph(_db(gmns_db), mode)
    return {"lanes": lanes, "lane_change": lc, "turn": turn, "edges": lc + turn}


def route_lanes(gmns_db, from_lane, to_lane, mode="driving"):
    """See :func:`duckosm.lane_routing.route_lanes`."""
    return json.loads(_rust().route_lanes(_db(gmns_db), str(from_lane), str(to_lane), mode))


# ---- graphs ------------------------------------------------------------------------------------------------------

def to_rustworkx(con, mode="driving", graph="turns", weight="time"):
    """The graph of ``mode`` as a ``rustworkx.PyDiGraph``, from the rows of the Rust code (no networkx).

    ``graph="turns"``: the graph of :func:`to_networkx` (a node is an ``edge_id``, an arc a legal turn with its ``weight``:
    ``"time"`` in seconds or ``"length"`` in metres). ``graph="junctions"``: the graph of ``to_networkx_nodes`` (a node is
    ``{"node_id", "x", "y"}``, an edge a road with all its columns, ``geometry`` as WKT; parallel roads stay). ``g.attrs["index"]``
    gives the node index of an id (an ``edge_id`` or a ``node_id``)."""
    try:
        import rustworkx as rx
    except ImportError as e:
        raise ImportError('to_rustworkx needs rustworkx: pip install "duckosm[rustworkx]"') from e
    if graph not in ("turns", "junctions"):
        raise ValueError("graph must be 'turns' or 'junctions'")
    db = _db(con)
    if db is None:
        raise ValueError("to_rustworkx reads a database file: give its path or a connection to it")
    junctions, roads, turns = _rust().graph_rows(db, mode, weight)
    if graph == "turns":
        ids = list(dict.fromkeys(i for a, b, _ in turns for i in (a, b)))     # the order of first use, as networkx
        g = rx.PyDiGraph(multigraph=False, attrs={"mode": mode, "weight": weight})
        g.attrs["index"] = dict(zip(ids, g.add_nodes_from(ids)))
        ix = g.attrs["index"]
        g.add_edges_from([(ix[a], ix[b], w) for a, b, w in turns])
        return g
    g = rx.PyDiGraph(multigraph=True, attrs={"mode": mode, "crs": "EPSG:4326"})
    g.attrs["index"] = dict(zip((j[0] for j in junctions),
                                g.add_nodes_from([{"node_id": n, "x": x, "y": y} for n, x, y in junctions])))
    ix = g.attrs["index"]
    g.add_edges_from([(ix[s], ix[t], {"edge_id": e, **json.loads(c)}) for s, t, e, c in roads])
    return g


# ---- the commands: build, tiles, quack, extract, boundary, levels, init-config -----------------------------------
# Each function runs the program duckosm (the Rust code, duckosm._rust.cli) in its own process, with the options of its
# command: the memory goes back to the system at the end, and the global thread pool of a command starts new each time.

def _stream(args, progress=None):
    """Run ``duckosm ARGS``; give each line of its standard error (its progress) to ``progress`` as it comes. Returns
    ``(stdout, the lines of stderr)``. A failure raises :class:`duckosm.DuckOSMError` with the message of the command."""
    import subprocess
    import sys
    import threading
    from duckosm.errors import DuckOSMError
    cmd = [sys.executable, "-m", "duckosm", *(str(a) for a in args if a is not None)]
    p = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    # stdout in a thread: a large output (the JSON of get --dry-run) fills the pipe, and the program then waits
    out = []
    reader = threading.Thread(target=lambda: out.append(p.stdout.read()))
    reader.start()
    err, part = [], b""

    def line(raw):
        text = raw.decode(errors="replace").strip()
        if text:
            err.append(text)
            progress and progress(text)

    for chunk in iter(lambda: p.stderr.read1(4096), b""):    # a download line ends with "\r"
        *lines, part = (part + chunk).replace(b"\r", b"\n").split(b"\n")
        for raw in lines:
            line(raw)
    line(part)
    reader.join()
    stdout = out[0].decode(errors="replace")
    if p.wait() != 0:
        message = "\n".join(err) or stdout
        raise DuckOSMError(f"duckosm {args[0]}: {message.strip()}")
    return stdout, err


def _run(*args, progress=None):
    """Run ``duckosm ARGS`` and give its standard output and standard error. ``progress``: see
    :func:`duckosm._progress.reporter`. A failure raises :class:`duckosm.DuckOSMError` (a ``RuntimeError``)."""
    from duckosm._progress import reporter
    stdout, err = _stream(args, reporter(progress))
    return stdout + "\n".join(err)


def _opt(flag, value):
    """``[flag, value]``, or nothing when ``value`` is None."""
    return [] if value is None else [flag, value]


def _each(flag, values):
    values = [values] if isinstance(values, str) else list(values or [])
    return [x for v in values for x in (flag, v)]


# The build options of the SDK: the keyword and its key in the config file (config/template.yaml). The other keys of
# the config file are for experts: give them with ``config=`` (a file or a dict).
BUILD_OPTIONS = {
    "speeds": ("options", "process_speeds"),              # False: no maxspeed_kmh (the raw maxspeed text stays), no cost_s
    "costs": ("options", "calculate_costs"),              # False: no cost_s
    "restrictions": ("options", "extract_restrictions"),  # False: no turn_restrictions
    "bus_routes": ("options", "bus_routes"),              # False: no bus.routes and bus.route_edges
    "connect_paths": ("clip", "connectivity_rescue"),     # False: free path ends stay free
    "snap_m": ("clip", "connect_snap_m"),                 # the distance for connect_paths, in metres
}


def _build_config(config, options):
    """The config file for ``duckosm build``: ``config`` (a YAML or JSON file, or a dict) with the keyword ``options``
    over it. None when the call needs no file of its own."""
    import copy
    import difflib
    import tempfile

    import yaml

    from duckosm.errors import DuckOSMError
    for k in options:
        if k not in BUILD_OPTIONS:
            near = difflib.get_close_matches(k, list(BUILD_OPTIONS), n=1)
            raise DuckOSMError(f"build has no option {k!r}" + (f": did you mean {near[0]!r}?" if near else "") +
                               f" The options: {', '.join(BUILD_OPTIONS)}. Other keys of config/template.yaml go in config=.")
    if options.get("speeds") is False:
        options.setdefault("costs", False)     # cost_s needs maxspeed_kmh (the build stops on costs without speeds)
    if not options and not isinstance(config, dict):
        return config
    if isinstance(config, dict):
        cfg = copy.deepcopy(config)
    elif config is not None:
        cfg = yaml.safe_load(Path(config).read_text(encoding="utf-8")) or {}
    else:
        cfg = {}
    for k, v in options.items():
        section, key = BUILD_OPTIONS[k]
        cfg.setdefault(section, {})[key] = v
    f = tempfile.NamedTemporaryFile("w", suffix=".json", prefix="duckosm-config-", delete=False, encoding="utf-8")
    with f:
        json.dump(cfg, f)          # JSON is YAML: the build reads it as a config file
    return f.name


def build(pbf=None, *, config=None, boundary=None, modes=None, output=None, source_db=None, h3_cell=None, fixes=None,
          features=None, levels=None, graph=None, h3_index=None, h3_resolution=None, dem=None, progress=None, **options):
    """``duckosm build``: build the networks of an area from a PBF, or cut one from a built database (``source_db``).

    ``config``: a config file (YAML or JSON) or a dict with the keys of ``config/template.yaml``; a keyword overrides
    the same key. ``options``: the keywords of :data:`BUILD_OPTIONS` (``speeds``, ``costs``,
    ``restrictions``, ``bus_routes``, ``connect_paths``, ``snap_m``). ``dem``: a DEM file (GeoTIFF): after the build,
    the heights of the nodes and the edges (:func:`duckosm.analysis.add_elevation`). ``progress``: None shows one
    progress line in a notebook, False shows nothing, a function gets each line. Returns the :class:`duckosm.Network`
    of the .duckdb file (also its path: ``os.PathLike``).

    >>> net = duckosm.build(pbf=area.pbf, boundary=area.boundary, modes=["driving", "walking"])
    >>> net = duckosm.build(pbf="my.osm.pbf", speeds=False, config={"options": {"bus_routes": False}})"""
    def flag(name, value):
        return None if value is None else (f"--{name}" if value else f"--no-{name}")
    if output is None and config is None:        # no place given: a new folder in the work folder
        stem = Path(pbf).name.split(".")[0] if pbf else (Path(boundary).stem if boundary else "area")
        output = _new_work(stem) / f"{stem}.duckdb"
    given = config
    config = _build_config(config, options)
    try:
        text = _run("build", *_opt("-c", config), *_opt("-p", pbf), *_opt("-o", output), *_opt("-b", boundary),
                *_opt("--source-db", source_db), *_opt("--h3-cell", h3_cell), *_each("-m", modes), *_opt("--fixes", fixes),
                flag("features", features), flag("levels", levels), flag("graph", graph), flag("h3-index", h3_index),
                *_opt("--h3-resolution", h3_resolution), progress=progress)
    finally:
        if config is not None and config != given:
            Path(config).unlink(missing_ok=True)
    from duckosm.network import Network
    net = Network(text.rsplit("Output: ", 1)[1].strip())
    if dem is not None:
        to_elevation(net.path, dem=dem)
    return net


def levels(db):
    """``duckosm levels``: make the drawing order of the roads again (``visualization.edge_levels``), with the edits, heads and caps of
    ``visualization.level_*``. The database is changed in place. Returns its path."""
    _run("levels", db)
    return Path(db)


def quack(pbf, *, out=".", name="area", formats=None, gdal=False, fixes=None, threads=None, sort=True, tiles=None,
          tile_zoom=None, progress=None):
    """``duckosm quack``: the features table of QuackOSM, or with ``gdal=True`` the five layers of GDAL's OSM driver. Returns
    the paths of the files that it wrote."""
    formats = [formats] if isinstance(formats, str) else formats
    _run("quack", pbf, "-o", out, "--name", name, *_opt("--format", formats and ",".join(formats)),
         "--gdal" if gdal else None, *_opt("--fixes", fixes), *_opt("--threads", threads), None if sort else "--no-sort",
         *_opt("--tiles", tiles), *_opt("--tile-zoom", tile_zoom), progress=progress)
    out = Path(out)
    return sorted(out.glob(f"{name}_gdal*" if gdal else f"{name}_quack.*"))


def tiles(region=None, *, boundary=None, out=None, config=None, zoom=8, memory="2G", jobs=None, tile_threads=2,
          keep_tiles=False, build_args=()):
    """``duckosm tiles``: build a large area by tiles. ``build_args``: the build options of each tile (after ``--``).
    Returns the output folder."""
    _run("tiles", region, *_opt("--boundary", boundary), *_opt("-o", out), *_opt("--config", config),
                    "--zoom", zoom, "--memory", memory, *_opt("--jobs", jobs), "--tile-threads", tile_threads,
                    "--keep-tiles" if keep_tiles else None, *(["--", *build_args] if build_args else []))
    if out is None:                    # the output_path of the config
        import yaml
        out = yaml.safe_load(Path(config).read_text()).get("output_path", ".")
    return Path(out)


def extract(source, db, *, name=None, osm_id=None, boundary=None):
    """``duckosm extract``: cut one area (give one of ``name``, ``osm_id`` and ``boundary``) out of a built database into
    the new database ``db``. Returns its path."""
    _run("extract", "--source", source, "--db", db, *_opt("--name", name), *_opt("--osm-id", osm_id),
         *_opt("--boundary", boundary))
    return Path(db)


def boundary(name=None, *, pbf=None, osm_id=None, offline=False, out=None):
    """``duckosm boundary``: find the boundary of an area by name or OSM id and write it as GeoJSON. Returns its path."""
    text = _run("boundary", name, *_opt("--pbf", pbf), *_opt("--osm-id", osm_id), "--offline" if offline else None,
                *_opt("-o", out))
    # the first line is "wrote <file>: ..."
    return Path(out) if out is not None else Path(text.split("\n", 1)[0].removeprefix("wrote ").split(": ", 1)[0])


def init_config(path="duckosm.yaml", *, force=False):
    """``duckosm init-config``: write the commented config template. Returns its path."""
    _run("init-config", path, "--force" if force else None)
    return Path(path)
# ---- level 0: the OSM file -----------------------------------------------------------------------------------------

class Area:
    """The result of :func:`get`: ``pbf`` (the cut ``.osm.pbf`` file) and ``boundary`` (its GeoJSON boundary), as paths."""

    def __init__(self, pbf, boundary):
        self.pbf, self.boundary = Path(pbf), Path(boundary)

    def __repr__(self):
        return f"Area(pbf={shown(self.pbf)!r}, boundary={shown(self.boundary)!r})"

    def save(self, folder, name=None):
        """Copy the two files of the area (the cut ``.osm.pbf`` and the ``.geojson``) into ``folder``, as
        ``<name>.osm.pbf`` and ``<name>.geojson`` (default: the name of the area). Returns the :class:`Area` of the
        copies.

        >>> area.save("my_area")"""
        import shutil
        folder = Path(folder)
        folder.mkdir(parents=True, exist_ok=True)
        stem = name or self.pbf.name.split(".")[0]
        pbf, boundary = folder / f"{stem}.osm.pbf", folder / f"{stem}.geojson"
        shutil.copy2(self.pbf, pbf)
        shutil.copy2(self.boundary, boundary)
        return Area(pbf, boundary)

    def build(self, modes=None, *, output=None, fixes=None, progress=None, **options):
        """Build the network of this area: :func:`duckosm.build` with this OSM file and boundary. ``output``: the
        ``.duckdb`` file (default: next to the OSM file). Returns the :class:`duckosm.Network`.

        >>> net = duckosm.get("Monaco").build(modes=["driving", "walking"])"""
        output = output or self.pbf.parent / f"{self.pbf.name.split('.')[0]}.duckdb"
        return build(pbf=self.pbf, boundary=self.boundary, modes=modes, output=output, fixes=fixes, progress=progress,
                     **options)

    def _repr_html_(self):
        """The card of the area in a notebook: its name and its two files with their sizes."""
        from html import escape
        rows = "".join(f"<tr><td>{k}</td><td><code>{escape(shown(p))}</code></td><td>{_size(p)}</td></tr>"
                       for k, p in (("OSM file", self.pbf), ("boundary", self.boundary)))
        return (f"<div><b>Area {escape(self.pbf.name.split('.')[0])}</b>"
                f"<table><tr><th></th><th>file</th><th>size</th></tr>{rows}</table></div>")


def _size(path):
    """The size of a file as text (for example "1.2 MB"), or "missing"."""
    try:
        n = os.path.getsize(path)
    except OSError:
        return "missing"
    return f"{n / 1e6:.1f} MB" if n >= 1e5 else f"{n / 1e3:.0f} kB"


def work_dir() -> Path:
    """duckOSM's work folder, ``<the cache>/work``: the files of an area or a network that you did not save. The cache
    is ``$DUCKOSM_CACHE``, else ``$XDG_CACHE_HOME/duckosm``, else ``~/.cache/duckosm`` (as the downloads)."""
    base = os.environ.get("DUCKOSM_CACHE") or (
        os.path.join(os.environ["XDG_CACHE_HOME"], "duckosm") if os.environ.get("XDG_CACHE_HOME")
        else os.path.join(os.path.expanduser("~"), ".cache", "duckosm"))
    return Path(base) / "work"


def _new_work(name=None) -> Path:
    """A new folder in the work folder for one call: ``<name>-<date>-<time>-<4 hex>``."""
    import secrets
    import time
    folder = work_dir() / f"{name or 'area'}-{time.strftime('%Y%m%d-%H%M%S')}-{secrets.token_hex(2)}"
    folder.mkdir(parents=True, exist_ok=True)
    return folder


def clear_work(days: float | None = None) -> int:
    """Delete the folders of the work folder (the files that you did not save), or only those older than ``days``.
    Returns how many folders it deleted. A saved file is not in the work folder, so it stays.

    >>> duckosm.clear_work(days=7)"""
    import shutil
    import time
    folder = work_dir()
    if not folder.is_dir():
        return 0
    n = 0
    for d in folder.iterdir():
        if d.is_dir() and (days is None or time.time() - d.stat().st_mtime > days * 86400):
            shutil.rmtree(d, ignore_errors=True)
            n += 1
    return n


def _slug(text, default="area"):
    """A file name from a place name: lower case, ``_`` for each run of other characters."""
    import re
    s = re.sub(r"[^a-z0-9]+", "_", str(text or "").lower()).strip("_")
    return s or default


def get(place=None, *, osm_id=None, bbox=None, h3_cell=None, boundary=None, point=None, dist_m=1000, out=None, name=None,
        fixes=None, cache=None, refresh=False, dry_run=False, progress=None):
    """``duckosm get``: the boundary of an area, the smallest Geofabrik extract that holds it (downloaded to a cache
    folder), and the cut of the extract to the boundary, with the OSM ``fixes`` in it when given. Give one of
    ``place``, ``osm_id``, ``bbox`` (``(west, south, east, north)``), ``h3_cell``, ``point`` and ``boundary``.
    ``point``: ``(lon, lat)``, the area is the box ``dist_m`` metres from it in each direction. ``boundary`` is a
    GeoJSON file, or GeoJSON as a dict (a geometry, a Feature or a FeatureCollection), or an object with
    ``__geo_interface__`` (shapely, geopandas): it is written to ``<out>/<name>.geojson`` (``name`` default ``"area"``).
    ``out``: the folder of the two files; default: a new folder in the work folder (:func:`work_dir`), so your folder
    stays clean; ``area.save(folder)`` keeps them. Returns an :class:`Area`: ``area.build()`` builds it.

    ``dry_run=True`` downloads and writes nothing: it returns a dict with the ``name``, the ``boundary`` (a GeoJSON
    Feature) and the ``extract`` (``id``, ``name``, ``km2``, ``url``, ``bytes``, ``cached``). ``progress``: None shows
    one progress line in a notebook, False shows nothing, a function gets each line (the download too).

    >>> area = duckosm.get("Monaco")
    >>> area = duckosm.get(point=(7.4246, 43.7384), dist_m=500)"""
    import math
    from duckosm._progress import reporter
    if out is None and not dry_run:
        out = _new_work(name or (_slug(place) if place else f"osm_{osm_id}" if osm_id else None))
    if point is not None:            # the box around the point (as osmnx graph_from_point)
        lon, lat = (float(v) for v in point)
        dy, dx = dist_m / 111_320, dist_m / (111_320 * math.cos(math.radians(lat)))
        bbox, name = (lon - dx, lat - dy, lon + dx, lat + dy), name or "point"
    if boundary is not None and not isinstance(boundary, (str, os.PathLike)):
        geo = getattr(boundary, "__geo_interface__", boundary)
        if not isinstance(geo, dict):
            raise TypeError("boundary must be a file path, a GeoJSON dict or an object with __geo_interface__")
        name = name or "area"
        out = out if out is not None else _new_work(name)
        Path(out).mkdir(parents=True, exist_ok=True)
        boundary = Path(out) / f"{name}.geojson"
        boundary.write_text(json.dumps(geo))
    if bbox is not None and not isinstance(bbox, str):
        bbox = ",".join(str(float(v)) for v in bbox)
    args = [place]
    for flag, value in (("--osm-id", osm_id), ("--bbox", bbox), ("--h3-cell", h3_cell), ("--boundary", boundary),
                        ("-o", out), ("--name", name), ("--fixes", fixes), ("--cache", cache)):
        if value is not None:
            args += [flag, value]
    args += ["--refresh"] * bool(refresh) + ["--dry-run"] * bool(dry_run)
    stdout, _ = _stream(["get", *args], reporter(progress))
    if dry_run:
        return json.loads(stdout)
    # the line "wrote <pbf> and <geojson> in <s> s"
    line = next(x for x in stdout.splitlines() if x.startswith("wrote "))
    pbf, rest = line.removeprefix("wrote ").split(" and ", 1)
    return Area(pbf, rest.rsplit(" in ", 1)[0])
