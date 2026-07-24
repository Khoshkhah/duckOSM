"""Elevation enrichment — sample a DEM at every node of a built duckOSM db.

A **post-processing** step (not part of the build pipeline): opens a finished db
read-write and adds, in place,

  - ``<mode>.nodes.ele``            ground elevation (m) at each node
  - ``<mode>.edges.z_from/z_to``    endpoint ground elevation (m)
  - ``main.elevation_metadata``     one-row provenance record

Two ways to supply the DEM:

  * ``dem=<path|url>``  — any GDAL-readable raster (GeoTIFF/COG/VRT/.img/.asc/.hgt …).
  * ``source=<name>``   — a global provider, auto-fetched for the db's node bbox:
      - ``auto`` (default) — pick the best provider whose coverage contains the bbox:
        EU-DTM (bare-earth) inside Europe *if* an OpenTopography API key is set, else
        Copernicus GLO-30 (global, anonymous). No country logic — a coverage test.
      - ``copernicus``     — Copernicus GLO-30, streamed from AWS over /vsicurl (no auth).
      - ``eudtm``          — Continental-Europe bare-earth DTM 30 m via OpenTopography
        (needs the ``OPENTOPOGRAPHY_API_KEY`` env var; Europe only).

Needs the optional deps ``rasterio`` + ``pyproj`` (``pip install 'duckosm[elevation]'``).
"""
from __future__ import annotations

import logging
import math
import os
from collections import defaultdict
from datetime import datetime, timezone
from pathlib import Path

import duckdb

logger = logging.getLogger("duckosm")

# Continental-Europe bounding box (w, s, e, n) — the coverage of EU_DTM, generous enough for
# EEA-39 incl. Scandinavia & Iceland. `auto` prefers EU-DTM only when the db bbox is FULLY inside.
EUROPE_BBOX = (-25.0, 34.0, 45.0, 72.0)


def _covers_world(bbox):
    return True


def _covers_europe(bbox):
    w, s, e, n = bbox
    W, S, E, N = EUROPE_BBOX
    return W <= w and e <= E and S <= s and n <= N


def _usable_always():
    return True


def _usable_opentopo():
    return bool(os.environ.get("OPENTOPOGRAPHY_API_KEY"))


# Global DEMs usable via --source. Each entry carries the descriptive metadata that fills
# main.elevation_metadata for free, plus a coverage predicate + usability check so `auto` can pick
# per area. `priority` (higher first) breaks ties; copernicus is the world-wide, always-usable
# fallback. ponytail: two providers is enough — add a row (coverage + fetch kind) for more.
PROVIDERS = {
    "copernicus": {
        "source": "Copernicus GLO-30", "uri": "s3://copernicus-dem-30m", "resolution_m": 30.0,
        "product": "DSM", "dem_crs": "EPSG:4326", "vertical_datum": "EGM2008",
        "license": "Copernicus open (attribution)",
        "kind": "tiles", "priority": 0, "covers": _covers_world, "usable": _usable_always,
    },
    "eudtm": {
        "source": "Continental Europe DTM 30m (EU_DTM)",
        "uri": "https://portal.opentopography.org/API/globaldem?demtype=EU_DTM",
        "resolution_m": 30.0, "product": "DTM", "dem_crs": "EPSG:4326",
        "vertical_datum": "EGM2008", "license": "EU-DTM via OpenTopography (see provider terms)",
        "kind": "opentopo", "demtype": "EU_DTM", "priority": 10,
        "covers": _covers_europe, "usable": _usable_opentopo,
    },
}

_COP_BASE = "https://copernicus-dem-30m.s3.amazonaws.com"
_OPENTOPO_API = "https://portal.opentopography.org/API/globaldem"


def _fix_polluted_proj():
    """Drop a PROJ_LIB/PROJ_DATA that another package pointed at its own, GDAL-incompatible
    proj.db — notably eclipse-sumo, whose ``import sumo`` sets these to a bundled proj.db that then
    makes rasterio fail *every* EPSG lookup in the same process. Cleared only when the path is that
    kind of bundled db (inside a ``sumo`` package dir), so a user's legitimate PROJ_DATA is left
    alone. Must run before the first CRS op — GDAL caches the PROJ context on first touch. A no-op
    in a clean process (the vars are unset), which is the normal `duckosm elevation` case."""
    for var in ("PROJ_LIB", "PROJ_DATA"):
        p = os.environ.get(var)
        if p and f"{os.sep}sumo{os.sep}" in f"{p}{os.sep}":
            os.environ.pop(var, None)


