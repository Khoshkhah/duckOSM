"""
CLI for duckOSM — a subcommand-based command line interface.

  duckosm build    build a routing network from a PBF, or clip one from a parent db
  duckosm init-config  write the commented config template to a file, to edit and build from
  duckosm boundary     find an area's boundary by name (in a PBF, else Nominatim) -> GeoJSON
  duckosm clip-pbf     cut a PBF down to a boundary (osmium)
  duckosm extract  slice a sub-area out of an existing build into a new db
  duckosm admin    add OSM administrative boundaries to a built db
  duckosm viz      render a roadstyle HTML map of a built network
  duckosm route-map  an interactive route planner page (click start and end; routes in the browser)
  duckosm sumo     export a built network to a SUMO net (edge_id preserved)
  duckosm export-graph  export a built network as a networkx graph file (GraphML / gpickle)
  duckosm export-gis    export a built network to GeoPackage / shapefile (edge_id preserved)
  duckosm gis-debug     read a GIS export back through GDAL and write an HTML debug/QA page
  duckosm gmns          extract a built network to a standalone GMNS DuckDB (lanes, movements, …)
  duckosm gmns-viz      write an interactive HTML viewer for a GMNS DuckDB (lanes / meso, tooltips)
  duckosm gmns-map      write a lane-level HTML map of a GMNS DuckDB (lanestyle)
  duckosm matsim        export a MATSim network.xml from a built duckOSM db (nodes + links)
  duckosm matsim-lanes  export MATSim lanes.xml + signals from a GMNS db (turn lanes, signalised nodes)
  duckosm opendrive     export an ASAM OpenDRIVE .xodr from a built duckOSM db (roads + lanes)
  duckosm elevation     sample a DEM into a built db (nodes.ele + edges.z_from/z_to, in place)
  duckosm railml        export a railML 2.4 rail infrastructure file from a built duckOSM db's raw OSM
  duckosm lanelet2      export a Lanelet2 HD-map (.osm) from a GMNS db's per-lane geometry
  duckosm lane-graph    build a lane-level routing graph (lane->lane turns + lane-changes) in a GMNS db
  duckosm route-lanes   plan a lane-level route (lane sequence + geometry + maneuvers) over a GMNS db

The `extract`/`admin` subcommands wrap duckosm.extract / duckosm.admin and forward their
arguments verbatim, so `duckosm extract --help` shows the full underlying options.
"""

import logging
from pathlib import Path

import click

from duckosm.config import Config
from duckosm.importer import DuckOSM
from duckosm.utils import check_db_name


class _DbPath(click.Path):
    """An existing database file, not named like a duckOSM schema (see check_db_name)."""

    def convert(self, value, param, ctx):
        value = super().convert(value, param, ctx)
        try:
            check_db_name(value)
        except ValueError as e:
            self.fail(str(e), param, ctx)
        return value


DB_PATH = _DbPath(exists=True)


def setup_logging(log_file=None):
    """Log to the console, and also to ``log_file`` when one is given (nothing is written to disk
    otherwise, so running duckosm doesn't leave folders behind in the current directory)."""
    handlers = [logging.StreamHandler()]
    if log_file:
        Path(log_file).parent.mkdir(parents=True, exist_ok=True)
        handlers.append(logging.FileHandler(log_file))
    logging.basicConfig(
        level=logging.INFO,
        format='[%(asctime)s] [%(levelname)s] %(message)s',
        datefmt='%H:%M:%S',
        handlers=handlers,
        force=True,
    )


def _stem(path):
    """'maps/monaco-latest.osm.pbf' -> 'monaco-latest' (also strips .geojson / .duckdb)."""
    name = Path(path).name
    for suffix in (".pbf", ".osm", ".geojson", ".json", ".duckdb"):
        name = name.removesuffix(suffix)
    return name


@click.group(context_settings=dict(help_option_names=["-h", "--help"]))
@click.version_option(package_name="duckosm")
def main():
    """duckOSM — high-performance OSM-to-routing-network converter.

    Convert OSM PBF files to DuckDB routing databases. Run a subcommand below;
    use `duckosm <command> --help` for that command's options.
    """


def _override(cfg, ctx, **values):
    """With --config, a build flag typed on the command line overrides the file's value (a flag left
    at its default doesn't)."""
    from click.core import ParameterSource
    typed = {k: v for k, v in values.items() if ctx.get_parameter_source(k) == ParameterSource.COMMANDLINE}
    if "pbf" in typed:
        cfg.source.type, cfg.source.pbf_path, cfg.pbf_path = "pbf", typed["pbf"], typed["pbf"]
    if "source_db" in typed:
        cfg.source.type, cfg.source.source_db = "duckdb", typed["source_db"]
    if "output" in typed:
        cfg.output_path = typed["output"]
    if "boundary" in typed:
        cfg.boundary.path = cfg.boundary_path = typed["boundary"]
    if "h3_cell" in typed:
        cfg.boundary.h3_cell = cfg.h3_cell = typed["h3_cell"]
    if "modes" in typed:
        cfg.modes = list(typed["modes"])
    for flag, option in (("graph", "build_graph"), ("h3_index", "h3_indexing"),
                         ("h3_resolution", "h3_resolution"), ("features", "build_features")):
        if flag in typed:
            setattr(cfg.options, option, typed[flag])


