"""
CLI for duckOSM — a subcommand-based command line interface.

  duckosm build    build a routing network from a PBF, or clip one from a parent db
  duckosm extract  slice a sub-area out of an existing build into a new db
  duckosm admin    add OSM administrative boundaries to a built db
  duckosm viz      render a roadstyle HTML map of a built network

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
def viz(db, modes, basemap, out_dir):
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
    rendered = [render_network(con, m, name, basemap=basemap, out_dir=out_dir) for m in chosen]
    rendered = [p for p in rendered if p]
    if not rendered:
        raise click.ClickException(
            "nothing rendered — install geopandas + roadstyle, or check the db has edges")
    for p in rendered:
        click.echo(f"wrote {p}")


if __name__ == '__main__':
    main()
