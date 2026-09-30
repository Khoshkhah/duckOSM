"""
GIS exporter — write a built duckOSM network to the universal GIS interchange formats,
**GeoPackage** (primary) or **ESRI shapefile** (fallback), preserving the stable ``edge_id``.

This is the widest-reach export: it hands the network to the whole GIS world (QGIS / ArcGIS / FME
and any tool with a GIS importer — Aimsun, PTV Visum, …) with no DuckDB knowledge required. Only the
*geographic* tables are written — ``<mode>.edges`` (layer ``edges_<mode>``), ``<mode>.nodes``
(``nodes_<mode>``) and ``main.boundary`` (``boundary``). The routing adjacency (``edge_graph`` /
``turn_restrictions``) is a topology GIS tools can't act on and belongs in the NetworkX / SUMO
exports, not here. See https://khoshkhah.github.io/duckOSM/exports/gis/ for the format spec.

    import duckdb
    from duckosm.gis import to_gis
    con = duckdb.connect("data/db/sodermalm.duckdb", read_only=True)
    to_gis(con, "sodermalm.gpkg")                 # GeoPackage: every mode present + boundary
    to_gis(con, "gis/", fmt="shp", modes=["driving"])  # one shapefile set per layer into gis/

GeoPackage is multi-layer, but DuckDB's ``COPY`` writes one layer per file (named after the file)
and overwrites on re-copy — so each layer is written to a temp single-layer ``.gpkg`` and assembled
into the one target with ``ogr2ogr -update -append -nln <layer>`` (the same GDAL dependency the
``duckosm admin --gpkg`` export uses). Without ``ogr2ogr`` it falls back to one file per layer.
Shapefiles need no assembly (one layer per file by nature).
"""
import logging
import os
import shutil
import subprocess
import tempfile

logger = logging.getLogger("duckosm")

# Shapefile field names are capped at 10 chars — rename the few that overflow so they stay
# meaningful instead of being silently truncated (GeoPackage keeps the full names).
_SHP_RENAME = {
    "maxspeed_kmh": "spd_kmh",
    "is_reverse": "is_rev",
    "admin_level": "adm_level",
}


def _mode_schemas(con):
    """Mode schemas that have an ``edges`` table (mirrors the `viz` command's discovery)."""
    return [r[0] for r in con.execute(
        "SELECT DISTINCT schema_name FROM duckdb_tables() "
        "WHERE table_name = 'edges' "
        "AND schema_name NOT IN ('information_schema', 'pg_catalog', 'main', 'raw') "
        "ORDER BY schema_name").fetchall()]


def _table_exists(con, schema, table):
    return con.execute(
        "SELECT count(*) FROM information_schema.tables "
        "WHERE table_schema = ? AND table_name = ?", [schema, table]).fetchone()[0] > 0


def _select_sql(con, schema, table, for_shp):
    """A ``SELECT`` over ``schema.table`` keeping every scalar + the geometry column, dropping list /
    nested columns (e.g. ``refs BIGINT[]`` — the shapefile format can't hold them and they're
    routing-internal).

    For shapefiles two format defects are worked around: (1) over-long field names are aliased via
    :data:`_SHP_RENAME` (10-char DBF cap), and (2) 64-bit integers are cast to text — a DBF numeric
    field can't hold an int64 exactly, so GDAL would otherwise write ``edge_id`` as a float and
    **silently corrupt it** (a 19-digit content hash exceeds a float64's ~15 exact digits). Casting
    the id columns to VARCHAR keeps them exact. GeoPackage stores Integer64 natively — no cast."""
    cols = con.execute(
        "SELECT column_name, data_type FROM information_schema.columns "
        "WHERE table_schema = ? AND table_name = ? ORDER BY ordinal_position",
        [schema, table]).fetchall()
    parts = []
    for name, dtype in cols:
        d = dtype.upper()
        if d.endswith("[]") or "STRUCT" in d or "MAP" in d:     # nested → GDAL/shp can't store it
            continue
        if name.upper() == "OGC_FID":                           # GDAL-reserved feature-id name
            continue                                            # (synthetic ST_Read artifact, not data)
        alias = _SHP_RENAME.get(name, name) if for_shp else name
        expr = f'"{name}"'
        if for_shp and ("BIGINT" in d or "HUGEINT" in d):       # keep int64 ids exact as text
            expr = f'CAST("{name}" AS VARCHAR)'
        if name in ("length_m", "cost_s") and ("DOUBLE" in d or "FLOAT" in d):
            # cm / centisecond accuracy suffices; CAST first — ROUND on a FLOAT stays FLOAT,
            # whose base-2 noise (57.849998) would leak into the written file
            expr = f'ROUND(CAST({expr} AS DOUBLE), 2)'
        parts.append(f'{expr} AS "{alias}"')
    return f'SELECT {", ".join(parts)} FROM "{schema}"."{table}"'


def _copy(con, select_sql, out_file, driver, srs):
    """One DuckDB spatial ``COPY`` → a single-layer GDAL dataset (SRS written so a .prj/CRS lands)."""
    con.execute(f"COPY ({select_sql}) TO '{out_file}' "
                f"(FORMAT gdal, DRIVER '{driver}', SRS '{srs}')")


def _count(con, schema, table):
    return con.execute(f'SELECT count(*) FROM "{schema}"."{table}"').fetchone()[0]