@main.command()
@click.option('--config', '-c', type=click.Path(exists=True), help='Path to YAML configuration file')
@click.option('--pbf', '-p', type=click.Path(exists=True), help='Path to PBF file')
@click.option('--output', '-o', type=click.Path(),
              help='Output DuckDB file (default: <boundary name>.duckdb if a boundary is given, else '
                   '<pbf name>.duckdb, in the current folder)')
@click.option('--boundary', '-b', type=click.Path(exists=True), help='GeoJSON boundary file for filtering')
@click.option('--source-db', type=click.Path(exists=True), help='Parent duckOSM db to clip from (source.type=duckdb)')
@click.option('--h3-cell', help='Build the area of this H3 cell (its outline is the boundary)')
@click.option('--graph/--no-graph', default=True, help='Build edge graph table')
@click.option('--h3-index/--no-h3-index', default=True, help='Add H3 spatial indexing')
@click.option('--h3-resolution', type=int, default=8, help='H3 resolution (0-15)')
@click.option('--modes', '-m', multiple=True, help='Transportation modes (driving, walking, cycling); repeat for several')
@click.option('--features/--no-features', default=True, show_default=True,
              help='Build the base-map layers (features.*: water, land, buildings, POIs…)')
@click.option('--fixes', type=click.Path(exists=True, dir_okay=False),
              help='Rules file of fixes for OSM errors (config key: osm_overrides)')
@click.option('--log-file', type=click.Path(dir_okay=False), help='Also write the log to this file')
def build(config, pbf, output, boundary, source_db, h3_cell, graph, h3_index, h3_resolution, modes,
          features, fixes, log_file):
    """Build a routing network from a PBF, or clip one from a parent db.

    \b
    Examples:
        # From a YAML config (duckosm init-config writes a commented one)
        duckosm build --config my_area.yaml
        # From CLI arguments: writes ./input.duckdb
        duckosm build --pbf input.osm.pbf -m driving -m walking
    """
    # Default to config/default.yaml if it exists and no config/pbf/source given.
    default_config = Path("config/default.yaml")
    if not config and not pbf and not source_db and default_config.exists():
        config = str(default_config)

    try:
        if config:
            cfg = Config.from_yaml(config)
        elif pbf or source_db:
            # Name the output after the input rather than a fixed path, so a pip user running
            # `duckosm build --pbf maps/monaco-latest.osm.pbf` gets ./monaco-latest.duckdb.
            name = _stem(output) if output else _stem(boundary or pbf or source_db)   # the area, if given
            cfg = Config.from_args(
                pbf_path=pbf or "",
                output_path=output or f"{name}.duckdb",
                name=name,
                boundary_path=boundary,
                source_db=source_db,
                h3_cell=h3_cell,
                build_graph=graph,
                h3_indexing=h3_index,
                h3_resolution=h3_resolution,
                build_features=features,
                modes=list(modes) if modes else ["driving"],
                osm_overrides=fixes,
            )
        else:
            raise click.ClickException(
                "provide --config, or --pbf, or --source-db "
                "(or config/default.yaml must exist)")
    except click.ClickException:
        raise
    except Exception as e:
        raise click.ClickException(f"loading config: {e}")

    if config:
        _override(cfg, click.get_current_context(), pbf=pbf, output=output, boundary=boundary,
                  source_db=source_db, h3_cell=h3_cell, graph=graph, h3_index=h3_index,
                  h3_resolution=h3_resolution, modes=modes, features=features)
    if fixes:
        cfg.osm_overrides = fixes                  # --fixes also overrides a config file's setting
    setup_logging(log_file)
    try:
        output_path = DuckOSM(cfg).run()
        click.echo(f"\nOutput: {output_path}")
    except Exception as e:
        raise click.ClickException(str(e))


@main.command()
@click.argument("name", required=False)
@click.option("--pbf", type=click.Path(exists=True),
              help="Search the borders stored in this PBF first (offline; needs GDAL's ogr2ogr)")
@click.option("--osm-id", type=int, help="Pick one exact OSM boundary relation (for ambiguous names)")
@click.option("--offline", is_flag=True, help="Never ask OpenStreetMap's Nominatim search online")
@click.option("--out", "-o", type=click.Path(dir_okay=False), help="Output GeoJSON (default: <name>.geojson)")
def boundary(name, pbf, osm_id, offline, out):
    """Find the boundary of the area called NAME and write it as GeoJSON.

    Looks in --pbf's own borders first (offline), then asks Nominatim online.

    \b
    Examples:
        duckosm boundary Monaco --pbf monaco-latest.osm.pbf     # -> monaco.geojson
        duckosm boundary "Södermalm, Stockholm"                 # online
    """
    logging.basicConfig(level=logging.INFO, format="%(message)s", force=True)
    from duckosm.area import find_boundary, write_boundary

    if not name and osm_id is None:
        raise click.ClickException("give a NAME or --osm-id")
    try:
        geom, info = find_boundary(name, pbf, osm_id, offline)
    except LookupError as e:
        raise click.ClickException(str(e))
    out = out or f"{(name or info['name']).split(',')[0].strip().lower().replace(' ', '_')}.geojson"
    write_boundary(geom, info, out)
    def describe(i):
        level = f"admin level {i['admin_level']}, " if i["admin_level"] is not None else ""
        return f"{i['name']} ({level}OSM relation {i['osm_id']}, {i['area_km2']} km²)"
    click.echo(f"wrote {out}: {describe(info)}, from {info['source']}")
    if info.get("others"):
        click.echo("also matched (pick one with --osm-id):")
        for o in info["others"]:
            click.echo(f"  {describe(o)}")