def _copernicus_url(lat_floor: int, lon_floor: int) -> str:
    """/vsicurl URL of the GLO-30 tile whose SW corner is (lat_floor, lon_floor)."""
    ns, ew = ("N" if lat_floor >= 0 else "S"), ("E" if lon_floor >= 0 else "W")
    name = (f"Copernicus_DSM_COG_10_{ns}{abs(lat_floor):02d}_00_"
            f"{ew}{abs(lon_floor):03d}_00_DEM")
    return f"/vsicurl/{_COP_BASE}/{name}/{name}.tif"


def _resolve_provider(source, bbox):
    """Turn a --source name into a concrete provider key. 'auto' → the highest-priority provider
    whose coverage contains `bbox` and which is usable (credentials present); always resolvable
    because copernicus covers the world and needs no auth."""
    if source != "auto":
        if source not in PROVIDERS:
            raise ValueError(f"unknown --source '{source}'; available: "
                             f"{', '.join(['auto', *PROVIDERS])} (or pass --dem <file>)")
        return source
    for name in sorted(PROVIDERS, key=lambda k: -PROVIDERS[k]["priority"]):
        p = PROVIDERS[name]
        if p["covers"](bbox) and p["usable"]():
            return name
    return "copernicus"                                     # guaranteed fallback


def _opentopo_fetch(demtype, bbox, api_key, pad=0.02):
    """Download a bbox-clipped GeoTIFF from the OpenTopography Global DEM API to a temp file."""
    import tempfile
    import urllib.parse
    import urllib.request
    w, s, e, n = bbox
    q = urllib.parse.urlencode({"demtype": demtype, "south": s - pad, "north": n + pad,
                                "west": w - pad, "east": e + pad, "outputFormat": "GTiff",
                                "API_Key": api_key})
    fd, path = tempfile.mkstemp(suffix=f"_{demtype}.tif")
    os.close(fd)
    logger.info(f"  fetching {demtype} from OpenTopography for bbox "
                f"{round(w,2)},{round(s,2)},{round(e,2)},{round(n,2)} …")
    urllib.request.urlretrieve(f"{_OPENTOPO_API}?{q}", path)  # small clipped raster; stdlib
    return path


def _clean(vals, nodata, out, filled, idxs):
    """Write cleaned sample values into out[idxs]; mark voids/non-finite in `filled`."""
    for j, v in zip(idxs, vals):
        z = float(v[0])
        if (nodata is not None and z == nodata) or not math.isfinite(z):
            continue                      # leave out[j] at its nodata_fill; filled[j] stays True
        out[j], filled[j] = z, False


def _dem_resolution_m(ds):
    res = min(abs(ds.res[0]), abs(ds.res[1]))
    if ds.crs is not None and ds.crs.is_geographic:
        res *= 111320.0                                     # degrees -> ~metres
    return round(res, 3)


def _file_sampler(ds, nodata_fill):
    """sample(lons, lats) -> (ele, filled) for an open rasterio dataset (any CRS, any format)."""
    import numpy as np
    from pyproj import Transformer

    nodata = ds.nodata
    left, bottom, right, top = ds.bounds
    if ds.crs is None:
        tf = None
    else:
        epsg = ds.crs.to_epsg()
        tf = Transformer.from_crs("EPSG:4326", f"EPSG:{epsg}" if epsg else ds.crs.to_wkt(),
                                  always_xy=True)

    def sample(lons, lats):
        xs, ys = np.asarray(lons, float), np.asarray(lats, float)
        if tf is not None:
            xs, ys = tf.transform(xs, ys)
        xs, ys = np.asarray(xs), np.asarray(ys)
        out = np.full(len(xs), float(nodata_fill))
        filled = np.ones(len(xs), bool)
        inb = (xs >= left) & (xs <= right) & (ys >= bottom) & (ys <= top)
        idxs = np.nonzero(inb)[0]
        if len(idxs):
            _clean(ds.sample(zip(xs[idxs], ys[idxs])), nodata, out, filled, idxs)
        return out, filled

    return sample


