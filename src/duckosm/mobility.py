"""``duckosm.mobility``: mobility data, GPS traces and sensors on the roads (docs/proposals/python-sdk.md, section 5.6).

Each function takes a ``ModeView`` (``net.driving``), a ``Network`` with one mode, or a path with ``mode=``. The snap,
the map matching and the edge id map run in Rust (``duckosm-core/src/matching.rs``).

The map matching is the hidden Markov model of Newson and Krumm (2009), solved with the Viterbi algorithm, as in
leuvenmapmatching, Valhalla and GraphHopper. The states of a GPS point are the edges within ``radius_m``; the emission
is a normal distribution of the distance to the edge (``sigma_m``); the transition is an exponential distribution of the
difference between the route distance along the edge directions and the straight distance (``beta_m``).

>>> m = duckosm.mobility.match(net.driving, gps)        # gps: columns trace_id, x (lon), y (lat), time
>>> m.edges                                             # the route of each trace, edge by edge
>>> duckosm.mobility.aggregate(net.driving, m.points, time="hour")
"""
from __future__ import annotations

import os
from collections import namedtuple
from pathlib import Path

from duckosm.errors import DuckOSMError, require
from duckosm.network import Network, resolve

__all__ = ["snap", "match", "Match", "join", "aggregate", "edge_id_map"]

Match = namedtuple("Match", ["points", "edges"])
Match.__doc__ = """The result of :func:`match`. ``points``: the input rows with ``edge_id``, ``fraction``, ``distance_m``,
``snap_x``, ``snap_y`` added. ``edges``: the route of each trace, one row for each edge in order (``trace_id``, ``seq``,
``edge_id``)."""

SNAP_COLUMNS = ("edge_id", "fraction", "distance_m", "snap_x", "snap_y")


def _rust():
    from duckosm import _rust
    return _rust


def _db(view, mode):
    db, mode = resolve(view, mode)
    return str(Path(os.fspath(db))), mode


def _xy(points, x, y):
    """The DataFrame of the points and their lon, lat arrays: a GeoDataFrame (points in any CRS), a DataFrame with the
    columns ``x`` and ``y``, or a list of ``(lon, lat)``."""
    np = require("numpy", "duckosm.mobility")
    import pandas as pd
    if hasattr(points, "geometry") and hasattr(points, "crs"):
        g = points.geometry.to_crs("EPSG:4326") if points.crs is not None else points.geometry
        return pd.DataFrame(points.drop(columns=points.geometry.name)), g.x.to_numpy(), g.y.to_numpy()
    if not isinstance(points, pd.DataFrame):
        points = pd.DataFrame(np.asarray(points, dtype="float64").reshape(-1, 2), columns=[x, y])
    for c in (x, y):
        if c not in points:
            raise DuckOSMError(f"the points have no column {c!r}; give x= and y= (the columns of lon and lat)")
    xs, ys = points[x].to_numpy("float64"), points[y].to_numpy("float64")
    if np.isnan(xs).any() or np.isnan(ys).any():
        raise DuckOSMError("the points cannot contain NaN coordinates")
    return points.reset_index(drop=True), xs, ys


def _columns(df, cols):
    import pandas as pd
    out = pd.DataFrame(dict(zip(SNAP_COLUMNS, cols)))
    out["edge_id"] = out["edge_id"].astype("Int64").where(out["edge_id"] >= 0)
    df = df.drop(columns=[c for c in SNAP_COLUMNS if c in df])        # the columns of an earlier snap or match go
    return pd.concat([df.reset_index(drop=True), out], axis=1)