@main.command(name="clip-pbf")
@click.argument("pbf", type=click.Path(exists=True))
@click.argument("boundary", type=click.Path(exists=True))
@click.option("--out", "-o", type=click.Path(dir_okay=False),
              help="Output PBF (default: <boundary name>.osm.pbf)")
@click.option("--strategy", type=click.Choice(["complete_ways", "smart", "simple"]),
              default="complete_ways", show_default=True,
              help="complete_ways keeps roads crossing the border whole; smart also completes "
                   "multipolygons (rivers, land cover) for a base-map build")
def clip_pbf_cmd(pbf, boundary, out, strategy):
    """Cut PBF down to the area in BOUNDARY (a GeoJSON polygon), with osmium.

    \b
    Example:
        duckosm clip-pbf sweden-latest.osm.pbf sodermalm.geojson   # -> sodermalm.osm.pbf
    """
    from duckosm.area import clip_pbf

    out = out or f"{_stem(boundary)}.osm.pbf"
    if Path(out).resolve() == Path(pbf).resolve():
        raise click.ClickException("the output would overwrite the input PBF; pass --out")
    try:
        clip_pbf(pbf, boundary, out, strategy)
    except RuntimeError as e:
        raise click.ClickException(str(e))
    mb = lambda p: Path(p).stat().st_size / 1e6
    click.echo(f"wrote {out} ({mb(out):.1f} MB, from {mb(pbf):.1f} MB)")


@main.command(name="init-config")
@click.argument("path", default="duckosm.yaml", type=click.Path(dir_okay=False))
@click.option("--force", is_flag=True, help="Overwrite PATH if it already exists")
def init_config(path, force):
    """Write the fully commented config template to PATH (default: duckosm.yaml).

    Edit it, then run `duckosm build --config PATH`.
    """
    from importlib.resources import files

    dst = Path(path)
    if dst.exists() and not force:
        raise click.ClickException(f"{dst} already exists (use --force to overwrite)")
    dst.write_text(files("duckosm").joinpath("templates/config.yaml").read_text(encoding="utf-8"),
                   encoding="utf-8")
    click.echo(f"Wrote {dst}. Edit it, then: duckosm build --config {dst}")


# extract/admin forward all arguments to an argparse-based script: don't let Click
# intercept unknown options or --help — the wrapped script parses and handles them.
_PASSTHROUGH = dict(context_settings=dict(ignore_unknown_options=True, help_option_names=[]),
                    add_help_option=False)


@main.command(**_PASSTHROUGH)
@click.argument('args', nargs=-1, type=click.UNPROCESSED)
def extract(args):
    """Slice a sub-area out of an existing build into a new, self-contained db.

    Wraps duckosm.extract — run `duckosm extract --help` for options
    (--source, --db, and one of --name / --osm-id / --boundary).
    """
    from duckosm.extract import main as extract_main
    raise SystemExit(extract_main(list(args)))


@main.command(**_PASSTHROUGH)
@click.argument('args', nargs=-1, type=click.UNPROCESSED)
def admin(args):
    """Add OSM administrative boundaries (admin_boundaries table) to a built db.

    Wraps duckosm.admin — run `duckosm admin --help` for options
    (--pbf, --db, optional --gpkg). Requires ogr2ogr (GDAL).
    """
    from duckosm.admin import main as admin_main
    raise SystemExit(admin_main(list(args)))


@main.command()
@click.argument('db', type=DB_PATH)
@click.option('--mode', '-m', 'modes', multiple=True,
              help='Mode(s) to draw a page of, repeat for several (default: every mode, plus a page '
                   'with all modes)')
@click.option('--basemap', default=None,
              help="First background: the database's own layers (default), or voyager, positron, "
                   "osm, satellite, …")
@click.option('--out-dir', default='reports', show_default=True,
              help='Output folder for <name>_map.html and <name>_<mode>_network.html')
@click.option('--arrows/--no-arrows', default=True, show_default=True,
              help='Direction arrows on one-way roads (shown when zoomed in)')
@click.option('--boundary/--no-boundary', default=True, show_default=True,
              help="Draw the area's boundary (main.boundary), if present")
def viz(db, modes, basemap, out_dir, arrows, boundary):
    """Draw the maps of a built network with mapstyle: one page with every mode
    (<name>_map.html) and one per mode (<name>_<mode>_network.html), over the database's own base
    map. With -m, only those modes' pages. Needs duckosm[viz].
    """
    from duckosm.viz import render_maps

    logging.basicConfig(level=logging.INFO, format='%(message)s', force=True)
    try:
        paths = render_maps(db, modes=list(modes) or None, out_dir=out_dir, basemap=basemap,
                            arrows=arrows, boundary=boundary, all_modes=not modes)
    except (ImportError, ValueError) as e:
        raise click.ClickException(str(e))
    for p in paths:
        click.echo(f"wrote {p}")


