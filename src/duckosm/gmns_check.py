"""Value-level checks on a GMNS database (docs/design/gmns_spec_conformance.md).

The spec's own validators check the *shape* of the tables (columns, types, keys, categories); this
checks what the values say about each other: a link starts and ends on its nodes, its length is its
geometry's, lane numbers run 1..n and ``link.lanes`` is no more than the lane rows, a movement turns at the node where its
inbound link ends and uses lanes the links have. Plain SQL, per ``gmns_<mode>`` schema.

``check_gmns(path)`` returns one row per check: ``(schema, check, bad, total)``; ``bad == 0`` passes.
Not checked, because they are expected in OSM data: links with no way on or in (dead ends, the area's
edge), parallel duplicate links, very short links.
"""
import duckdb

_TOL_DEG = 1e-6          # ~0.1 m: a link's end points are its nodes' coordinates, copied
_LEN_TOL = 0.01          # link.length vs its geometry (sphere vs ellipsoid explains < 0.5 %)


def _checks(s):
    """(name, bad-count SQL, total-count SQL) for schema ``s``."""
    L, N, K, M = f"{s}.link", f"{s}.node", f"{s}.lane", f"{s}.movement"
    cnt = f"(SELECT link_id, count(*) n, min(lane_num) lo, max(lane_num) hi FROM {K} GROUP BY link_id)"
    return [
        ("link starts at its from_node", f"""SELECT count(*) FROM {L} l JOIN {N} n ON n.node_id = l.from_node_id
            WHERE abs(ST_X(ST_StartPoint(l.geom)) - n.x_coord) > {_TOL_DEG}
               OR abs(ST_Y(ST_StartPoint(l.geom)) - n.y_coord) > {_TOL_DEG}""", f"SELECT count(*) FROM {L}"),
        ("link ends at its to_node", f"""SELECT count(*) FROM {L} l JOIN {N} n ON n.node_id = l.to_node_id
            WHERE abs(ST_X(ST_EndPoint(l.geom)) - n.x_coord) > {_TOL_DEG}
               OR abs(ST_Y(ST_EndPoint(l.geom)) - n.y_coord) > {_TOL_DEG}""", f"SELECT count(*) FROM {L}"),
        ("link from_node / to_node exist", f"""SELECT count(*) FROM {L} WHERE from_node_id NOT IN
            (SELECT node_id FROM {N}) OR to_node_id NOT IN (SELECT node_id FROM {N})""", f"SELECT count(*) FROM {L}"),
        ("link is not a loop", f"SELECT count(*) FROM {L} WHERE from_node_id = to_node_id", f"SELECT count(*) FROM {L}"),
        ("link.length is its geometry's length", f"""SELECT count(*) FROM {L} WHERE abs(length /
            NULLIF(ST_Length_Spheroid(ST_FlipCoordinates(geom)), 0) - 1) > {_LEN_TOL}""", f"SELECT count(*) FROM {L}"),
        ("node has a link", f"""SELECT count(*) FROM {N} WHERE node_id NOT IN
            (SELECT from_node_id FROM {L} UNION SELECT to_node_id FROM {L})""", f"SELECT count(*) FROM {N}"),
        ("link has lane rows", f"SELECT count(*) FROM {L} WHERE link_id NOT IN (SELECT link_id FROM {K})",
         f"SELECT count(*) FROM {L}"),
        ("lane_num is 1..n", f"SELECT count(*) FROM {cnt} WHERE lo <> 1 OR hi <> n", f"SELECT count(*) FROM {cnt}"),
        # `lanes` counts motor lanes only (no turn pockets, bike lanes, shoulders), so it can be less than the
        # lane rows, never more
        ("link.lanes is not more than the lane rows", f"""SELECT count(*) FROM {L} l JOIN {cnt} c USING (link_id)
            WHERE l.lanes > c.n""", f"SELECT count(*) FROM {L} WHERE lanes IS NOT NULL"),
        ("location lies on its link", f"""SELECT count(*) FROM {s}.location x LEFT JOIN {L} l USING (link_id)
            WHERE l.link_id IS NULL OR x.lr < 0 OR x.lr > l.length + 0.01""", f"SELECT count(*) FROM {s}.location"),
        ("movement turns where its inbound link ends", f"""SELECT count(*) FROM {M} m JOIN {L} l ON l.link_id = m.ib_link_id
            WHERE l.to_node_id <> m.node_id""", f"SELECT count(*) FROM {M}"),
        ("movement turns where its outbound link starts", f"""SELECT count(*) FROM {M} m JOIN {L} l ON l.link_id = m.ob_link_id
            WHERE l.from_node_id <> m.node_id""", f"SELECT count(*) FROM {M}"),
        ("movement ib and ob links differ", f"SELECT count(*) FROM {M} WHERE ib_link_id = ob_link_id", f"SELECT count(*) FROM {M}"),
        ("movement inbound lanes exist", f"""SELECT count(*) FROM {M} m JOIN {cnt} c ON c.link_id = m.ib_link_id
            WHERE m.start_ib_lane IS NOT NULL AND (m.start_ib_lane < 1 OR m.end_ib_lane > c.n
                                                   OR m.start_ib_lane > m.end_ib_lane)""", f"SELECT count(*) FROM {M}"),
        ("movement outbound lanes exist", f"""SELECT count(*) FROM {M} m JOIN {cnt} c ON c.link_id = m.ob_link_id
            WHERE m.start_ob_lane IS NOT NULL AND (m.start_ob_lane < 1 OR m.end_ob_lane > c.n
                                                   OR m.start_ob_lane > m.end_ob_lane)""", f"SELECT count(*) FROM {M}"),
        ("movement lane ranges are as long on both sides", f"""SELECT count(*) FROM {M}
            WHERE start_ib_lane IS NOT NULL AND start_ob_lane IS NOT NULL
              AND end_ib_lane - start_ib_lane <> end_ob_lane - start_ob_lane""", f"SELECT count(*) FROM {M}"),
        ("movement has both lane ranges or neither", f"""SELECT count(*) FROM {M}
            WHERE (start_ib_lane IS NULL) <> (end_ib_lane IS NULL) OR (start_ob_lane IS NULL) <> (end_ob_lane IS NULL)""",
         f"SELECT count(*) FROM {M}"),
    ]


def check_gmns(path, modes=None):
    """Run the checks on every ``gmns_<mode>`` schema of the GMNS database ``path`` (or only ``modes``).
    Returns ``[(schema, check, bad, total), ...]``."""
    con = duckdb.connect(str(path), read_only=True)
    try:
        con.execute("LOAD spatial")
    except duckdb.Error:
        con.execute("INSTALL spatial; LOAD spatial")
    try:
        schemas = [r[0] for r in con.execute(
            "SELECT DISTINCT table_schema FROM information_schema.tables WHERE table_name = 'lane' "
            "AND table_schema LIKE 'gmns\\_%' ESCAPE '\\' ORDER BY 1").fetchall()]
        if modes:
            schemas = [s for s in schemas if s[len("gmns_"):] in modes]
        have = {(a, b) for a, b in con.execute("SELECT table_schema, table_name FROM information_schema.tables").fetchall()}
        return [(s, name, con.execute(bad).fetchone()[0], con.execute(total).fetchone()[0])
                for s in schemas for name, bad, total in _checks(s)
                if (s, "location") in have or ".location" not in bad]
    finally:
        con.close()
