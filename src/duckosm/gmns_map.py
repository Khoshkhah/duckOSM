"""
Lane-level HTML map of a GMNS DuckDB, drawn by [lanestyle](https://github.com/Khoshkhah/lanestyle):
every lane at its real width over a base map, with bridges and tunnels in their order; click a lane
to see the lanes its movements lead into. The inspection viewer is [gmns_viewer](gmns_viewer.py).

    from duckosm.gmns_map import write_map
    write_map("monaco_gmns.duckdb", "monaco_lanes.html")
"""
from pathlib import Path


def write_map(gmns_db, out, mode="driving", palette="mono", source_db=None):
    """Write the lane map of ``mode`` to ``out``. ``source_db`` (the core db) gives the bridge /
    tunnel / layer of a GMNS file written before the link table carried them."""
    try:
        import lanestyle as ls
    except ImportError:
        raise ImportError('gmns-map needs lanestyle: pip install "duckosm[viz]"') from None
    lanes, turns = ls.from_gmns(str(gmns_db), mode=mode, source_db=source_db and str(source_db))
    ls.render_lanes(lanes, turns=turns, palette=palette).save(str(out))
    return Path(out)