@main.command()
@click.argument('db', type=DB_PATH)
@click.option('--json', 'as_json', is_flag=True, help='Print one JSON object instead of a table')
def info(db, as_json):
    """What a built database holds: per mode its edges, nodes, private edges, km, legal turns and
    turn restrictions; when and by which duckOSM version it was built; its time zone; and the
    other schemas (raw OSM data, base-map layers, across modes, boundary, elevation). Read-only."""
    import json

    import duckdb

    from duckosm.query import db_info

    con = duckdb.connect(db, read_only=True)
    d = db_info(con)
    if as_json:
        click.echo(json.dumps(d, indent=1))
        return
    built = (f"built {d['built_at'][:16]}, duckOSM {d['duckosm_version']}" if d["built_at"]
             else "build date not recorded")
    click.echo(f"{Path(db).name}  ({built}; time zone {d['timezone'] or 'not recorded'})")
    if not d["modes"]:
        click.echo("no mode schemas (no <mode>.edges table)")
    else:
        cols = ["mode", "edges", "nodes", "private_edges", "km", "edge_graph", "turn_restrictions"]
        fmt = lambda v: "-" if v is None else (f"{v:,}" if isinstance(v, int) else str(v))
        rows = [cols] + [[fmt(m[c]) if c != "mode" else m[c] for c in cols] for m in d["modes"]]
        w = [max(len(r[i]) for r in rows) for i in range(len(cols))]
        for r in rows:
            click.echo("  ".join(v.ljust(w[i]) if i == 0 else v.rjust(w[i]) for i, v in enumerate(r)))
    also = [label for label, on in (
        ("raw (OSM data)", d["raw"]),
        (f"features ({len(d['features'])} layers)", d["features"]),
        ("mm (across modes)", d["multimodal"]), ("boundary", d["boundary"]),
        (f"admin_boundaries ({d['admin_boundaries']})", d["admin_boundaries"]),
        ("elevation", d["elevation"])) if on]
    click.echo("also: " + (", ".join(also) if also else "nothing else"))


@main.command()
@click.argument('db', type=DB_PATH)
@click.argument('osm_id', type=int)
@click.option('--mode', '-m', 'modes', multiple=True,
              help='Mode schema(s) to include (default: every mode present in the db)')
@click.option('--geom/--no-geom', default=False, show_default=True,
              help='Include the edge geometry as WKT (wide!)')
@click.option('--out', '-o', type=click.Path(), default=None,
              help='Write the table to a file instead of printing '
                   '(format by extension: .csv, .parquet, or .json)')
@click.option('--json', 'as_json', is_flag=True,
              help='Print one JSON object: the raw way (tags, refs) and its edges')
def way(db, osm_id, modes, geom, out, as_json):
    """Everything this db knows about one OSM_ID, across all modes, in one table.

    Prints the raw way row (tags + node refs), then the union of the per-mode edges
    extracted from it — one row per (mode, edge), columns NULL-filled where a mode
    doesn't carry them — in traversal order, so the cross-mode segmentation can be
    compared at a glance (same edge_id/edge_ref across modes = aligned). A negative
    OSM_ID inspects synthetic PathConnector connector edges.
    """
    import duckdb

    from duckosm.query import way_raw, way_table_sql

    con = duckdb.connect(db, read_only=True)
    con.execute("INSTALL spatial; LOAD spatial;")

    raw = way_raw(con, osm_id)
    if as_json:
        import json
        try:
            cur = con.execute(way_table_sql(con, osm_id, modes=list(modes) or None, geometry=geom))
        except ValueError as e:
            raise click.ClickException(str(e))
        cols = [c[0] for c in cur.description]
        click.echo(json.dumps({"osm_id": osm_id, "raw": raw,
                               "edges": [dict(zip(cols, r)) for r in cur.fetchall()]},
                              indent=1, default=str))
        return
    if raw:
        click.echo(f"raw way {raw['osm_id']}: {len(raw['refs'])} node refs")
        click.echo(f"  tags: {raw['tags']}")
        click.echo(f"  refs: {raw['refs']}")
    elif osm_id > 0:
        click.echo(f"raw way {osm_id}: not in raw.ways (clip build, or id not in this extract)")
    else:
        click.echo(f"osm_id {osm_id} is synthetic (PathConnector connector; -min of the "
                   "two joined ways)")

    try:
        sql = way_table_sql(con, osm_id, modes=list(modes) or None, geometry=geom)
    except ValueError as e:
        raise click.ClickException(str(e))
    n = con.execute(f"SELECT count(*) FROM ({sql})").fetchone()[0]
    if n == 0:
        raise click.ClickException(f"osm_id {osm_id} has no edges in any requested mode")
    if out:
        fmt = Path(out).suffix.lower().lstrip('.')
        copy_opts = {"csv": "(FORMAT CSV, HEADER)", "parquet": "(FORMAT PARQUET)",
                     "json": "(FORMAT JSON, ARRAY true)"}
        if fmt not in copy_opts:
            raise click.ClickException(f"unsupported extension .{fmt} — use .csv, .parquet or .json")
        con.execute(f"COPY ({sql}) TO '{out}' {copy_opts[fmt]}")
        click.echo(f"wrote {n} rows -> {out}")
    else:
        # very large max_width so NO column is ever hidden behind "…" — the table wraps in a
        # narrow terminal instead of silently dropping columns (use -o file.csv for a clean copy)
        con.sql(sql).show(max_rows=1000, max_width=1_000_000)


@main.command()
@click.argument('db', type=DB_PATH)
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
@click.argument('db', type=DB_PATH)
@click.option('--transfer-cost', type=float, default=60.0, show_default=True,
              help='Flat transfer penalty in seconds at each shared junction (v1 coarse)')
