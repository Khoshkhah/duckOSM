"""GTFS stops for the GMNS ``location`` table (docs/exports/gmns_tables.md).

``read_stops(paths)`` reads the stops of one or more GTFS feeds (a ``.zip`` or an unzipped folder) as a table
with a ``loc_type`` in OpenStreetMap's words and whether a bus serves the stop, from the routes that call there.
Only ``stops.txt`` is needed; with ``stop_times.txt``, ``trips.txt`` and ``routes.txt`` the stops are told apart
by the kind of service (a tram or train stop is no bus stop and has no place on a road).
"""
import os
import tempfile
import zipfile

# GTFS route_type -> OpenStreetMap-style name (basic types 0-12, and the extended ones by their hundreds)
_BASIC = {0: "tram_stop", 1: "subway_station", 2: "train_station", 3: "bus_stop", 4: "ferry_terminal",
          5: "tram_stop", 6: "cable_car_station", 7: "funicular_station", 11: "bus_stop", 12: "train_station"}
_EXTENDED = {1: "train_station", 2: "bus_stop", 4: "subway_station", 7: "bus_stop", 9: "tram_stop",
             10: "ferry_terminal"}                       # 100s rail, 200s coach, 400s urban rail, 700s bus, ...
_BUS = {"bus_stop"}
_FILES = ("stops.txt", "stop_times.txt", "trips.txt", "routes.txt")


def _kind(route_type):
    """The OSM-style name of the stop a route type calls at."""
    t = int(route_type)
    return _BASIC.get(t) or (_EXTENDED.get(t // 100) if t >= 100 else None) or "transit_stop"


def _unpack(path, into):
    """The GTFS files of ``path`` (a zip or a folder), as paths under ``into`` or the folder itself."""
    if os.path.isdir(path):
        return path
    if not zipfile.is_zipfile(path):
        raise ValueError(f"{path}: not a GTFS zip file or folder")
    with zipfile.ZipFile(path) as z:
        names = {os.path.basename(n): n for n in z.namelist() if os.path.basename(n) in _FILES}
        if "stops.txt" not in names:
            raise ValueError(f"{path}: no stops.txt: not a GTFS feed")
        for base, n in names.items():
            with z.open(n) as src, open(os.path.join(into, base), "wb") as dst:
                dst.write(src.read())
    return into


def read_stops(paths):
    """The stops of the GTFS feeds ``paths`` (zip files or folders): a DataFrame ``stop_id``, ``name``, ``lat``,
    ``lon``, ``loc_type`` (``bus_stop``, ``tram_stop``, ``train_station``, ``entrance``, ...) and ``is_bus``.
    Stops (``location_type`` 0 or empty) and station entrances (2) only: stations (1) and boarding areas (4) group
    stops, they are not places to board. A stop no trip calls at is ``transit_stop``."""
    import duckdb
    import pandas as pd

    frames = []
    for path in ([paths] if isinstance(paths, (str, os.PathLike)) else list(paths)):
        with tempfile.TemporaryDirectory() as tmp:
            d = _unpack(str(path), tmp)
            have = {f for f in _FILES if os.path.exists(os.path.join(d, f))}
            if "stops.txt" not in have:
                raise ValueError(f"{path}: no stops.txt: not a GTFS feed")
            con = duckdb.connect()
            rd = lambda f: f"read_csv('{os.path.join(d, f)}', all_varchar = true, header = true, " \
                           f"null_padding = true, ignore_errors = true)"   # noqa: E731
            cols = {r[0] for r in con.execute(f"DESCRIBE SELECT * FROM {rd('stops.txt')}").fetchall()}
            lt = "location_type" if "location_type" in cols else "NULL"
            stops = con.execute(f"""SELECT stop_id, stop_name AS name, stop_lat::DOUBLE AS lat, stop_lon::DOUBLE AS lon,
                coalesce(try_cast({lt} AS INTEGER), 0) AS ltype FROM {rd('stops.txt')}
                WHERE try_cast(stop_lat AS DOUBLE) IS NOT NULL AND try_cast(stop_lon AS DOUBLE) IS NOT NULL
                  AND coalesce(try_cast({lt} AS INTEGER), 0) IN (0, 2)""").df()
            kinds = {}
            if {"stop_times.txt", "trips.txt", "routes.txt"} <= have:
                for stop_id, types in con.execute(f"""SELECT st.stop_id, list(DISTINCT try_cast(r.route_type AS INTEGER))
                    FROM {rd('stop_times.txt')} st JOIN {rd('trips.txt')} t ON t.trip_id = st.trip_id
                    JOIN {rd('routes.txt')} r ON r.route_id = t.route_id GROUP BY 1""").fetchall():
                    names = {_kind(x) for x in types if x is not None}
                    kinds[stop_id] = "bus_stop" if names & _BUS else (sorted(names)[0] if names else "transit_stop")
                default = "transit_stop"
            else:
                default = "bus_stop"                       # no service data: assume a bus network
            stops["loc_type"] = [("entrance" if lt_ == 2 else kinds.get(sid, default))
                                 for sid, lt_ in zip(stops.stop_id, stops.ltype)]
            stops["is_bus"] = stops.loc_type == "bus_stop"
            frames.append(stops.drop(columns="ltype"))
            con.close()
    return pd.concat(frames, ignore_index=True) if frames else pd.DataFrame(
        columns=["stop_id", "name", "lat", "lon", "loc_type", "is_bus"])
