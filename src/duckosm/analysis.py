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


def largest_component(view, out: str | os.PathLike | None = None, strong: bool = True, mode: str | None = None) -> Network:
    """A new network file with only the largest component of the mode (OSMnx ``largest_component``). The mode keeps the
    nodes, edges and turn tables of that component; the other modes do not change. ``out``: the new file (default
    ``<name>_largest.duckdb`` next to the file). Returns its :class:`Network`.

    >>> big = duckosm.analysis.largest_component(net.driving)"""
    db, mode, net = _view(view, mode)
    out = Path(out) if out is not None else net.path.with_name(f"{net.path.name.split('.')[0]}_largest.duckdb")
    if out.resolve() == net.path.resolve():
        raise DuckOSMError("out must be a new file, not the file of the network")
    out.parent.mkdir(parents=True, exist_ok=True)
    _rust().analysis_largest_component(db, mode, bool(strong), str(out))
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
    out = Path(out) if out is not None else net.path.with_name(f"{net.path.name.split('.')[0]}_clip.duckdb")
    if point is not None and not dist_m:
        raise DuckOSMError("point= needs dist_m= (the radius in metres)")
    if polygon is None:
        _rust().extract_area(str(net.path), str(out), None if bbox is None else [float(v) for v in bbox],
                             None if point is None else tuple(map(float, point)), None if dist_m is None else float(dist_m))
        return Network(out)
    require("shapely", "clip with polygon=")
    from shapely.geometry import mapping
    with tempfile.TemporaryDirectory() as tmp:            # your polygon as a GeoJSON file; duckosm extract joins its parts
        area = Path(tmp) / "clip.geojson"
        area.write_text(polygon.to_crs("EPSG:4326").to_json() if hasattr(polygon, "to_crs") else json.dumps(
            {"type": "FeatureCollection", "features": [{"type": "Feature", "properties": {}, "geometry": mapping(polygon)}]}))
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
    import pandas as pd
    db, mode, _ = _view(view, mode)
    try:
        ids, grades = _rust().analysis_grades(db, mode)
    except RuntimeError as e:
        raise DuckOSMError(str(e)) from None
    return pd.Series(grades, index=pd.Index(ids, name="edge_id"), name="grade", dtype="float64")

def sample_points(view, n: int, seed: int | None = None, mode: str | None = None):
    """``n`` random points on the streets (OSMnx ``sample_points``): a street is chosen with a chance that follows its
    length, and the point is at a random place along it. The two rows of a two-way road count as one street. Returns a
    GeoDataFrame (EPSG:4326) with the ``edge_id`` of each point.

    >>> pts = duckosm.analysis.sample_points(net.walking, 200, seed=1)"""
    import random
    gpd = require("geopandas", "sample_points")
    db, mode, _ = _view(view, mode)
    ids, xs, ys = _rust().analysis_sample_points(db, mode, int(n), random.getrandbits(64) if seed is None else int(seed) % 2**64)
    return gpd.GeoDataFrame({"edge_id": ids}, geometry=gpd.points_from_xy(xs, ys), crs="EPSG:4326")

def to_osmnx(view, mode: str | None = None):
    """The mode as the ``networkx.MultiDiGraph`` of OSMnx, so that each OSMnx function works on it (it passes
    ``ox.convert.validate_graph``). Node keys: ``node_id``, with ``x``, ``y`` and ``street_count``. Each edge keeps its
    ``edge_id`` and has the OSMnx names: ``osmid``, ``length``, ``geometry``, ``highway``, ``name``, ``oneway``,
    ``lanes``, ``speed_kph`` and ``travel_time``. Edge keys: 0, 1, ... for the edges between the same two nodes. The
    graph is marked ``simplified`` (each edge has its line); for ``ox.simplify_graph``, set
    ``G.graph["simplified"] = False`` first.

    >>> G = duckosm.analysis.to_osmnx(net.driving)
    >>> ox.plot_graph(G)"""
    import pickle
    require("networkx", "to_osmnx")
    require("shapely", "to_osmnx")             # the pickle makes shapely lines
    db, mode, net = _view(view, mode)
    return pickle.loads(_rust().analysis_osmnx_pickle(db, mode, net.path.name.split(".")[0]))
