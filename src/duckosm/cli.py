"""
CLI for duckOSM — a subcommand-based command line interface.

  duckosm build    build a routing network from a PBF, or clip one from a parent db
  duckosm extract  slice a sub-area out of an existing build into a new db
  duckosm admin    add OSM administrative boundaries to a built db
  duckosm viz      render a roadstyle HTML map of a built network
  duckosm sumo     export a built network to a SUMO net (edge_id preserved)
  duckosm export-graph  export a built network as a networkx graph file (GraphML / gpickle)
  duckosm export-gis    export a built network to GeoPackage / shapefile (edge_id preserved)
  duckosm gis-debug     read a GIS export back through GDAL and write an HTML debug/QA page
  duckosm gmns          extract a built network to a standalone GMNS DuckDB (lanes, movements, …)

The `extract`/`admin` subcommands wrap the matching scripts/ tools and forward their
arguments verbatim, so `duckosm extract --help` shows the full underlying options.
"""

import importlib.util
import logging
from datetime import datetime
from pathlib import Path

import click

from duckosm.config import Config
from duckosm.importer import DuckOSM

# scripts/ lives at the repo root (this package is normally installed editable from the
# checkout). The extract/admin subcommands load and delegate to those scripts.
_SCRIPTS_DIR = Path(__file__).resolve().parents[2] / "scripts"


def _load_script(filename):
    """Import scripts/<filename> by path and return the module (for delegation)."""
    path = _SCRIPTS_DIR / filename
    if not path.exists():
        raise click.ClickException(
            f"{path} not found. This subcommand wraps scripts/{filename}, which ships "
            "with the source checkout — run duckosm from the repo (pip install -e .)."
        )
    spec = importlib.util.spec_from_file_location(path.stem, path)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def setup_logging(name="duckosm"):
    """Configure logging to both console and a per-area file logs/<name>_<ts>.log."""
    log_dir = Path("logs")
    log_dir.mkdir(exist_ok=True)
    timestamp = datetime.now().strftime("%Y%m%d_%H%M%S")
    log_file = log_dir / f"{name}_{timestamp}.log"
    logging.basicConfig(
        level=logging.INFO,
        format='[%(asctime)s] [%(levelname)s] %(message)s',
        datefmt='%H:%M:%S',
        handlers=[logging.FileHandler(log_file), logging.StreamHandler()],
        force=True,
    )
    return log_file


@click.group(context_settings=dict(help_option_names=["-h", "--help"]))
@click.version_option(package_name="duckosm")
def main():
    """duckOSM — high-performance OSM-to-routing-network converter.

    Convert OSM PBF files to DuckDB routing databases. Run a subcommand below;
    use `duckosm <command> --help` for that command's options.
    """


@main.command()
@click.option('--config', '-c', type=click.Path(exists=True), help='Path to YAML configuration file')
@click.option('--pbf', '-p', type=click.Path(exists=True), help='Path to PBF file')
@click.option('--output', '-o', type=click.Path(), default='data/output/network.duckdb', help='Output DuckDB file path')
@click.option('--boundary', '-b', type=click.Path(exists=True), help='GeoJSON boundary file for filtering')
@click.option('--source-db', type=click.Path(exists=True), help='Parent duckOSM db to clip from (source.type=duckdb)')
@click.option('--h3-cell', help='H3 cell ID for filtering')
@click.option('--graph/--no-graph', default=True, help='Build edge graph table')
@click.option('--h3-index/--no-h3-index', default=True, help='Add H3 spatial indexing')
@click.option('--h3-resolution', type=int, default=8, help='H3 resolution (0-15)')
@click.option('--modes', '-m', multiple=True, help='Transportation modes (driving, walking, cycling)')
def build(config, pbf, output, boundary, source_db, h3_cell, graph, h3_index, h3_resolution, modes):
    """Build a routing network from a PBF, or clip one from a parent db.

    \b
    Examples:
        # From a YAML config
        duckosm build --config config/default.yaml
        # From CLI arguments
        duckosm build --pbf input.pbf --output network.duckdb --graph
    """
    # Default to config/default.yaml if it exists and no config/pbf/source given.
    default_config = Path("config/default.yaml")
    if not config and not pbf and not source_db and default_config.exists():
        config = str(default_config)

    try:
        if config:
            cfg = Config.from_yaml(config)
        elif pbf or source_db:
            cfg = Config.from_args(
                pbf_path=pbf or "",
                output_path=output,
                boundary_path=boundary,
                source_db=source_db,
                h3_cell=h3_cell,
                build_graph=graph,
                h3_indexing=h3_index,
                h3_resolution=h3_resolution,
                modes=list(modes) if modes else ["driving"],
            )
        else:
            raise click.ClickException(
                "provide --config, or --pbf, or --source-db "
                "(or config/default.yaml must exist)")
    except click.ClickException:
        raise
    except Exception as e:
        raise click.ClickException(f"loading config: {e}")

    setup_logging(cfg.name)
    try:
        output_path = DuckOSM(cfg).run()
        click.echo(f"\nOutput: {output_path}")
    except Exception as e:
        raise click.ClickException(str(e))


