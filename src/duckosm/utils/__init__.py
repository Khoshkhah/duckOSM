# DuckDB names a database after its file, so a file called like one of duckOSM's schemas makes
# every `driving.edges`-style reference ambiguous ("Ambiguous reference to catalog or schema").
RESERVED_DB_NAMES = ("driving", "walking", "cycling", "mm", "raw", "features", "main")


def check_db_name(path):
    """Raise ValueError when a database file is named like a duckOSM schema (``driving.duckdb``)."""
    from pathlib import Path
    stem = Path(path).stem
    if stem.lower() in RESERVED_DB_NAMES:
        raise ValueError(f"a database can't be called {Path(path).name}: DuckDB names a database after "
                         f"its file, and '{stem}' is also one of duckOSM's schemas "
                         f"({', '.join(RESERVED_DB_NAMES)}). Rename it, e.g. {stem}_network.duckdb")


def utm_epsg(lon, lat):
    """EPSG code of the WGS84 UTM zone containing (lon, lat), e.g. 'EPSG:32632' for Monaco."""
    return f"EPSG:{(32600 if lat >= 0 else 32700) + int((lon + 180) // 6) % 60 + 1}"


def data_utm_crs(con, table, geom="geom"):
    """A metric CRS that fits the data wherever it is: the UTM zone of ``table``'s centre."""
    lon, lat = con.execute(f"SELECT avg(ST_X(ST_Centroid({geom}))), avg(ST_Y(ST_Centroid({geom}))) "
                           f"FROM {table} WHERE {geom} IS NOT NULL").fetchone()
    return utm_epsg(lon, lat)


def add_timezone(con, table="main.visualization_metadata"):
    """Store the IANA time zone of the area (e.g. 'Europe/Stockholm') in ``table``.

    Looked up at a real network node, the one nearest the median of all nodes, not at the bounding
    box's centre: for a coastal or odd-shaped extract that centre can sit at sea or across a border
    (Geofabrik's Monaco extract: 20 km offshore, in France's zone). Required, not best-effort: hourly
    traffic data needs the local zone, so this raises when it can't be determined."""
    from timezonefinder import TimezoneFinder

    mode = con.execute("SELECT table_schema FROM information_schema.tables WHERE table_name = 'nodes' "
                       "AND table_schema IN ('driving', 'walking', 'cycling') "
                       "ORDER BY table_schema = 'driving' DESC LIMIT 1").fetchone()
    if not mode:
        raise RuntimeError("cannot set the time zone: no <mode>.nodes table")
    lon, lat = con.execute(f"""
        WITH n AS (SELECT ST_X(geom) AS x, ST_Y(geom) AS y FROM {mode[0]}.nodes WHERE node_id > 0),
             m AS (SELECT median(x) AS mx, median(y) AS my FROM n)
        SELECT x, y FROM n, m ORDER BY (x - mx) * (x - mx) + (y - my) * (y - my) LIMIT 1""").fetchone()
    tz = TimezoneFinder().timezone_at(lat=lat, lng=lon)
    if not tz:
        raise RuntimeError(f"no time zone found at lat={lat}, lon={lon}")
    con.execute(f"ALTER TABLE {table} ADD COLUMN IF NOT EXISTS timezone VARCHAR")
    con.execute(f"UPDATE {table} SET timezone = ?", [tz])
    return tz
