"""Add an ``admin_boundaries`` table (all OSM administrative levels) to a duckOSM DB: the Rust ``duckosm admin``.

    duckosm admin --pbf sweden-latest.osm.pbf --db data/db/sweden.duckdb [--gpkg sweden_admin.gpkg]
"""
import sys


def main(argv=None) -> int:
    from duckosm import _rust
    return _rust.cli(["duckosm", "admin", *(sys.argv[1:] if argv is None else argv)])


if __name__ == "__main__":
    raise SystemExit(main())
