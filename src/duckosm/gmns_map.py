"""
Lane-level HTML map of a GMNS DuckDB, drawn by [lanestyle](https://github.com/Khoshkhah/lanestyle) (0.3 or later): the roads
of the level area as roadstyle draws them, each as wide as its lanes, with the lanes, lane lines, arrows, zebras and
sidewalks on them. The inspection viewer is [gmns_viewer](gmns_viewer.py).

    from duckosm.gmns_map import write_map
    write_map("monaco_gmns.duckdb", "monaco_lanes.html", source_db="monaco.duckdb")   # needs `duckosm levels monaco.duckdb`
"""
from pathlib import Path


def write_map(gmns_db, out, source_db, area=None):
    """Write the lane page of ``gmns_db`` to ``out``: the lanes on the roads of the level ``area`` (default: ``<source_db>.levels``,
    made by ``duckosm levels``) of ``source_db``, the core db the GMNS file was made from."""
    try:
        import lanestyle as ls
    except ImportError:
        raise ImportError('gmns-map needs lanestyle: pip install "duckosm[viz]"') from None
    area = Path(area) if area else Path(source_db).with_suffix(".levels")
    if not area.is_dir():
        raise FileNotFoundError(f"no level area {area}: make it with `duckosm levels {source_db}`")
    ls.lane_page(str(area), str(gmns_db), str(source_db)).save(str(out))
    return Path(out)
