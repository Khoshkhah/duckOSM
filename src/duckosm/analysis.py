"""``duckosm.analysis``: street network analysis, the functions of OSMnx (docs/proposals/python-sdk.md, section 5.1).

Each function takes a ``ModeView`` (``net.driving``), a ``Network`` with one mode, or a path with ``mode=``. The counts,
the bearings, the components and the nearest searches run in Rust (``duckosm-core/src/analysis.rs``); this module
calls them and makes the pandas, numpy and networkx values.

>>> net = duckosm.network("Monaco")
>>> duckosm.analysis.stats(net.driving)
>>> duckosm.analysis.nearest_edges(net.driving, [7.42, 7.43], [43.73, 43.74])
"""
from __future__ import annotations

import json
import os
import shutil
import tempfile
from pathlib import Path

from duckosm.errors import DuckOSMError, require
from duckosm.network import Network, resolve

__all__ = ["stats", "streets_per_node", "bearings", "orientation_entropy", "plot_orientation", "nearest_nodes",
           "nearest_edges", "components", "largest_component", "clip", "add_elevation", "grades", "sample_points",
           "to_osmnx"]

# the keys of OSMnx basic_stats, in its order
STATS_KEYS = ("n", "m", "k_avg", "edge_length_total", "edge_length_avg", "streets_per_node_avg",
              "streets_per_node_counts", "streets_per_node_proportions", "intersection_count", "street_length_total",
              "street_segment_count", "street_length_avg", "circuity_avg", "self_loop_proportion", "node_density_km",
              "intersection_density_km", "edge_density_km", "street_density_km")


def _rust():
    from duckosm import _rust
    return _rust


def _view(view, mode):
    """``(database file, mode, Network)`` of a module call."""
    db, mode = resolve(view, mode)
    return str(Path(os.fspath(db))), mode, db if isinstance(db, Network) else Network(db)


def stats(view, area_m2: float | None = None, mode: str | None = None):
    """The numbers of OSMnx ``basic_stats``, in two views (columns ``osmnx`` and ``duckosm``):

    | View | Nodes and edges |
    |---|---|
    | ``osmnx`` | a node with exactly 2 neighbouring nodes and its roads passing through (for example a split for another mode) is not a node, and its two edges are one edge: the graph of OSMnx |
    | ``duckosm`` | every row of ``<mode>.nodes`` and ``<mode>.edges`` |

    ``area_m2``: the area in m² for the densities (``*_density_km``); None: no densities, as in OSMnx (the boundary of
    an area can include the sea). Lengths are in metres. ``streets_per_node_counts`` and ``streets_per_node_proportions`` are dicts (streets → nodes).

    >>> duckosm.analysis.stats(net.driving)
    >>> net.driving.stats()["osmnx"]["intersection_count"]"""
    import pandas as pd
    db, mode, _ = _view(view, mode)
    raw = json.loads(_rust().analysis_stats(db, mode, None if area_m2 is None else float(area_m2)))
    cols = {}
    for name in ("osmnx", "duckosm"):
        s = raw[name]
        for k in ("streets_per_node_counts", "streets_per_node_proportions"):
            s[k] = dict(enumerate(s[k]))
        cols[name] = pd.Series({k: s[k] for k in STATS_KEYS if k in s}, dtype=object)
    return pd.DataFrame(cols)


def streets_per_node(view, mode: str | None = None):
    """The streets at each node (OSMnx ``count_streets_per_node``): a Series indexed by ``node_id``. A two-way road
    counts once: 1 = a dead end, 2 = a road passing through, 3 = a T junction, 4 = a crossing.

    >>> duckosm.analysis.streets_per_node(net.driving).value_counts()"""
    import pandas as pd
    db, mode, _ = _view(view, mode)
    ids, counts = _rust().analysis_streets_per_node(db, mode)
    return pd.Series(counts, index=pd.Index(ids, name="node_id"), name="streets", dtype="int64")


