"""GMNS (General Modeling Network Specification) from a built duckOSM db: a thin layer on the Rust build of ``duckosm gmns``
(``rust/duckosm-export/src/gmns.rs``). docs/exports/gmns.md, docs/exports/gmns_tables.md.

    from duckosm.gmns import to_gmns
    to_gmns("data/db/sodermalm.duckdb", "sodermalm_gmns.duckdb")          # standalone GMNS db
    to_gmns("data/db/sodermalm.duckdb", "out.duckdb", to_csv="gmns/")     # + spec CSVs
"""
from duckosm import _api


def to_gmns(source_db, out_path, modes=None, to_csv=None, lane_geometry=True, combined=False, drive_side="right",
            pair_carriageways=True, csv_extensions=False, gtfs=None, gtfs_max_m=30.0, walk_frame=False, walk_clearance_m=0.0,
            tile_zoom=9, lane_source="auto"):
    """The GMNS db ``out_path`` from the duckOSM db ``source_db`` (overwritten if it exists), as ``duckosm gmns``.

    ``modes``: the mode schemas (default: every mode present). ``to_csv``: also the spec CSVs into this folder.
    ``lane_geometry``: a line for each lane. ``combined``: also the mode-tagged ``gmns_all`` network. ``drive_side``:
    ``right`` or ``left``. ``pair_carriageways``: a one-way road next to its opposite direction is one side of a two-way
    road. ``csv_extensions``: keep duckOSM's extra columns in the CSVs (``u_`` prefix). ``gtfs``: GTFS feeds whose stops
    become ``location`` rows, within ``gtfs_max_m`` metres of a link. ``walk_frame``, ``walk_clearance_m``: a mapped
    sidewalk is placed at its road's kerb. ``tile_zoom``: build by map tiles of this zoom (None: the whole area at once).
    ``lane_source``: which lane goes to which at a junction: ``netconvert``, ``rules`` (no SUMO needed) or ``auto``
    (netconvert when it is installed, else the rules); docs/concepts/lane-rules.md.

    Returns ``{"path", "csv", "combined", "modes": {mode: {counts per table}}}``. Every ``link_id`` == ``edge_id``."""
    return _api.to_gmns(source_db, out_path, modes=modes, to_csv=to_csv, lane_geometry=lane_geometry, combined=combined,
                        drive_side=drive_side, pair_carriageways=pair_carriageways, csv_extensions=csv_extensions, gtfs=gtfs,
                        gtfs_max_m=gtfs_max_m, walk_frame=walk_frame, walk_clearance_m=walk_clearance_m, tile_zoom=tile_zoom,
                        lane_source=lane_source)


def to_meso(gmns_db, modes=None, trim_m=6.0):
    """A mesoscopic network (``meso_<mode>``: road sections and junction connectors) in a GMNS db, the sections inset
    ``trim_m`` metres from each end. ``modes`` default: driving. Returns ``{mode: counts}``."""
    return _api.to_meso(gmns_db, modes=modes, trim_m=trim_m)


def to_micro(gmns_db, modes=None, cell_length_m=7.0):
    """A microscopic network (``micro_<mode>``: lane cells of ``cell_length_m``, lane changes, turns) in a GMNS db. ``modes``
    default: driving. Returns ``{mode: counts}``."""
    return _api.to_micro(gmns_db, modes=modes, cell_length_m=cell_length_m)
