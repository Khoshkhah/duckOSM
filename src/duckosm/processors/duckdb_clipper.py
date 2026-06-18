"""
Clip an area's network out of a parent duckOSM db, preserving edge_ids.

`source.type: duckdb` mode. Instead of building from a PBF, this copies the rows of an
existing parent build (e.g. sweden.duckdb) that fall inside the boundary, for one mode
schema. Because edges are copied VERBATIM (same osm_id / source / target / geometry),
their `edge_id` hash is unchanged — the clipped area is a stable view of its parent,
with no re-key for downstream projects.

Assumes: the parent db is ATTACHed as `parent`, and the boundary polygon is loaded into
`main.boundary` (column `geom`). Writes edges / nodes / edge_graph / turn_restrictions
into the active schema. A ComponentFilter pass afterwards removes boundary stubs.
"""
import logging

from duckosm.processors.base import BaseProcessor

logger = logging.getLogger("duckosm")

_PRED = {
    "within": "ST_Within(e.geometry, b.geom)",            # whole edge inside the polygon
    "intersects": "ST_Intersects(e.geometry, b.geom)",    # edge touches the polygon
    "centroid": "ST_Within(ST_Centroid(e.geometry), b.geom)",
}


class DuckdbClipper(BaseProcessor):
    def __init__(self, con, mode, parent_alias="parent", predicate="intersects"):
        super().__init__(con)
        self.mode = mode
        self.parent = parent_alias
        self.predicate = predicate if predicate in _PRED else "intersects"

    def run(self) -> None:
        p, m, pred = self.parent, self.mode, _PRED[self.predicate]

        # edges inside the boundary — edge_id (and everything else) preserved verbatim
        self.execute(f"""
            CREATE OR REPLACE TABLE edges AS
            SELECT e.* FROM {p}.{m}.edges e
            WHERE EXISTS (SELECT 1 FROM main.boundary b WHERE {pred})
        """)
        # nodes touched by surviving edges
        self.execute(f"""
            CREATE OR REPLACE TABLE nodes AS
            SELECT n.* FROM {p}.{m}.nodes n
            WHERE n.node_id IN (SELECT source FROM edges UNION SELECT target FROM edges)
        """)
        # edge_graph rows whose BOTH endpoints survive the clip
        try:
            self.execute(f"""
                CREATE OR REPLACE TABLE edge_graph AS
                SELECT g.* FROM {p}.{m}.edge_graph g
                WHERE g.from_edge IN (SELECT edge_id FROM edges)
                  AND g.to_edge   IN (SELECT edge_id FROM edges)
            """)
        except Exception as e:
            logger.warning(f"  [{m}] parent has no edge_graph to clip ({e})")
        # turn restrictions whose edges survive (driving)
        try:
            self.execute(f"""
                CREATE OR REPLACE TABLE turn_restrictions AS
                SELECT t.* FROM {p}.{m}.turn_restrictions t
                WHERE t.from_edge_id IN (SELECT edge_id FROM edges)
                  AND t.to_edge_id   IN (SELECT edge_id FROM edges)
            """)
        except Exception:
            pass                                              # no restrictions for this mode/build
        # service roads inside the boundary (parent built with separate_service) — kept out of
        # the routing graph but carried along so viz can still show every road
        try:
            self.execute(f"""
                CREATE OR REPLACE TABLE service_edges AS
                SELECT e.* FROM {p}.{m}.service_edges e
                WHERE EXISTS (SELECT 1 FROM main.boundary b WHERE {pred})
            """)
        except Exception:
            pass                                              # parent has no service_edges
        # full (with-service) edge graph — endpoints survive among edges OR service_edges
        try:
            self.execute("""
                CREATE OR REPLACE TABLE edge_graph_with_service AS
                SELECT g.* FROM {p}.{m}.edge_graph_with_service g
                WHERE g.from_edge IN (SELECT edge_id FROM edges UNION SELECT edge_id FROM service_edges)
                  AND g.to_edge   IN (SELECT edge_id FROM edges UNION SELECT edge_id FROM service_edges)
            """.format(p=p, m=m))
        except Exception:
            pass                                              # parent has no edge_graph_with_service

        n_e = self.fetchone("SELECT COUNT(*) FROM edges")[0]
        n_n = self.fetchone("SELECT COUNT(*) FROM nodes")[0]
        logger.info(f"  [{m}] clipped {n_e:,} edges / {n_n:,} nodes from {p}.{m} "
                    f"(predicate={self.predicate}, edge_ids preserved)")