# extract/admin forward all arguments to an argparse-based script: don't let Click
# intercept unknown options or --help — the wrapped script parses and handles them.
_PASSTHROUGH = dict(context_settings=dict(ignore_unknown_options=True, help_option_names=[]),
                    add_help_option=False)


@main.command(**_PASSTHROUGH)
@click.argument('args', nargs=-1, type=click.UNPROCESSED)
def extract(args):
    """Slice a sub-area out of an existing build into a new, self-contained db.

    Wraps scripts/extract_area.py — run `duckosm extract --help` for options
    (--source, --db, and one of --name / --osm-id / --boundary).
    """
    raise SystemExit(_load_script("extract_area.py").main(list(args)))


@main.command(**_PASSTHROUGH)
@click.argument('args', nargs=-1, type=click.UNPROCESSED)
def admin(args):
    """Add OSM administrative boundaries (admin_boundaries table) to a built db.

    Wraps scripts/add_admin_boundaries.py — run `duckosm admin --help` for options
    (--pbf, --db, optional --gpkg). Requires ogr2ogr (GDAL).
    """
    raise SystemExit(_load_script("add_admin_boundaries.py").main(list(args)))


@main.command()
@click.argument('db', type=click.Path(exists=True))
@click.option('--mode', '-m', 'modes', multiple=True,
              help='Mode schema(s) to render (default: every mode present in the db)')
@click.option('--basemap', default='voyager', show_default=True,
              help='Default base map: voyager, positron, esri_gray, osm, satellite')
@click.option('--out-dir', default='reports', show_default=True,
              help='Output directory for <name>_<mode>_network.html')
@click.option('--arrows/--no-arrows', default=False, show_default=True,
              help='Overlay one-way direction arrows (gray chevrons, shown when zoomed in)')
@click.option('--boundary/--no-boundary', default=True, show_default=True,
              help='Overlay the clip/area boundary outline (main.boundary), if present')
def viz(db, modes, basemap, out_dir, arrows, boundary):
    """Render a roadstyle HTML map of a built network.

    Writes <out-dir>/<name>_<mode>_network.html per mode, with edges styled by
    highway class. Needs geopandas + roadstyle installed.
    """
    import duckdb

    from duckosm.viz import render_network

    logging.basicConfig(level=logging.INFO, format='%(message)s', force=True)

    con = duckdb.connect(db, read_only=True)
    con.execute("INSTALL spatial; LOAD spatial;")

    present = [r[0] for r in con.execute(
        "SELECT DISTINCT schema_name FROM duckdb_tables() "
        "WHERE table_name = 'edges' "
        "AND schema_name NOT IN ('information_schema', 'pg_catalog', 'main', 'raw') "
        "ORDER BY schema_name").fetchall()]
    chosen = list(modes) if modes else present
    if not chosen:
        raise click.ClickException(f"no mode schemas with an 'edges' table in {db}")
    unknown = [m for m in chosen if m not in present]
    if unknown:
        raise click.ClickException(
            f"mode(s) {unknown} not found in {db} (present: {present or 'none'})")

    name = Path(db).stem
    rendered = [render_network(con, m, name, basemap=basemap, out_dir=out_dir, arrows=arrows,
                               boundary=boundary)
                for m in chosen]
    rendered = [p for p in rendered if p]
    if not rendered:
        raise click.ClickException(
            "nothing rendered — install geopandas + roadstyle, or check the db has edges")
    for p in rendered:
        click.echo(f"wrote {p}")


