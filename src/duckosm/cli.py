"""
CLI for duckOSM.
"""

import logging
import os
import sys
from datetime import datetime
from pathlib import Path

import click

from duckosm.config import Config
from duckosm.importer import DuckOSM


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


@click.command()
@click.option(
    '--config', '-c',
    type=click.Path(exists=True),
    help='Path to YAML configuration file'
)
@click.option(
    '--pbf', '-p',
    type=click.Path(exists=True),
    help='Path to PBF file'
)
@click.option(
    '--output', '-o',
    type=click.Path(),
    default='data/output/network.duckdb',
    help='Output DuckDB file path'
)
@click.option(
    '--boundary', '-b',
    type=click.Path(exists=True),
    help='GeoJSON boundary file for filtering'
)
@click.option(
    '--source-db',
    type=click.Path(exists=True),
    help='Parent duckOSM db to clip from (source.type=duckdb)'
)
@click.option(
    '--h3-cell',
    help='H3 cell ID for filtering'
)
@click.option(
    '--graph/--no-graph',
    default=True,
    help='Build edge graph table'
)
@click.option(
    '--h3-index/--no-h3-index',
    default=True,
    help='Add H3 spatial indexing'
)
@click.option(
    '--h3-resolution',
    type=int,
    default=8,
    help='H3 resolution (0-15)'
)
@click.option(
    '--modes', '-m',
    multiple=True,
    help='Transportation modes (driving, walking, cycling)'
)
def main(
    config,
    pbf,
    output,
    boundary,
    source_db,
    h3_cell,
    graph,
    h3_index,
    h3_resolution,
    modes
):
    """
    duckOSM - High-performance OSM-to-routing-network converter.
    
    Convert OSM PBF files to DuckDB routing databases.
    
    Examples:
    
        # Using config file
        duckosm --config config/default.yaml
        
        # Using CLI arguments
        duckosm --pbf input.pbf --output network.duckdb --graph
    """
    # Default to config/default.yaml if it exists and no config/pbf/source given
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
                modes=list(modes) if modes else ["driving"]
            )
        else:
            click.echo("Error: provide --config, or --pbf, or --source-db "
                       "(or config/default.yaml must exist)", err=True)
            sys.exit(1)
    except Exception as e:
        click.echo(f"Error loading config: {e}", err=True)
        sys.exit(1)

    log_file = setup_logging(cfg.name)

    # Run import
    try:
        importer = DuckOSM(cfg)
        output_path = importer.run()
        click.echo(f"\nOutput: {output_path}")
    except Exception as e:
        click.echo(f"Error: {e}", err=True)
        sys.exit(1)


if __name__ == '__main__':
    main()