@click.option('--schema', default='mm', show_default=True,
              help='Target schema for the mm.edges / mm.transfers tables')
def multimodal(db, transfer_cost, schema):
    """Build the intermodal transfer graph (mm.edges + mm.transfers) into a built db.

    Stitches the per-mode networks (driving/walking/cycling) into one layered graph so a trip can
    switch mode mid-route (walk->drive->walk / park-and-ride). Needs >=2 modes including walking.
    Route the result with `duckosm.route_multimodal(con, src_node, dst_node)`. See
    https://khoshkhah.github.io/duckOSM/concepts/multimodal/. The db is modified in place (writes the `mm` schema).
    """
    import duckdb

    from duckosm.processors import MultimodalBuilder

    logging.basicConfig(level=logging.INFO, format='%(message)s', force=True)
    con = duckdb.connect(db)                     # read-write: this writes the mm.* tables
    try:
        stats = MultimodalBuilder(con, transfer_s=transfer_cost, schema=schema).run()
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
@click.argument('db', type=DB_PATH)
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
@click.argument('db', type=DB_PATH)
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
@click.option('--json', 'as_json', is_flag=True,
              help='Also print the check as JSON: verdict, summary, per-layer checks')
def gis_debug(export_path, source_db, out, name, as_json):
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
        path, payload = write_debug(export_path, source_db=source_db, out=out, name=name,
                                    return_payload=True)
    except Exception as e:
        raise click.ClickException(str(e))
    if as_json:
        import json
        click.echo(json.dumps({"out": str(path), "verdict": payload["verdict"],
                               "summary": payload["summary"], "layers": payload["layers"]},
                              indent=1, default=str))
    else:
        click.echo(f"wrote {path} — open in a browser")


@main.command(name="gmns")
@click.argument('db', type=DB_PATH)
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
@click.option('--combined', is_flag=True, default=False,
              help='Also write a single mode-tagged gmns_all network (links merged, allowed_uses unioned)')
@click.option('--drive-side', type=click.Choice(['right', 'left']), default='right', show_default=True,
              help='Traffic side: two-way lanes offset to this side so the directions separate')
@click.option('--pair-carriageways/--no-pair-carriageways', default=True, show_default=True,
              help='Place a one-way road mapped next to its opposite direction as one two-way road '
                   '(lanes from the line between them), so the two directions never overlap')
@click.option('--micro', is_flag=True, default=False,
              help='Also build a microscopic (cell-based) network — micro_<mode> schemas')
@click.option('--micro-mode', 'micro_modes', multiple=True, help='Modes to build micro for (default: driving)')
def gmns(db, out, modes, to_csv, lane_geometry, meso, meso_modes, combined, drive_side,
         pair_carriageways, micro, micro_modes):
    """Extract a built network to a standalone GMNS DuckDB — every GMNS table OSM can support
    (config, node, link, geometry, lane, movement, use_definition/use_group, signal_controller,
    curb_seg), with native geometry and lane detail, keeping duckOSM edge_id as link_id.

    GMNS is the open network standard consumed by DTALite / Path4GMNS / the AMS ecosystem. Units:
    length metres, free_speed km/h, coordinates EPSG:4326. `--to-csv` also writes the spec CSVs. Lane
    / signal / curb detail needs the OSM tags (raw schema); needs the DuckDB spatial extension.
    """
    from duckosm.gmns import to_gmns, to_meso, to_micro

    logging.basicConfig(level=logging.INFO, format='%(message)s', force=True)
    if out is None:
        out = f"{Path(db).stem}_gmns.duckdb"
    try:
        res = to_gmns(db, out, modes=list(modes) or None, to_csv=to_csv, lane_geometry=lane_geometry,
                      combined=combined, drive_side=drive_side, pair_carriageways=pair_carriageways)
        meso_res = to_meso(out, modes=list(meso_modes) or ["driving"]) if meso else None
        micro_res = to_micro(out, modes=list(micro_modes) or ["driving"]) if micro else None
    except Exception as e:
        raise click.ClickException(str(e))
    click.echo(f"wrote {res['path']}")
    for mode, m in res['modes'].items():
        click.echo(f"  {mode}: {m['link']} links, {m['lane']} lanes, {m['movement']} movements, "
                   f"{m['signal_controller']} signals, {m['curb_seg']} curb segments")
    if res.get('combined'):
        click.echo(f"  combined: gmns_all — {res['combined']['link']} links, "
                   f"{res['combined']['node']} nodes (mode-tagged)")
    if meso_res:
        for mode, m in meso_res.items():
            click.echo(f"  meso[{mode}]: {m['meso_link']} meso links "
                       f"({m['normal']} section + {m['movement']} connector), {m['meso_node']} nodes")
    if micro_res:
        for mode, m in micro_res.items():
            click.echo(f"  micro[{mode}]: {m['micro_link']} micro links ({m['cell']} cell + "
                       f"{m['lane_change']} lane-change + {m['movement']} turn), {m['micro_node']} nodes")
    if res.get('csv'):
        click.echo(f"  + GMNS CSVs -> {res['csv']}")


