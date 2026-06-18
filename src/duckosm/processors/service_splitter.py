"""
Service-road splitter — the final per-mode pipeline step.

Moves `highway = 'service'` edges (driveways, parking aisles, alleys, …) OUT of the routing
graph and into a separate `service_edges` table, then prunes them from the derived tables so
the main graph stays consistent and routes only on "real" roads:

  - `service_edges` ← the service rows (same schema as `edges`)
  - `edges`         ← service rows removed
  - `edge_graph_with_service` ← the FULL graph (incl. service) kept as a snapshot
  - `edge_graph`    ← transitions touching a removed edge dropped (service-free routing graph)
  - `turn_restrictions` ← rows referencing a removed edge dropped
  - `edge_id_map`   ← rows whose merged edge was removed dropped
  - `nodes`         ← nodes no longer touched by any remaining edge dropped

So routing can use `edge_graph` (real roads only) or `edge_graph_with_service` (includes
driveways/alleys); resolve geometry for either from `edges` ∪ `service_edges`.

Visualization still shows every road: `duckosm.viz` unions `edges` + `service_edges`.

Idempotent and safe for the duckdb-clip path: `service_edges` is created if missing and only
*appended* to, so a clip that already inherited a parent's `service_edges` is left intact when
its `edges` has no service rows to move.
"""
import logging

from duckosm.processors.base import BaseProcessor

logger = logging.getLogger("duckosm")


class ServiceSplitter(BaseProcessor):
    """Separate `highway='service'` edges into `<mode>.service_edges` and prune the main graph."""

    def run(self) -> None:
        # Ensure the table exists (empty, same schema) so consumers + viz can always reference it.
        self.execute("CREATE TABLE IF NOT EXISTS service_edges AS SELECT * FROM edges WHERE FALSE")

        n = self.fetchone("SELECT COUNT(*) FROM edges WHERE highway = 'service'")[0]
        if n == 0:
            logger.info("  separate_service: no service edges to move")
            return

        # Back up the FULL graph (with service) before pruning, so routing can fall back to it
        # when service roads are needed. The default `edge_graph` is then the service-free graph.
        try:
            self.execute("CREATE OR REPLACE TABLE edge_graph_with_service AS SELECT * FROM edge_graph")
        except Exception:
            pass  # build_graph off — no edge_graph to back up

        self.execute("INSERT INTO service_edges SELECT * FROM edges WHERE highway = 'service'")
        self.execute("DELETE FROM edges WHERE highway = 'service'")

        # Prune the derived tables of references to the just-removed edges (best-effort: some
        # may not exist depending on options/build path).
        for sql in (
            "DELETE FROM edge_graph WHERE from_edge NOT IN (SELECT edge_id FROM edges) "
            "OR to_edge NOT IN (SELECT edge_id FROM edges)",
            "DELETE FROM turn_restrictions WHERE from_edge_id NOT IN (SELECT edge_id FROM edges) "
            "OR to_edge_id NOT IN (SELECT edge_id FROM edges)",
            "DELETE FROM edge_id_map WHERE new_edge_id NOT IN (SELECT edge_id FROM edges)",
            "DELETE FROM nodes WHERE node_id NOT IN "
            "(SELECT source FROM edges UNION SELECT target FROM edges)",
        ):
            try:
                self.execute(sql)
            except Exception:
                pass  # table absent for this mode/build — nothing to prune

        n_svc = self.fetchone("SELECT COUNT(*) FROM service_edges")[0]
        n_edges = self.fetchone("SELECT COUNT(*) FROM edges")[0]
        logger.info(f"  separate_service: moved {n:,} service edge(s) -> service_edges "
                    f"({n_svc:,} total); {n_edges:,} edge(s) remain in the routing graph")