def to_gis(con, out, modes=None, fmt: str = "gpkg", boundary: bool = True, name: str = None,
           srs: str = "EPSG:4326", ogr2ogr_bin: str = None):
    """Export the geographic network (edges + nodes [+ boundary]) to GeoPackage or shapefile.

    Parameters
    ----------
    con : DuckDB connection to a built duckOSM db (spatial extension loaded here).
    out : GeoPackage → the target ``.gpkg`` file. Shapefile → the target **directory**.
    modes : mode schemas to export (default: every mode present in the db).
    fmt : ``"gpkg"`` (default) or ``"shp"``.
    boundary : also export ``main.boundary`` as a ``boundary`` layer, if it exists (default True).
    name : basename for layers / shapefiles (default: the ``out`` file stem, else ``"network"``).
    srs : CRS written into every layer (default ``"EPSG:4326"`` — duckOSM geometries are lon/lat).
    ogr2ogr_bin : explicit ``ogr2ogr`` path (else auto-located; used only to assemble the GeoPackage).

    Returns ``{"fmt", "path"(gpkg)|"dir"(shp), "layers": {name: n_features}, "files": [...]}``.
    Every layer keeps ``edge_id`` as a plain attribute — the cross-format stable join key.
    """
    try:
        con.execute("INSTALL spatial; LOAD spatial;")
    except Exception:
        pass

    fmt = fmt.lower()
    if fmt not in ("gpkg", "shp"):
        raise ValueError(f"fmt must be 'gpkg' or 'shp', got {fmt!r}")

    present = _mode_schemas(con)
    chosen = list(modes) if modes else present
    if not chosen:
        raise ValueError("no mode schemas with an 'edges' table in the db")
    unknown = [m for m in chosen if m not in present]
    if unknown:
        raise ValueError(f"mode(s) {unknown} not found (present: {present or 'none'})")

    # layers: (layer_name, schema, table) — one edges + one nodes per mode, plus the shared boundary
    layers = []
    for m in chosen:
        layers.append((f"edges_{m}", m, "edges"))
        if _table_exists(con, m, "nodes"):
            layers.append((f"nodes_{m}", m, "nodes"))
    if boundary and _table_exists(con, "main", "boundary"):
        layers.append(("boundary", "main", "boundary"))

    base = name or (os.path.splitext(os.path.basename(out))[0] if fmt == "gpkg" else None) or "network"

    if fmt == "shp":
        return _export_shp(con, out, base, layers, srs)
    return _export_gpkg(con, out, base, layers, srs, ogr2ogr_bin)


def _export_shp(con, out_dir, base, layers, srs):
    """One shapefile set per layer (a .shp holds a single layer/geometry type)."""
    os.makedirs(out_dir, exist_ok=True)
    written, counts = [], {}
    for lyr, schema, table in layers:
        path = os.path.join(out_dir, f"{base}_{lyr}.shp")
        _copy(con, _select_sql(con, schema, table, for_shp=True), path, "ESRI Shapefile", srs)
        counts[lyr] = _count(con, schema, table)
        written.append(path)
        logger.info(f"GIS[shp]: {lyr} — {counts[lyr]:,} features -> {path}")
    return {"fmt": "shp", "dir": out_dir, "layers": counts, "files": written}


def _export_gpkg(con, out, base, layers, srs, ogr2ogr_bin):
    """Multi-layer GeoPackage assembled with ogr2ogr (fallback: one .gpkg per layer)."""
    ogr = ogr2ogr_bin or shutil.which("ogr2ogr")
    counts = {}

    if not ogr:
        # fallback: DuckDB writes one single-layer .gpkg per layer, alongside `out`
        logger.warning("ogr2ogr not found — writing one GeoPackage per layer (install GDAL for a "
                       "single multi-layer .gpkg)")
        out_dir = os.path.dirname(out) or "."
        os.makedirs(out_dir, exist_ok=True)
        written = []
        for lyr, schema, table in layers:
            path = os.path.join(out_dir, f"{base}_{lyr}.gpkg")
            _copy(con, _select_sql(con, schema, table, for_shp=False), path, "GPKG", srs)
            counts[lyr] = _count(con, schema, table)
            written.append(path)
            logger.info(f"GIS[gpkg]: {lyr} — {counts[lyr]:,} features -> {path}")
        return {"fmt": "gpkg", "path": None, "layers": counts, "files": written}

    out_dir = os.path.dirname(out) or "."
    os.makedirs(out_dir, exist_ok=True)
    if os.path.exists(out):
        os.remove(out)                      # start clean so -append builds a fresh, complete file
    with tempfile.TemporaryDirectory() as tmp:
        for i, (lyr, schema, table) in enumerate(layers):
            part = os.path.join(tmp, f"{lyr}.gpkg")
            _copy(con, _select_sql(con, schema, table, for_shp=False), part, "GPKG", srs)
            args = [ogr, "-f", "GPKG"]
            if os.path.exists(out):
                args += ["-update", "-append"]
            args += [out, part, "-nln", lyr]
            res = subprocess.run(args, capture_output=True, text=True)
            if res.returncode != 0:
                raise RuntimeError(f"ogr2ogr failed on layer {lyr} (exit {res.returncode}):\n"
                                   f"{res.stderr[-2000:]}")
            counts[lyr] = _count(con, schema, table)
            logger.info(f"GIS[gpkg]: {lyr} — {counts[lyr]:,} features")
    logger.info(f"wrote GeoPackage ({len(layers)} layers) -> {out}")
    return {"fmt": "gpkg", "path": out, "layers": counts, "files": [out]}