def _make_sampler(dem, provider, nodata_fill, bbox):
    """Return (sample_fn, meta, closer). `dem` wins; else `provider` is a resolved PROVIDERS key."""
    import numpy as np
    import rasterio

    if dem:
        ds = rasterio.open(dem)
        meta = {"source": Path(dem).name, "source_type": "file", "uri": str(Path(dem).resolve()),
                "resolution_m": _dem_resolution_m(ds), "product": "unknown",
                "dem_crs": ds.crs.to_string() if ds.crs else "unknown",
                "vertical_datum": "unknown", "license": "unknown"}
        return _file_sampler(ds, nodata_fill), meta, ds.close

    p = PROVIDERS[provider]

    if p["kind"] == "opentopo":                             # fetch a clipped GeoTIFF, then sample it
        path = _opentopo_fetch(p["demtype"], bbox, os.environ["OPENTOPOGRAPHY_API_KEY"])
        try:
            ds = rasterio.open(path)
        except Exception as e:
            try:
                os.unlink(path)
            except OSError:
                pass
            raise RuntimeError(f"{provider}: OpenTopography returned no usable raster for this bbox "
                               f"(check API key / coverage) — {e}") from e
        meta = {"source": p["source"], "source_type": "download", "uri": p["uri"],
                "resolution_m": p["resolution_m"], "product": p["product"],
                "dem_crs": ds.crs.to_string() if ds.crs else p["dem_crs"],
                "vertical_datum": p["vertical_datum"], "license": p["license"]}

        def closer():
            ds.close()
            try:
                os.unlink(path)
            except OSError:
                pass

        return _file_sampler(ds, nodata_fill), meta, closer

    # kind == "tiles": copernicus GLO-30, EPSG:4326 tiles → sample lon/lat directly, per tile.
    def sample(lons, lats):
        lons, lats = np.asarray(lons, float), np.asarray(lats, float)
        out = np.full(len(lons), float(nodata_fill))
        filled = np.ones(len(lons), bool)
        groups = defaultdict(list)
        for i in range(len(lons)):
            groups[(int(math.floor(lats[i])), int(math.floor(lons[i])))].append(i)
        for (latf, lonf), idxs in groups.items():
            try:
                ds = rasterio.open(_copernicus_url(latf, lonf))
            except Exception:
                logger.warning(f"  {provider} tile missing at {latf},{lonf} (ocean/void?) "
                               f"— {len(idxs)} node(s) get nodata_fill")
                continue
            with ds:
                _clean(ds.sample([(lons[i], lats[i]) for i in idxs]), ds.nodata, out, filled, idxs)
        return out, filled

    meta = {"source_type": "download", **{k: p[k] for k in
            ("source", "uri", "resolution_m", "product", "dem_crs", "vertical_datum", "license")}}
    return sample, meta, None


def _mode_schemas(con, modes):
    present = [r[0] for r in con.execute(
        "SELECT DISTINCT schema_name FROM duckdb_tables() WHERE table_name = 'edges' "
        "AND schema_name NOT IN ('information_schema', 'pg_catalog', 'main', 'raw') "
        "ORDER BY schema_name").fetchall()]
    chosen = list(modes) if modes else present
    if not chosen:
        raise ValueError("no mode schemas with an 'edges' table in this db")
    unknown = [m for m in chosen if m not in present]
    if unknown:
        raise ValueError(f"mode(s) {unknown} not found (present: {present or 'none'})")
    return chosen


def _db_bbox(con, modes):
    """(w, s, e, n) over all nodes in the chosen mode schemas."""
    w = s = e = n = None
    for m in modes:
        r = con.execute(f"SELECT min(ST_X(geom)), min(ST_Y(geom)), max(ST_X(geom)), "
                        f"max(ST_Y(geom)) FROM {m}.nodes WHERE geom IS NOT NULL").fetchone()
        if r[0] is None:
            continue
        w = r[0] if w is None else min(w, r[0])
        s = r[1] if s is None else min(s, r[1])
        e = r[2] if e is None else max(e, r[2])
        n = r[3] if n is None else max(n, r[3])
    if w is None:
        raise ValueError("no node geometry to sample")
    return (w, s, e, n)


def _write_metadata(con, meta, nodata_fill, n_nodes, n_nodata):
    from duckosm import __version__
    con.execute("""CREATE OR REPLACE TABLE main.elevation_metadata(
        source VARCHAR, source_type VARCHAR, uri VARCHAR, resolution_m DOUBLE, product VARCHAR,
        dem_crs VARCHAR, vertical_datum VARCHAR, license VARCHAR, nodata_fill DOUBLE,
        n_nodes BIGINT, n_nodata BIGINT, sampled_at VARCHAR, duckosm_version VARCHAR)""")
    con.execute("INSERT INTO main.elevation_metadata VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?)", [
        meta["source"], meta["source_type"], meta["uri"], meta["resolution_m"], meta["product"],
        meta["dem_crs"], meta["vertical_datum"], meta["license"], float(nodata_fill),
        n_nodes, n_nodata, datetime.now(timezone.utc).isoformat(timespec="seconds"), __version__])