@main.command(name="gmns-viz")
@click.argument('gmns_db', type=click.Path(exists=True))
@click.option('--mode', '-m', default='driving', show_default=True, help='Mode schema to view')
@click.option('--out', '-o', default=None, help='Output HTML (default: <name>_viewer.html)')
def gmns_viz(gmns_db, mode, out):
    """Write a self-contained interactive HTML viewer for a GMNS DuckDB.

    Toggle between the individual *lanes* (offset by use) and the *mesoscopic* network (section +
    turn-connector links); hover any line for its type, id and attributes; scroll to zoom, drag to
    pan. The meso layer appears only if the db has a meso_<mode> schema (`duckosm gmns --meso`).
    Reads only DuckDB (geometry is drawn client-side); open the HTML in any browser.
    """
    logging.basicConfig(level=logging.INFO, format='%(message)s', force=True)

    from duckosm.gmns_viewer import write_viewer

    if out is None:
        out = f"{Path(gmns_db).stem}_viewer.html"
    try:
        path = write_viewer(gmns_db, out, mode=mode)
    except Exception as e:
        raise click.ClickException(str(e))
    click.echo(f"wrote {path} — open in a browser")


@main.command(name="matsim")
@click.argument('db', type=DB_PATH)
@click.option('--mode', '-m', default='driving', show_default=True,
              help="Mode(s): a single mode, 'all', or a comma-list (e.g. driving,cycling) → one "
                   "multimodal network with modes= per link")
@click.option('--crs', default=None,
              help='Projected metric CRS for node coords (default: the UTM zone of the data; '
                   'e.g. EPSG:3006 for SWEREF99 TM)')
@click.option('--gzip/--no-gzip', 'gzip', default=True, show_default=True, help='Gzip the output (MATSim convention)')
@click.option('--out', '-o', default=None, help='Output path (default: <name>_network.xml[.gz])')
def matsim(db, mode, crs, gzip, out):
    """Export a MATSim network.xml from a built duckOSM db — the directed node+link substrate for
    MATSim / BEAM / eqasim. Each edge becomes one directed link (edge_id preserved), with length,
    freespeed, capacity, permlanes and modes; node coordinates reprojected to a metric CRS.
    See https://khoshkhah.github.io/duckOSM/exports/matsim/.
    """
    logging.basicConfig(level=logging.INFO, format='%(message)s', force=True)

    from duckosm.matsim import to_matsim

    if out is None:
        out = f"{Path(db).stem}_network.xml" + (".gz" if gzip else "")
    try:
        res = to_matsim(db, out, mode=mode, crs=crs, gzip=gzip)
    except Exception as e:
        raise click.ClickException(str(e))
    click.echo(f"wrote {out} — {res['nodes']} nodes, {res['links']} links (CRS {res['crs']})")


@main.command(name="lane-graph")
@click.argument('gmns_db', type=click.Path(exists=True))
@click.option('--mode', '-m', default='driving', show_default=True, help='GMNS mode schema')
def lane_graph(gmns_db, mode):
    """Build a lane-level routing graph in a GMNS db — lane->lane turn edges (from movements) +
    lane-change edges (adjacent lanes), written as lane_<mode>.lane_edges. See https://khoshkhah.github.io/duckOSM/exports/lane-routing/.
    """
    logging.basicConfig(level=logging.INFO, format='%(message)s', force=True)

    from duckosm.lane_routing import build_lane_graph

    try:
        res = build_lane_graph(gmns_db, mode=mode)
    except Exception as e:
        raise click.ClickException(str(e))
    click.echo(f"built lane graph — {res['lanes']} lanes, {res['edges']} edges "
               f"({res['turn']} turn + {res['lane_change']} lane-change)")


@main.command(name="route-lanes")
@click.argument('gmns_db', type=click.Path(exists=True))
@click.argument('from_lane')
@click.argument('to_lane')
@click.option('--mode', '-m', default='driving', show_default=True, help='GMNS mode schema')
@click.option('--out', '-o', default=None, help='Write the route as GeoJSON to this path')
@click.option('--json', 'as_json', is_flag=True,
              help='Print the route as JSON: lanes, cost, maneuvers, geometry (WKT)')
def route_lanes_cmd(gmns_db, from_lane, to_lane, mode, out, as_json):
    """Plan a lane-level route between two lanes (each a lane_id or an edge_id → its lane 1). Prints the
    lane count / cost / maneuvers; with -o writes the route geometry as GeoJSON. See https://khoshkhah.github.io/duckOSM/exports/lane-routing/.
    """
    logging.basicConfig(level=logging.INFO, format='%(message)s', force=True)

    from duckosm.lane_routing import route_lanes

    try:
        res = route_lanes(gmns_db, from_lane, to_lane, mode=mode)
    except Exception as e:
        raise click.ClickException(str(e))
    if as_json:
        import json
        click.echo(json.dumps({"from": from_lane, "to": to_lane, **res}, indent=1))
    elif not res["lanes"]:
        click.echo(f"no lane route from {from_lane} to {to_lane}")
        return
    else:
        click.echo(f"route: {len(res['lanes'])} lanes, cost {res['cost']:.0f}, "
                   f"maneuvers: {', '.join(res['maneuvers']) or '(none)'}")
    if out and res["geometry"]:
        import json

        # minimal inline WKT LINESTRING -> GeoJSON (no extra deps)
        body = res["geometry"][res["geometry"].index("(") + 1:res["geometry"].rindex(")")]
        coords = [[float(v) for v in p.split()[:2]] for p in body.split(",")]
        fc = {"type": "FeatureCollection", "features": [{"type": "Feature",
              "properties": {"lanes": len(res["lanes"]), "cost": res["cost"],
                             "maneuvers": res["maneuvers"]},
              "geometry": {"type": "LineString", "coordinates": coords}}]}
        Path(out).write_text(json.dumps(fc))
        if not as_json:
            click.echo(f"wrote {out}")