def bearings(view, mode: str | None = None):
    """The compass bearing of each edge, from its source node to its target node, 0 to 360 degrees (OSMnx
    ``add_edge_bearings``): a Series indexed by ``edge_id``. NaN for a self-loop.

    >>> duckosm.analysis.bearings(net.driving)"""
    import pandas as pd
    db, mode, _ = _view(view, mode)
    ids, b = _rust().analysis_bearings(db, mode)
    return pd.Series(b, index=pd.Index(ids, name="edge_id"), name="bearing", dtype="float64")


def _orientation(view, num_bins, min_length, weight, mode):
    if weight not in (None, "length"):
        raise DuckOSMError(f"weight must be None or 'length', not {weight!r}")
    db, mode, _ = _view(view, mode)
    return mode, _rust().analysis_orientation(db, mode, int(num_bins), float(min_length), weight == "length")


def orientation_entropy(view, num_bins: int = 36, min_length: float = 0, weight: str | None = None,
                        mode: str | None = None) -> float:
    """The entropy of the street bearings (OSMnx ``orientation_entropy`` on the undirected graph): ln(``num_bins``) =
    3.58 when the streets go in all directions, ln(4) = 1.39 for a perfect grid. ``weight="length"``: a long street
    counts more.

    >>> duckosm.analysis.orientation_entropy(net.driving)"""
    return float(_orientation(view, num_bins, min_length, weight, mode)[1][1])


def plot_orientation(view, num_bins: int = 36, min_length: float = 0, weight: str | None = None, *, title=None, ax=None,
                     area: bool = True, color: str = "#003366", mode: str | None = None):
    """The polar histogram of the street bearings (OSMnx ``plot_orientation``). ``area=True``: the area of a bar, not
    its length, shows its share. Returns ``(fig, ax)``.

    >>> fig, ax = duckosm.analysis.plot_orientation(net.driving, title="Monaco")"""
    np = require("numpy", "plot_orientation")
    plt = require("matplotlib.pyplot", "plot_orientation")
    mode, (counts, _) = _orientation(view, num_bins, min_length, weight, mode)
    counts = np.asarray(counts)
    freq = counts / counts.sum()
    radius = np.sqrt(freq) if area else freq
    if ax is None:
        fig, ax = plt.subplots(figsize=(5, 5), subplot_kw={"projection": "polar"})
    else:
        fig = ax.figure
    ax.set_theta_zero_location("N")
    ax.set_theta_direction("clockwise")
    ax.set_ylim(top=radius.max())
    ax.set_yticks(np.linspace(0, radius.max(), 5))
    ax.set_yticklabels([])
    ax.set_xticks(np.deg2rad(np.arange(0, 360, 45)))
    ax.set_xticklabels(["N", "", "E", "", "S", "", "W", ""], fontweight="bold")
    ax.bar(np.deg2rad(np.arange(num_bins) * 360 / num_bins), radius, width=2 * np.pi / num_bins, align="center",
           color=color, edgecolor="k", linewidth=0.5, alpha=0.7, zorder=2)
    ax.set_title(title if title is not None else mode, y=1.05, fontweight="bold")
    return fig, ax


def _nearest(call, view, x, y, return_dist, mode):
    np = require("numpy", "nearest_nodes and nearest_edges")
    scalar = np.ndim(x) == 0
    xs, ys = np.atleast_1d(np.asarray(x, dtype="float64")), np.atleast_1d(np.asarray(y, dtype="float64"))
    if xs.shape != ys.shape:
        raise DuckOSMError("x and y must have the same length")
    if np.isnan(xs).any() or np.isnan(ys).any():
        raise DuckOSMError("x and y cannot contain NaN")
    db, mode, _ = _view(view, mode)
    ids, dist = call(db, mode, xs.tolist(), ys.tolist())
    ids, dist = np.asarray(ids, dtype="int64"), np.asarray(dist, dtype="float64")
    if scalar:
        ids, dist = int(ids[0]), float(dist[0])
    return (ids, dist) if return_dist else ids