def to_elevation(db, dem=None, source="auto", modes=None, nodata_fill=0.0):
    """Add elevation to a built duckOSM db in place. Returns a stats dict.

    Args:
        db: path to a built duckOSM .duckdb (modified in place, read-write).
        dem: a local/remote raster (any GDAL format). Takes precedence over ``source``.
        source: ``auto`` (default) picks a provider by the db's bbox; or a provider name
            (``copernicus`` / ``eudtm``). Ignored when ``dem`` is given.
        modes: mode schema(s) to enrich (default: every mode present).
        nodata_fill: value written where the DEM has a void / doesn't cover the node.
    """
    _fix_polluted_proj()                                    # before any rasterio/pyproj CRS op
    try:
        import numpy  # noqa: F401
        import pyproj  # noqa: F401
        import rasterio  # noqa: F401
    except ImportError as e:
        raise RuntimeError("elevation needs rasterio + pyproj — "
                           "pip install 'duckosm[elevation]'") from e

    con = duckdb.connect(str(db))                           # read-write: enriches in place
    con.execute("INSTALL spatial; LOAD spatial;")
    try:
        chosen = _mode_schemas(con, modes)
        bbox = _db_bbox(con, chosen)
        provider = None
        if not dem:
            provider = _resolve_provider(source, bbox)
            if source == "auto":
                logger.info(f"  source=auto → {provider} "
                            f"(bbox {', '.join(f'{v:.2f}' for v in bbox)})")
        sample, meta, closer = _make_sampler(dem, provider, nodata_fill, bbox)
    except Exception:
        con.close()
        raise
    try:
        per_mode, total_nodes, total_nodata = {}, 0, 0
        for m in chosen:
            rows = con.execute(
                f"SELECT node_id, ST_X(geom), ST_Y(geom) FROM {m}.nodes WHERE geom IS NOT NULL"
            ).fetchall()
            if not rows:
                per_mode[m] = {"nodes": 0, "nodata": 0}
                continue
            ids = [r[0] for r in rows]
            eles, filled = sample([r[1] for r in rows], [r[2] for r in rows])
            n_nd = int(filled.sum())

            # ADD IF NOT EXISTS (not drop-then-add): DuckDB blocks DROP/ALTER-COLUMN on a table
            # that has any index, but ADD is allowed. The column is DOUBLE from the start, so a
            # fresh db is 2 dp-clean; a re-run keeps the DOUBLE column and just overwrites values.
            con.execute(f"ALTER TABLE {m}.nodes ADD COLUMN IF NOT EXISTS ele DOUBLE")
            con.execute("CREATE OR REPLACE TEMP TABLE _ele(node_id BIGINT, ele DOUBLE)")
            # store to 2 dp (cm) — DEM accuracy is metre-scale, so more digits are false precision
            con.executemany("INSERT INTO _ele VALUES (?, ?)",
                            list(zip(ids, (round(float(e), 2) for e in eles))))
            con.execute(f"UPDATE {m}.nodes n SET ele = "
                        f"(SELECT ele FROM _ele t WHERE t.node_id = n.node_id)")
            con.execute(f"ALTER TABLE {m}.edges ADD COLUMN IF NOT EXISTS z_from DOUBLE")
            con.execute(f"ALTER TABLE {m}.edges ADD COLUMN IF NOT EXISTS z_to DOUBLE")
            con.execute(f"UPDATE {m}.edges e SET "
                        f"z_from = (SELECT ele FROM {m}.nodes n WHERE n.node_id = e.source), "
                        f"z_to   = (SELECT ele FROM {m}.nodes n WHERE n.node_id = e.target)")

            per_mode[m] = {"nodes": len(ids), "nodata": n_nd}
            total_nodes += len(ids)
            total_nodata += n_nd
            logger.info(f"  [{m}] {len(ids)} nodes sampled ({n_nd} nodata/fill)")

        _write_metadata(con, meta, nodata_fill, total_nodes, total_nodata)
        con.execute("CHECKPOINT")
    finally:
        if closer is not None:
            closer()
        con.close()
    return {"source": meta["source"], "modes": per_mode,
            "n_nodes": total_nodes, "n_nodata": total_nodata}