@main.command()
@click.argument('db', type=click.Path(exists=True))
@click.option('--mode', '-m', default='driving', show_default=True,
              help='Mode schema to export')
@click.option('--out-dir', default='sumo', show_default=True,
              help='Output directory for <name>.{nod,edg,net}.xml')
@click.option('--name', default=None,
              help='Net basename (default: the db filename stem)')
@click.option('--connections/--no-connections', default=True, show_default=True,
              help='Emit turn-restriction connections from edge_graph (else let netconvert infer)')
@click.option('--config', '-c', type=click.Path(exists=True), default=None,
              help='netconvert config (.netccfg) to use instead of the built-in default')
@click.option('--netconvert/--no-netconvert', default=True, show_default=True,
              help='Assemble the .net.xml with netconvert (else write only plain-XML)')
def sumo(db, mode, out_dir, name, connections, config, netconvert):
    """Export a built network to a SUMO net, keeping duckOSM edge_id as the SUMO edge id.

    Writes <out-dir>/<name>.{nod,edg,con}.xml + a standard .netccfg (and .net.xml unless
    --no-netconvert). Connections come from edge_graph so turn restrictions are honoured.
    netconvert ships with SUMO (`pip install duckosm[sumo]` or a system install).
    """
    import duckdb

    from duckosm.sumo import to_sumo

    logging.basicConfig(level=logging.INFO, format='%(message)s', force=True)
    con = duckdb.connect(db, read_only=True)
    net_name = name or Path(db).stem
    try:
        out = to_sumo(con, out_dir, mode=mode, net_name=net_name, connections=connections,
                      config=config, run_netconvert=netconvert)
    except Exception as e:
        raise click.ClickException(str(e))
    click.echo(f"wrote {out['edg']} ({out['n_edges']} edges) and {out['nod']} "
               f"({out['n_nodes']} nodes)")
    if out.get("con"):
        click.echo(f"wrote {out['con']} ({out['n_connections']} connections)")
    if out.get("net"):
        click.echo(f"wrote {out['net']}")


@main.command()
@click.argument('db', type=click.Path(exists=True))
@click.option('--transfer-cost', type=float, default=60.0, show_default=True,
              help='Flat transfer penalty in seconds at each shared junction (v1 coarse)')
@click.option('--schema', default='mm', show_default=True,
              help='Target schema for the mm.edges / mm.transfers tables')
@click.option('--realistic', is_flag=True, default=False,
              help='v2 park-and-ride (restrict transfers to parking POIs) — NOT yet implemented')
def multimodal(db, transfer_cost, schema, realistic):
    """Build the intermodal transfer graph (mm.edges + mm.transfers) into a built db.

    Stitches the per-mode networks (driving/walking/cycling) into one layered graph so a trip can
    switch mode mid-route (walk->drive->walk / park-and-ride). Needs >=2 modes including walking.
    Route the result with `duckosm.route_multimodal(con, src_node, dst_node)`. See
    docs/multimodal.md. The db is modified in place (writes the `mm` schema).
    """
    import duckdb

    from duckosm.processors import MultimodalBuilder

    logging.basicConfig(level=logging.INFO, format='%(message)s', force=True)
    con = duckdb.connect(db)                     # read-write: this writes the mm.* tables
    try:
        stats = MultimodalBuilder(con, transfer_s=transfer_cost, realistic=realistic,
                                  schema=schema).run()
        con.execute("CHECKPOINT")
    except NotImplementedError as e:
        raise click.ClickException(str(e))
    except Exception as e:
        raise click.ClickException(str(e))
    finally:
        con.close()
    click.echo(f"wrote {schema}.edges ({stats.get('edge_count', 0)} edges) and "
               f"{schema}.transfers ({stats.get('transfer_count', 0)} arcs)")


@main.command(name="export-graph")
@click.argument('db', type=click.Path(exists=True))
@click.option('--mode', '-m', default='driving', show_default=True,
              help='Mode schema to export')