@main.command(name="lanelet2")
@click.argument('gmns_db', type=click.Path(exists=True))
@click.option('--mode', '-m', default='driving', show_default=True, help='GMNS mode schema')
@click.option('--out', '-o', default=None, help='Output .osm (default: <name>.lanelet2.osm)')
def lanelet2(gmns_db, mode, out):
    """Export a Lanelet2 HD-map (.osm) from a GMNS db — each per-lane geometry becomes a lanelet
    (left/right boundaries from centerline ± half-width) with subtype/one_way/speed_limit tags, for
    Autoware / the lanelet2 library. A lane-level map skeleton in the AD standard (OSM XML, renders
    natively) — not survey-grade. See https://khoshkhah.github.io/duckOSM/exports/lanelet2/.
    """
    logging.basicConfig(level=logging.INFO, format='%(message)s', force=True)

    from duckosm.lanelet2 import to_lanelet2

    if out is None:
        out = f"{Path(gmns_db).stem}.lanelet2.osm"
    try:
        res = to_lanelet2(gmns_db, out, mode=mode)
    except Exception as e:
        raise click.ClickException(str(e))
    click.echo(f"wrote {out} — {res['lanelets']} lanelets, {res['ways']} boundary ways, {res['nodes']} nodes")


@main.command(name="railml")
@click.argument('db', type=DB_PATH)
@click.option('--out', '-o', default=None, help='Output file (default: <name>.railml.xml)')
def railml(db, out):
    """Export a railML 2.4 rail infrastructure file from a built duckOSM db — a NEW rail extraction
    (duckOSM has no rail mode): railway ways from the raw OSM are split into tracks with topology,
    switches, signals and OCPs/stations, for OpenTrack / RailSys / Viriato. See https://khoshkhah.github.io/duckOSM/exports/railml/.
    """
    logging.basicConfig(level=logging.INFO, format='%(message)s', force=True)

    from duckosm.railml import to_railml

    if out is None:
        out = f"{Path(db).stem}.railml.xml"
    try:
        res = to_railml(db, out)
    except Exception as e:
        raise click.ClickException(str(e))
    click.echo(f"wrote {out} — {res['tracks']} tracks, {res['switches']} switches, "
               f"{res['signals']} signals, {res['ocps']} OCPs")


@main.command(name="opendrive")
@click.argument('db', type=DB_PATH)
@click.option('--mode', '-m', default='driving', show_default=True, help='Mode schema to export')
@click.option('--crs', default=None,
              help='Projected metric CRS for the reference line (default: the UTM zone of the data)')
@click.option('--junctions', is_flag=True, default=False,
              help='Phase 2: routable junctions + connecting roads (requires a GMNS db, not the core db)')
@click.option('--out', '-o', default=None, help='Output .xodr (default: <name>.xodr)')
def opendrive(db, mode, crs, junctions, out):
    """Export an ASAM OpenDRIVE .xodr from a built duckOSM db — roads with a reprojected reference line
    and lane-level width offsets, for AV sims (CARLA/esmini) and commercial micro (Vissim/Aimsun).
    Phase 1 (default): geometry + lanes. `--junctions` adds routable junctions + turn connecting roads
    (needs a GMNS db). See https://khoshkhah.github.io/duckOSM/exports/opendrive/.
    """
    logging.basicConfig(level=logging.INFO, format='%(message)s', force=True)

    from duckosm.opendrive import to_opendrive

    if out is None:
        out = f"{Path(db).stem}.xodr"
    try:
        res = to_opendrive(db, out, mode=mode, crs=crs, junctions=junctions)
    except Exception as e:
        raise click.ClickException(str(e))
    extra = f" + {res['connecting_roads']} connecting roads, {res['junctions']} junctions" if junctions else ""
    click.echo(f"wrote {out} — {res['roads']} roads{extra} (CRS {res['crs']})")


@main.command(name="route-map")
@click.argument('db', type=DB_PATH)
@click.option('--mode', '-m', default=None,
              type=click.Choice(['driving', 'walking', 'cycling', 'walk+drive']),
              help='The first choice and the network in front (walk+drive: every mode; needs '
                   '`duckosm multimodal`); the page offers every mode')
@click.option('--basemap', default=None,
              help="First background: the database's own layers (default), or voyager, positron, osm, …")
@click.option('--out', '-o', default=None, help='Output HTML (default: reports/<name>_route_map.html)')
def route_map_cmd(db, mode, basemap, out):
    """Write a route planner page for a built db, drawn by mapstyle: drag a start and an end, pick
    Drive, Walk, Cycle or Walk + drive (needs `duckosm multimodal` first), get the route and
    turn-by-turn directions. It routes in the browser with the same answers as route_points() /
    route_multimodal_points() and directions(). Needs duckosm[viz].
    """
    logging.basicConfig(level=logging.INFO, format='%(message)s', force=True)
    from duckosm.route_map import write_route_map

    name = Path(db).stem
    try:
        path = write_route_map(db, out or f"reports/{name}_route_map.html", mode=mode, basemap=basemap)
    except (ImportError, ValueError) as e:
        raise click.ClickException(str(e))
    click.echo(f"wrote {path} — open it in a browser")


