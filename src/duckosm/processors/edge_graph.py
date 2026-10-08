"""
Edge graph builder processor - builds edge adjacency graph.
"""

from duckosm.processors.base import BaseProcessor

# who uses the mode's own rows of edge_graph (its `uses` column); rows into or out of a bus-only edge
# (driving's private_edges, access 'bus') are 'bus' (docs/design/bus_only_edges.md)
ROUTED_USES = {"driving": "car", "walking": "walk", "cycling": "bike"}


def routed_graph(con, mode):
    """The rows of ``<mode>.edge_graph`` the mode routes on, as a FROM-able SQL relation: the rows of its
    own edges (``uses`` = :data:`ROUTED_USES`), not the bus rows. A graph built before the ``uses`` column
    holds only those rows."""
    has = con.execute("SELECT count(*) FROM information_schema.columns WHERE table_schema = ? "
                      "AND table_name = 'edge_graph' AND column_name = 'uses'", [mode]).fetchone()[0]
    return f"(SELECT * FROM {mode}.edge_graph WHERE uses = '{ROUTED_USES[mode]}')" if has else f"{mode}.edge_graph"


class EdgeGraphBuilder(BaseProcessor):
    """
    Build edge adjacency graph for routing.
    
    Creates:
        - edge_graph: Pairs of connected edges (from_edge -> to_edge)
    
    Two edges are connected if the target of one is the source of another.
    Turn restrictions are excluded. Each row says who may use it (``uses``): the mode's own
    (``ROUTED_USES``, e.g. 'car') between two of its edges; 'bus' into or out of a bus-only edge
    (``private_edges`` with access 'bus', driving), by the same rules. Routing reads only the
    mode's own rows (:func:`routed_graph`); the GMNS export reads all.
    """
    
    def __init__(self, con, mode):
        super().__init__(con)
        self.mode = mode                         # the tables are the active schema's (USE <mode>)

    def run(self) -> None:
        """Build edge graph."""
        self._create_edge_graph()
        self._remove_restricted_turns()
    
    def _create_edge_graph(self) -> None:
        """Create edge adjacency table (line graph for edge-based routing).
        
        Structure:
        - from_edge: The incoming edge
        - to_edge: The outgoing edge
        - via_edge: Same as to_edge (for shortcut table compatibility)
        - cost: Travel cost of the FROM edge
        - uses: who may take the turn (see the class)
        """
        self.execute(f"""
            CREATE OR REPLACE TABLE edge_graph AS
            SELECT 
                e1.edge_id AS from_edge,
                e2.edge_id AS to_edge,
                e2.edge_id AS via_edge,
                e1.cost_s AS cost,
                '{ROUTED_USES[self.mode]}' AS uses
            FROM edges e1
            INNER JOIN edges e2 ON e1.target = e2.source
            WHERE e1.edge_id != e2.edge_id  -- No self-loops
        """)
        has_bus = self.fetchone("""SELECT count(*) FROM information_schema.columns WHERE table_schema = current_schema()
                                   AND table_name = 'private_edges' AND column_name = 'access'""")[0]
        if has_bus:          # the same join over the edges and the bus-only edges, the rows with a bus edge
            self.execute("""
                INSERT INTO edge_graph
                WITH e AS (SELECT edge_id, source, target, cost_s, false AS bus FROM edges
                           UNION ALL SELECT edge_id, source, target, cost_s, true FROM private_edges WHERE access = 'bus')
                SELECT e1.edge_id, e2.edge_id, e2.edge_id, e1.cost_s, 'bus'
                FROM e e1 INNER JOIN e e2 ON e1.target = e2.source
                WHERE e1.edge_id != e2.edge_id AND (e1.bus OR e2.bus)
            """)
    
    def _remove_restricted_turns(self) -> None:
        """Remove edge-graph transitions forbidden by turn restrictions.

        Handles every restriction type, which split into two semantics:
        - ``no_*`` (no_left_turn, no_u_turn, no_entry, …): the named ``from -> to`` turn is
          prohibited — drop exactly that transition.
        - ``only_*`` (only_straight_on, only_left_turn, …): the named ``from -> to`` turn is
          MANDATORY — drop every *other* transition out of ``from_edge``. All of from_edge's
          successors leave the via node (edge_graph joins ``e1.target = e2.source`` and the
          restriction's via_node = from_edge.target), so keeping only ``to_edge`` enforces it.
        """
        # Check if turn_restrictions table exists in current schema
        result = self.fetchone("""
            SELECT COUNT(*) FROM information_schema.tables
            WHERE table_name = 'turn_restrictions'
            AND table_schema = current_schema()
        """)

        if result[0] == 0:
            return

        # no_*: drop the single prohibited transition.
        self.execute("""
            DELETE FROM edge_graph
            WHERE EXISTS (
                SELECT 1 FROM turn_restrictions tr
                WHERE tr.from_edge_id = edge_graph.from_edge
                AND tr.to_edge_id = edge_graph.to_edge
                AND tr.restriction_type LIKE 'no_%'
            )
        """)
        # only_*: drop every transition out of from_edge EXCEPT the mandated to_edge.
        self.execute("""
            DELETE FROM edge_graph
            WHERE EXISTS (
                SELECT 1 FROM turn_restrictions tr
                WHERE tr.from_edge_id = edge_graph.from_edge
                AND tr.restriction_type LIKE 'only_%'
                AND edge_graph.to_edge <> tr.to_edge_id
            )
        """)