def nearest_nodes(view, x, y, return_dist: bool = False, mode: str | None = None):
    """The nearest node to each point (OSMnx ``nearest_nodes``). ``x``, ``y``: longitudes and latitudes (numbers or
    arrays). Returns the ``node_id`` of each point (an array, or a number for a number); ``return_dist=True`` also gives
    the distances in metres.

    >>> ids, dist = duckosm.analysis.nearest_nodes(net.driving, xs, ys, return_dist=True)"""
    return _nearest(_rust().analysis_nearest_nodes, view, x, y, return_dist, mode)


def nearest_edges(view, x, y, return_dist: bool = False, mode: str | None = None):
    """The nearest edge to each point (OSMnx ``nearest_edges``), by the distance to the edge's line. Returns the
    ``edge_id`` of each point; the two rows of a two-way road are at the same distance, and the lower ``edge_id`` comes.
    ``return_dist=True`` also gives the distances in metres.

    >>> duckosm.analysis.nearest_edges(net.driving, 7.4246, 43.7384)"""
    return _nearest(_rust().analysis_nearest_edges, view, x, y, return_dist, mode)


def components(view, strong: bool = True, mode: str | None = None):
    """The connected component of each node: a Series indexed by ``node_id``, 0 for the largest component, 1 for the
    next, ... ``strong=True``: you can go from each node of a component to each other node along the edge directions;
    ``strong=False``: weakly connected (directions do not count).

    >>> duckosm.analysis.components(net.driving).value_counts()"""
    import pandas as pd
    db, mode, _ = _view(view, mode)
    ids, labels = _rust().analysis_components(db, mode, bool(strong))
    return pd.Series(labels, index=pd.Index(ids, name="node_id"), name="component", dtype="int64")


# the turn tables of a mode and their two edge columns (the tables of duckosm extract)
_TURN_TABLES = {"edge_graph": ("from_edge", "to_edge"), "turn_restrictions": ("from_edge_id", "to_edge_id"),
                "turn_permission": ("from_edge", "to_edge"), "turn_path_restrictions": ("from_edge", "to_edge")}


def largest_component(view, out: str | os.PathLike | None = None, strong: bool = True, mode: str | None = None) -> Network:
    """A new network file with only the largest component of the mode (OSMnx ``largest_component``). The mode keeps the
    nodes, edges and turn tables of that component; the other modes do not change. ``out``: the new file (default
    ``<name>_largest.duckdb`` next to the file). Returns its :class:`Network`.

    >>> big = duckosm.analysis.largest_component(net.driving)"""
    import duckdb
    db, mode, net = _view(view, mode)
    keep = components(view, strong=strong, mode=mode)
    keep = keep.index[keep == 0].tolist()
    out = Path(out) if out is not None else net.path.with_name(f"{net.path.name.split('.')[0]}_largest.duckdb")
    if out.resolve() == net.path.resolve():
        raise DuckOSMError("out must be a new file, not the file of the network")
    out.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(net.path, out)
    con = duckdb.connect(str(out))
    try:
        con.load_extension("spatial")           # the RTREE index of the edges
        tables = {t for (t,) in con.execute("SELECT table_name FROM information_schema.tables WHERE table_schema = ?",
                                            [mode]).fetchall()}
        con.execute("CREATE TEMP TABLE _keep AS SELECT unnest($k::BIGINT[]) AS node_id", {"k": keep})
        con.execute(f"DELETE FROM {mode}.edges WHERE source NOT IN (SELECT node_id FROM _keep) "
                    "OR target NOT IN (SELECT node_id FROM _keep)")
        con.execute(f"DELETE FROM {mode}.nodes WHERE node_id NOT IN (SELECT node_id FROM _keep)")
        for t, (a, b) in _TURN_TABLES.items():
            if t in tables:
                con.execute(f"DELETE FROM {mode}.{t} WHERE {a} NOT IN (SELECT edge_id FROM {mode}.edges) "
                            f"OR {b} NOT IN (SELECT edge_id FROM {mode}.edges)")
        con.execute("CHECKPOINT")
    finally:
        con.close()
    # ponytail: the multimodal graph mm.* keeps its rows; cut it too when a user routes across modes on the result
    return Network(out)