@click.option('--out', '-o', default=None,
              help='Output file (default: <name>_<mode>.<ext>). Extension picks the format.')
@click.option('--graph', '-g', 'graph_kind', type=click.Choice(['node', 'edge']), default='node',
              show_default=True,
              help="'node': geographic MultiDiGraph (junctions + full edge info); "
                   "'edge': edge-based routing DiGraph")
@click.option('--format', '-f', 'fmt', type=click.Choice(['graphml', 'gpickle']), default=None,
              help='Output format (default: inferred from --out extension, else graphml)')
@click.option('--weight', type=click.Choice(['time', 'length']), default='time', show_default=True,
              help="Arc weight for --graph edge (ignored for node graphs)")
@click.option('--geometry', type=click.Choice(['wkt', 'shapely', 'none']), default='wkt',
              show_default=True,
              help="Edge geometry for --graph node (shapely falls back to WKT for graphml)")
def export_graph(db, mode, out, graph_kind, fmt, weight, geometry):
    """Export a built network as a networkx graph file (GraphML or gpickle).

    By default writes the geographic node-based MultiDiGraph (OSM junctions as nodes, road
    segments keyed by edge_id carrying their full attribute set). Use --graph edge for the
    edge-based routing DiGraph. GraphML is portable (scalar-only — lists/geometry stringified);
    gpickle round-trips losslessly but is Python-only. Needs networkx (`pip install networkx`).
    """
    import duckdb

    from duckosm.routing import write_graph

    logging.basicConfig(level=logging.INFO, format='%(message)s', force=True)
    con = duckdb.connect(db, read_only=True)

    if out is None:
        ext = ".graphml" if (fmt or "graphml") == "graphml" else ".gpickle"
        out = f"{Path(db).stem}_{mode}{ext}"
    try:
        res = write_graph(con, out, mode=mode, graph=graph_kind, fmt=fmt, weight=weight,
                          geometry=geometry)
    except Exception as e:
        raise click.ClickException(str(e))
    click.echo(f"wrote {res['path']} ({res['fmt']}: {res['n_nodes']} nodes, "
               f"{res['n_edges']} edges)")


@main.command(name="export-gis")
@click.argument('db', type=click.Path(exists=True))
@click.option('--mode', '-m', 'modes', multiple=True,
              help='Mode schema(s) to export (default: every mode present in the db)')
@click.option('--format', '-f', 'fmt', type=click.Choice(['gpkg', 'shp']), default='gpkg',
              show_default=True, help='GeoPackage (multi-layer, primary) or ESRI shapefile')
@click.option('--out', '-o', default=None,
              help='gpkg: output .gpkg file; shp: output directory (default: <name>.gpkg / <name>_gis/)')
@click.option('--boundary/--no-boundary', default=True, show_default=True,
              help='Also export main.boundary as a boundary layer, if present')
@click.option('--name', default=None, help='Layer/file basename (default: the db filename stem)')
def export_gis(db, modes, fmt, out, boundary, name):
    """Export a built network to GeoPackage / shapefile, keeping duckOSM edge_id as an attribute.

    Writes the geographic layers only — edges_<mode>, nodes_<mode> and boundary — in EPSG:4326.
    GeoPackage is one multi-layer file (assembled with ogr2ogr; needs GDAL); shapefile is one
    fileset per layer. Routing adjacency (edge_graph/turn_restrictions) is not GIS data — use
    `export-graph` or `sumo` for that. Needs the DuckDB spatial extension.
    """
    import duckdb

    from duckosm.gis import to_gis

    logging.basicConfig(level=logging.INFO, format='%(message)s', force=True)
    con = duckdb.connect(db, read_only=True)
    base = name or Path(db).stem
    if out is None:
        out = f"{base}.gpkg" if fmt == "gpkg" else f"{base}_gis"
    try:
        res = to_gis(con, out, modes=list(modes) or None, fmt=fmt, boundary=boundary, name=base)
    except Exception as e:
        raise click.ClickException(str(e))
    dest = res.get("path") or res.get("dir")
    click.echo(f"wrote {dest} ({res['fmt']}: {len(res['layers'])} layers — "
               + ", ".join(f"{k} {v}" for k, v in res['layers'].items()) + ")")
    if res['fmt'] == 'gpkg' and res.get('path') is None:
        click.echo(f"  (ogr2ogr not found — wrote {len(res['files'])} single-layer files instead)")