@main.command(name="elevation")
@click.argument('db', type=DB_PATH)
@click.option('--dem', default=None,
              help='Local/remote raster (GeoTIFF/COG/VRT/.img/.asc/.hgt — any GDAL format). '
                   'Takes precedence over --source.')
@click.option('--source', default='auto', show_default=True,
              help="Global DEM to auto-fetch if no --dem: 'auto' (best for the db's area), "
                   "'copernicus' (GLO-30, anonymous), or 'eudtm' (Europe bare-earth, needs "
                   "OPENTOPOGRAPHY_API_KEY)")
@click.option('--mode', '-m', 'modes', multiple=True,
              help='Mode schema(s) to enrich (default: every mode present in the db)')
@click.option('--nodata-fill', type=float, default=0.0, show_default=True,
              help='Elevation written where the DEM has a void / does not cover the node')
@click.option('--suffix', default='', metavar='NAME',
              help="Store as a SECOND surface instead of overwriting: --suffix dsm writes "
                   "ele_dsm / z_from_dsm / z_to_dsm. Object height = ele_dsm - ele.")
def elevation(db, dem, source, modes, nodata_fill, suffix):
    """Sample a DEM at every node and add elevation to a built db, in place.

    Adds <mode>.nodes.ele and <mode>.edges.z_from/z_to, and records provenance in
    main.elevation_metadata. Either point --dem at any GDAL-readable raster, or let
    --source auto-fetch a global DEM. `--source auto` (default) reads the db's node
    bbox and picks the best provider whose coverage contains it: EU-DTM (bare-earth)
    inside Europe when OPENTOPOGRAPHY_API_KEY is set, else Copernicus GLO-30 (streamed
    from AWS, no auth, works anywhere). Re-runnable — a second pass overwrites. The db
    is modified in place. Needs `pip install duckosm[elevation]` (rasterio + pyproj).

    --suffix keeps a second surface side by side, so a bare-earth DTM and a DSM can both
    live in one db and `ele_dsm - ele` gives height above ground.
    See https://khoshkhah.github.io/duckOSM/guides/elevation/.
    """
    logging.basicConfig(level=logging.INFO, format='%(message)s', force=True)

    from duckosm.elevation import to_elevation

    try:
        res = to_elevation(db, dem=dem, source=source, modes=list(modes) or None,
                           nodata_fill=nodata_fill, suffix=suffix)
    except Exception as e:
        raise click.ClickException(str(e))
    click.echo(f"elevation added from {res['source']} — {res['n_nodes']} nodes "
               f"({res['n_nodata']} nodata/fill) across {len(res['modes'])} mode(s) "
               f"-> {res['column']}")


@main.command(name="matsim-lanes")
@click.argument('gmns_db', type=click.Path(exists=True))
@click.option('--mode', '-m', default='driving', show_default=True, help='GMNS mode schema')
@click.option('--signals/--no-signals', default=True, show_default=True,
              help='Also write signalSystems/signalGroups/signalControl.xml for signalised nodes')
@click.option('--cycle', default=90, show_default=True, help='Default signal cycle time (seconds)')
@click.option('--out', '-o', 'out_dir', default='.', show_default=True, help='Output directory')
def matsim_lanes(gmns_db, mode, signals, cycle, out_dir):
    """Export MATSim lanes.xml (+ signalSystems/Groups/Control.xml) from a GMNS db. Turn lanes come
    from the movement table; signals from signalised nodes with a default fixed-time plan (the timing
    is a synthetic placeholder — OSM has no signal plans). Pair with `duckosm matsim` (same edge_id
    link ids). See https://khoshkhah.github.io/duckOSM/exports/matsim/#lanes-and-signals.
    """
    logging.basicConfig(level=logging.INFO, format='%(message)s', force=True)

    from duckosm.matsim_lanes import to_matsim_lanes

    try:
        res = to_matsim_lanes(gmns_db, out_dir=out_dir, mode=mode, signals=signals, cycle_s=cycle)
    except Exception as e:
        raise click.ClickException(str(e))
    msg = f"wrote lanes.xml ({res['lanes_links']} assignments)"
    if signals:
        msg += f" + signals ({res['signal_systems']} systems)"
    click.echo(f"{msg} -> {out_dir}")


@main.command(name="gmns-map")
@click.argument('gmns_db', type=click.Path(exists=True))
@click.option('--mode', '-m', default='driving', show_default=True, help='Mode schema to map')
@click.option('--palette', default='mono', show_default=True, help="roadstyle palette: 'mono', 'carto' or 'highsat'")
@click.option('--source-db', type=click.Path(exists=True), default=None,
              help='Core db for bridge / tunnel / layer, for a GMNS file written before links carried them')
@click.option('--out', '-o', default=None, help='Output HTML (default: <name>_lanes.html)')
def gmns_map(gmns_db, mode, palette, source_db, out):
    """Write a lane-level HTML map of a GMNS DuckDB with lanestyle: every lane at its real width over
    a base map, bridges over tunnels; click a lane to see the lanes it can turn into.
    Needs `pip install "duckosm[viz]"`.
    """
    from duckosm.gmns_map import write_map

    if out is None:
        out = f"{Path(gmns_db).stem}_lanes.html"
    try:
        path = write_map(gmns_db, out, mode=mode, palette=palette, source_db=source_db)
    except Exception as e:
        raise click.ClickException(str(e))
    click.echo(f"wrote {path} — open in a browser")


if __name__ == '__main__':
    main()