def clip(net, *, bbox=None, polygon=None, point=None, dist_m: float | None = None,
         out: str | os.PathLike | None = None) -> Network:
    """A new network file with the edges inside an area (OSMnx ``truncate_graph_bbox`` and ``truncate_graph_polygon``;
    ``duckosm extract``). Give one area: ``bbox=(west, south, east, north)``, ``polygon=`` (a shapely polygon or a
    GeoDataFrame in EPSG:4326), or ``point=(lon, lat)`` with ``dist_m`` (a circle). An edge that crosses the border stays
    whole. All modes are cut. ``out``: the new file (default ``<name>_clip.duckdb``). Returns its :class:`Network`.

    >>> small = duckosm.analysis.clip(net, point=(7.4280, 43.7393), dist_m=500)"""
    from duckosm import _api
    if hasattr(net, "network"):          # a ModeView: its file
        net = net.network
    net = net if isinstance(net, Network) else Network(net)
    if sum(a is not None for a in (bbox, polygon, point)) != 1:
        raise DuckOSMError("give one area: bbox=, polygon= or point= with dist_m=")
    shapely = require("shapely", "clip")
    from shapely.geometry import box, mapping
    if bbox is not None:
        geom = box(*bbox)
    elif point is not None:
        if not dist_m:
            raise DuckOSMError("point= needs dist_m= (the radius in metres)")
        gpd = require("geopandas", "clip with point=")
        p = gpd.GeoSeries([shapely.geometry.Point(point)], crs="EPSG:4326")
        utm = p.estimate_utm_crs()
        geom = p.to_crs(utm).buffer(float(dist_m), 64).to_crs("EPSG:4326").iloc[0]
    else:
        geom = polygon.to_crs("EPSG:4326").union_all() if hasattr(polygon, "to_crs") else polygon
    out = Path(out) if out is not None else net.path.with_name(f"{net.path.name.split('.')[0]}_clip.duckdb")
    with tempfile.TemporaryDirectory() as tmp:
        area = Path(tmp) / "clip.geojson"
        area.write_text(json.dumps({"type": "FeatureCollection", "features": [
            {"type": "Feature", "properties": {}, "geometry": mapping(geom)}]}))
        _api.extract(net.path, out, boundary=area)
    return Network(out)


def add_elevation(net, dem: str | os.PathLike | None = None, **kwargs) -> Network:
    """Add the ground height to the nodes (``ele``) and the edges (``z_from``, ``z_to``, ``z_vertices``): ``duckosm
    elevation`` (OSMnx ``add_node_elevations_raster``). ``dem``: a DEM file (GeoTIFF); default: the source of the area
    (``source="auto"``). Other options: as :func:`duckosm.to_elevation`. Returns the network. Then :func:`grades`.

    >>> duckosm.analysis.add_elevation(net, dem="dtm.tif")"""
    from duckosm import _api
    if hasattr(net, "network"):
        kwargs.setdefault("modes", [net.mode])
        net = net.network
    net = net if isinstance(net, Network) else Network(net)
    _api.to_elevation(net.path, dem=dem, **kwargs)
    return net


def grades(view, mode: str | None = None):
    """The grade of each edge, ``(z_to - z_from) / length_m`` (OSMnx ``add_edge_grades``; 0.05 = 5 % up): a Series
    indexed by ``edge_id``. Needs :func:`add_elevation` first.

    >>> duckosm.analysis.grades(net.driving).describe()"""
    db, mode, net = _view(view, mode)
    cols = set(net.columns(f"{mode}.edges").column_name)
    if not {"z_from", "z_to"} <= cols:
        raise DuckOSMError(f"{mode}.edges has no z_from and z_to: run duckosm.analysis.add_elevation(net) first")
    df = net.sql(f"SELECT edge_id, (z_to - z_from) / nullif(length_m, 0) AS grade FROM {mode}.edges ORDER BY edge_id")
    return df.set_index("edge_id")["grade"].astype("float64")