@main.command(name="gis-debug")
@click.argument('export_path', type=click.Path(exists=True))
@click.option('--source-db', type=click.Path(exists=True), default=None,
              help='Built duckOSM db to round-trip against (checks every exported edge_id matches)')
@click.option('--out', '-o', default=None, help='Output HTML (default: <name>_gis_debug.html)')
@click.option('--name', default=None, help='Display name (default: the export filename stem)')
def gis_debug(export_path, source_db, out, name):
    """Debug an export: read the GeoPackage/shapefile back through GDAL and write a self-contained
    HTML page — a canvas map of every layer plus a QA audit (feature counts, CRS, edge_id integrity,
    and a round-trip diff vs --source-db). Verifies the file on disk, not the db. Needs geopandas.

    EXPORT_PATH is a .gpkg file or a directory of shapefiles.
    """
    logging.basicConfig(level=logging.INFO, format='%(message)s', force=True)

    from duckosm.gis_debug import write_debug

    if out is None:
        out = f"{Path(export_path).stem or 'export'}_gis_debug.html"
    try:
        path = write_debug(export_path, source_db=source_db, out=out, name=name)
    except Exception as e:
        raise click.ClickException(str(e))
    click.echo(f"wrote {path} — open in a browser")


@main.command(name="gmns")
@click.argument('db', type=click.Path(exists=True))
@click.option('--out', '-o', default=None,
              help='Output GMNS .duckdb file (default: <name>_gmns.duckdb)')
@click.option('--mode', '-m', 'modes', multiple=True,
              help='Mode schema(s) to extract (default: every mode present in the db)')
@click.option('--to-csv', default=None,
              help='Also dump spec-standard GMNS CSVs into this directory')
@click.option('--lane-geometry/--no-lane-geometry', default=True, show_default=True,
              help='Compute per-lane offset geometry for lane-level rendering (needs shapely)')
@click.option('--meso', is_flag=True, default=False,
              help='Also build a mesoscopic (lane-level) network — meso_<mode> schemas')
@click.option('--meso-mode', 'meso_modes', multiple=True,
              help='Modes to build meso for (default: driving)')
def gmns(db, out, modes, to_csv, lane_geometry, meso, meso_modes):
    """Extract a built network to a standalone GMNS DuckDB — every GMNS table OSM can support
    (config, node, link, geometry, lane, movement, use_definition/use_group, signal_controller,
    curb_seg), with native geometry and lane detail, keeping duckOSM edge_id as link_id.

    GMNS is the open network standard consumed by DTALite / Path4GMNS / the AMS ecosystem. Units:
    length metres, free_speed km/h, coordinates EPSG:4326. `--to-csv` also writes the spec CSVs. Lane
    / signal / curb detail needs the OSM tags (raw schema); needs the DuckDB spatial extension.
    """
    from duckosm.gmns import to_gmns, to_meso

    logging.basicConfig(level=logging.INFO, format='%(message)s', force=True)
    if out is None:
        out = f"{Path(db).stem}_gmns.duckdb"
    try:
        res = to_gmns(db, out, modes=list(modes) or None, to_csv=to_csv, lane_geometry=lane_geometry)
        meso_res = to_meso(out, modes=list(meso_modes) or ["driving"]) if meso else None
    except Exception as e:
        raise click.ClickException(str(e))
    click.echo(f"wrote {res['path']}")
    for mode, m in res['modes'].items():
        click.echo(f"  {mode}: {m['link']} links, {m['lane']} lanes, {m['movement']} movements, "
                   f"{m['signal_controller']} signals, {m['curb_seg']} curb segments")
    if meso_res:
        for mode, m in meso_res.items():
            click.echo(f"  meso[{mode}]: {m['meso_link']} meso links "
                       f"({m['normal']} section + {m['movement']} connector), {m['meso_node']} nodes")
    if res.get('csv'):
        click.echo(f"  + GMNS CSVs -> {res['csv']}")


if __name__ == '__main__':
    main()