def snap(view, points, radius_m: float | None = None, *, x: str = "x", y: str = "y", mode: str | None = None):
    """The nearest edge of each point and the position on it (one point at a time, no route: for sensors and
    addresses; for GPS traces use :func:`match`). ``points``: a GeoDataFrame, a DataFrame with lon and lat columns
    (``x=``, ``y=``) or a list of ``(lon, lat)``. Adds ``edge_id``, ``fraction`` (0 at the start of the edge, 1 at its
    end), ``distance_m``, ``snap_x`` and ``snap_y`` (they replace columns of the same name). ``radius_m``: no edge further away (``edge_id`` is missing).

    >>> duckosm.mobility.snap(net.driving, sensors, radius_m=30)"""
    df, xs, ys = _xy(points, x, y)
    db, mode = _db(view, mode)
    return _columns(df, _rust().mobility_snap(db, mode, xs.tolist(), ys.tolist(),
                                              None if radius_m is None else float(radius_m)))


def match(view, traces, *, trace: str | None = "trace_id", time: str | None = "time", x: str = "x", y: str = "y",
          radius_m: float = 50.0, sigma_m: float = 5.0, beta_m: float = 10.0, uturn_m: float = 50.0,
          method: str = "hmm", window_s: int = 60, mode: str | None = None) -> Match:
    """Map matching: the edges that each GPS trace drove, and the edge of each point. ``traces``: a GeoDataFrame or a
    DataFrame with lon and lat columns (``x=``, ``y=``); ``trace``: the column of the trace id (None or missing: one
    trace); ``time``: the column to order the points of a trace by (None or missing: the row order).

    | Option | Meaning |
    |---|---|
    | ``radius_m`` | the edges within this distance of a point are its candidates |
    | ``sigma_m`` | the GPS error (metres) |
    | ``beta_m`` | how much longer than the straight line a route between two points can be |
    | ``uturn_m`` | the extra metres of a U-turn onto the other direction of the same road |
    | ``method`` | ``"hmm"`` (default): the hidden Markov model. ``"dtw"``: GPS-DTW (dynamic time warping, the gps-dtw crate); it needs ``time``, and ``sigma_m``, ``beta_m`` and ``uturn_m`` do not apply. On 600 Monaco traces, GPS-DTW is faster at each GPS interval and finds far fewer wrong roads with dense GPS (1 to 5 s); the HMM finds more of the true roads, and is better with sparse GPS (15 to 30 s) |
    | ``window_s`` | ``method="dtw"``: the seconds of one DTW window (0: the whole trace in one window) |

    Returns :class:`Match`: ``points`` (the rows with ``edge_id``, ``fraction``, ``distance_m``, ``snap_x``,
    ``snap_y``; no ``edge_id`` when no edge is within ``radius_m``) and ``edges`` (``trace_id``, ``seq``, ``edge_id``:
    the route, including the edges between two points). A gap that no route crosses starts the trace again.

    >>> m = duckosm.mobility.match(net.driving, gps)
    >>> net.map("driving", highlight=m.edges.edge_id)"""
    import pandas as pd
    df, xs, ys = _xy(traces, x, y)
    tcol = trace if trace in df else None
    keys = [c for c in (tcol, time if time in df else None) if c]
    df = df.assign(_x=xs, _y=ys)
    if keys:
        df = df.sort_values(keys, kind="stable").reset_index(drop=True)
    xs, ys = df.pop("_x").to_numpy(), df.pop("_y").to_numpy()
    ids = pd.factorize(df[tcol])[0] if tcol else [0] * len(df)
    db, mode = _db(view, mode)
    ts = None
    if method == "dtw":
        if time not in df:
            raise DuckOSMError(f'method="dtw" needs the time of each point: no column {time!r} (give time=)')
        t = df[time]
        if not pd.api.types.is_numeric_dtype(t):                  # a time: whole seconds since 1970 (any unit of datetime64)
            t = (pd.to_datetime(t, utc=True) - pd.Timestamp("1970-01-01", tz="UTC")) / pd.Timedelta("1s")
        ts = t.round().astype("int64").tolist()
    cols, (rt, re) = _rust().mobility_match(db, mode, list(map(int, ids)), xs.tolist(), ys.tolist(), float(radius_m),
                                            float(sigma_m), float(beta_m), 4.0, float(uturn_m), method, ts, int(window_s))
    points = _columns(df, cols)
    labels = df[tcol].drop_duplicates().to_numpy() if tcol else [0]
    edges = pd.DataFrame({"trace_id": [labels[i] for i in rt], "edge_id": pd.array(re, dtype="int64")})
    edges.insert(1, "seq", edges.groupby("trace_id", sort=False).cumcount())
    return Match(points, edges)