def sample_points(view, n: int, seed: int | None = None, mode: str | None = None):
    """``n`` random points on the streets (OSMnx ``sample_points``): a street is chosen with a chance that follows its
    length, and the point is at a random place along it. The two rows of a two-way road count as one street. Returns a
    GeoDataFrame (EPSG:4326) with the ``edge_id`` of each point.

    >>> pts = duckosm.analysis.sample_points(net.walking, 200, seed=1)"""
    np = require("numpy", "sample_points")
    _, mode, net = _view(view, mode)
    e = net.edges(mode)
    a, b = np.minimum(e.source, e.target), np.maximum(e.source, e.target)
    e = e.assign(_a=a, _b=b).drop_duplicates(["osm_id", "_a", "_b", "length_m"]).reset_index(drop=True)
    utm = e.estimate_utm_crs()
    lines = e.geometry.to_crs(utm)
    rng = np.random.default_rng(seed)
    w = lines.length.to_numpy()
    pick = rng.choice(len(e), size=int(n), p=w / w.sum())
    pts = lines.iloc[pick].reset_index(drop=True).interpolate(rng.random(int(n)), normalized=True)
    gpd = require("geopandas", "sample_points")
    return gpd.GeoDataFrame({"edge_id": e.edge_id.iloc[pick].to_numpy()}, geometry=pts.to_crs("EPSG:4326").values,
                            crs="EPSG:4326")


def to_osmnx(view, mode: str | None = None):
    """The mode as the ``networkx.MultiDiGraph`` of OSMnx, so that each OSMnx function works on it (it passes
    ``ox.convert.validate_graph``). Node keys: ``node_id``, with ``x``, ``y`` and ``street_count``. Each edge keeps its
    ``edge_id`` and has the OSMnx names: ``osmid``, ``length``, ``geometry``, ``highway``, ``name``, ``oneway``,
    ``lanes``, ``speed_kph`` and ``travel_time``. Edge keys: 0, 1, ... for the edges between the same two nodes. The
    graph is marked ``simplified`` (each edge has its line); for ``ox.simplify_graph``, set
    ``G.graph["simplified"] = False`` first.

    >>> G = duckosm.analysis.to_osmnx(net.driving)
    >>> ox.plot_graph(G)"""
    nx = require("networkx", "to_osmnx")
    import pandas as pd
    _, mode, net = _view(view, mode)
    nodes = net.sql(f"SELECT node_id, ST_X(geom) AS x, ST_Y(geom) AS y FROM {mode}.nodes ORDER BY node_id")
    e = net.edges(mode)
    # simplified: each edge has its line, so ox.project_graph projects the lines too
    G = nx.MultiDiGraph(crs="epsg:4326", name=net.path.name.split(".")[0], mode=mode, simplified=True)
    spn = streets_per_node(view, mode=mode)
    G.add_nodes_from((int(r.node_id), {"x": float(r.x), "y": float(r.y), "street_count": int(spn.get(r.node_id, 0))})
                     for r in nodes.itertuples())
    names = {"edge_id": "edge_id", "osm_id": "osmid", "length_m": "length", "highway": "highway", "name": "name",
             "oneway": "oneway", "lanes": "lanes", "maxspeed_kmh": "speed_kph", "cost_s": "travel_time"}
    cols = [c for c in names if c in e.columns]
    for row, geom in zip(e[["source", "target", *cols]].itertuples(index=False), e.geometry):
        d = {names[c]: v.item() if hasattr(v, "item") else v for c, v in zip(cols, row[2:]) if not pd.isna(v)}
        G.add_edge(int(row[0]), int(row[1]), geometry=geom, **d)
    return G