def join(view, data, on: str = "edge_id", how: str = "inner", mode: str | None = None):
    """Your data on the edges: a GeoDataFrame of the edges with the columns of ``data`` (a DataFrame with an
    ``edge_id`` column, or a Series keyed by ``edge_id``). ``how="left"``: all edges, with NaN where there is no data.

    >>> duckosm.mobility.join(net.driving, counts)"""
    import pandas as pd
    db, mode = resolve(view, mode)
    net = db if isinstance(db, Network) else Network(db)
    if isinstance(data, pd.Series):
        data = data.rename(data.name or "value").rename_axis(on).reset_index()
    if on not in data:
        raise DuckOSMError(f"the data has no column {on!r}")
    return net.edges(mode).merge(data, on=on, how=how)


def aggregate(view, matched, by: str = "edge_id", time: str | None = "hour", *, time_column: str = "time",
              value: str | None = None, mode: str | None = None):
    """Counts (and the mean of ``value``) for each edge and time bin. ``matched``: the ``points`` of :func:`match` or the
    result of :func:`snap`. ``time``: the bin (``"hour"``, ``"15min"``, ``"day"`` or any pandas frequency; None: no time
    bins). Returns a DataFrame with ``edge_id``, ``time`` (the start of the bin), ``count`` and ``<value>_mean``.

    >>> duckosm.mobility.aggregate(net.driving, m.points, time="hour", value="speed_kmh")"""
    import pandas as pd
    resolve(view, mode)                          # the mode rules, as the other calls
    df = matched.dropna(subset=[by])
    keys = [by]
    if time is not None:
        if time_column not in df:
            raise DuckOSMError(f"no column {time_column!r}: give time_column=, or time=None")
        freq = {"hour": "h", "day": "D"}.get(time, time)
        df = df.assign(**{"time": pd.to_datetime(df[time_column]).dt.floor(freq)})
        keys.append("time")
    g = df.groupby(keys)
    out = g.size().rename("count").to_frame()
    if value is not None:
        out[f"{value}_mean"] = g[value].mean()
    return out.reset_index()


def edge_id_map(old_net, new_net, mode: str | None = None, tol_m: float = 2.0):
    """The ``edge_id`` of an old build on the edges of a new build of the same area (after an OSM update or a change of
    the fixes). An edge id that is in both builds maps to itself. Another old edge maps to the new edges of the same OSM
    way that lie on it in the same direction (within ``tol_m``), with the ``share`` of the old edge on each. Returns a
    DataFrame: ``old_edge_id``, ``new_edge_id``, ``share``. An old edge with no row has no new edge (the road went).

    >>> m = duckosm.mobility.edge_id_map(duckosm.open("old.duckdb"), net, mode="driving")
    >>> data.merge(m, left_on="edge_id", right_on="old_edge_id")"""
    import pandas as pd
    old_db, mode = resolve(old_net, mode)
    new_db, mode2 = resolve(new_net, mode)
    if mode != mode2:
        raise DuckOSMError(f"the two networks are different modes: {mode} and {mode2}")
    o, n, s = _rust().mobility_edge_id_map(str(Path(os.fspath(old_db))), str(Path(os.fspath(new_db))), mode,
                                           float(tol_m))
    return pd.DataFrame({"old_edge_id": pd.array(o, dtype="int64"), "new_edge_id": pd.array(n, dtype="int64"),
                         "share": s})
