"""
Edge graph builder processor - builds edge adjacency graph.
"""

from duckosm.processors.base import BaseProcessor


class EdgeGraphBuilder(BaseProcessor):
    """
    Build edge adjacency graph for routing.
    
    Creates:
        - edge_graph: Pairs of connected edges (from_edge -> to_edge), open to all traffic
        - turn_permission: what edge_graph does not say: turns open only to some vehicles, and turns
          banned only for some vehicles or only at some times (docs/design/turn_permissions.md)
    
    Two edges are connected if the target of one is the source of another.
    Turn restrictions are excluded.
    """
    
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
        """
        self.execute("""
            CREATE OR REPLACE TABLE edge_graph AS
            SELECT 
                e1.edge_id AS from_edge,
                e2.edge_id AS to_edge,
                e2.edge_id AS via_edge,
                e1.cost_s AS cost
            FROM edges e1
            INNER JOIN edges e2 ON e1.target = e2.source
            WHERE e1.edge_id != e2.edge_id  -- No self-loops
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

        self.execute("""CREATE OR REPLACE TABLE turn_permission (from_edge BIGINT, to_edge BIGINT, allowed BOOLEAN,
                         vehicles VARCHAR, except_vehicles VARCHAR, condition VARCHAR, restriction_id BIGINT)""")
        if result[0] == 0:
            return

        for col in ("except_vehicles", "applies_to", "condition"):     # a table from before these columns: every rule binds all, always
            self.execute(f"ALTER TABLE turn_restrictions ADD COLUMN IF NOT EXISTS {col} VARCHAR")
        # the turns a rule bans: no_* the named one, only_* every other turn out of from_edge
        banned = """
            SELECT eg.from_edge, eg.to_edge, tr.* FROM edge_graph eg JOIN turn_restrictions tr ON tr.from_edge_id = eg.from_edge
            AND ((tr.restriction_type LIKE 'no_%' AND tr.to_edge_id = eg.to_edge)
              OR (tr.restriction_type LIKE 'only_%' AND eg.to_edge <> tr.to_edge_id))"""
        # what edge_graph will not say, recorded before it changes: a ban for all with exemptions leaves the turn open to the
        # exempt vehicles; a conditional ban, or one for a single vehicle class, leaves the turn in edge_graph but closed to them
        self.execute(f"""
            INSERT INTO turn_permission
            SELECT from_edge, to_edge, true, except_vehicles, NULL, NULL, restriction_id FROM ({banned})
            WHERE applies_to IS NULL AND condition IS NULL AND except_vehicles IS NOT NULL
            UNION ALL
            SELECT from_edge, to_edge, false, applies_to, except_vehicles, condition, restriction_id FROM ({banned})
            WHERE applies_to IS NOT NULL OR condition IS NOT NULL
        """)
        # no_*: drop the single prohibited transition (a rule for all vehicles, always).
        self.execute("""
            DELETE FROM edge_graph
            WHERE EXISTS (
                SELECT 1 FROM turn_restrictions tr
                WHERE tr.from_edge_id = edge_graph.from_edge
                AND tr.to_edge_id = edge_graph.to_edge
                AND tr.restriction_type LIKE 'no_%'
                AND tr.applies_to IS NULL AND tr.condition IS NULL
            )
        """)
        # only_*: drop every transition out of from_edge EXCEPT the mandated to_edge.
        self.execute("""
            DELETE FROM edge_graph
            WHERE EXISTS (
                SELECT 1 FROM turn_restrictions tr
                WHERE tr.from_edge_id = edge_graph.from_edge
                AND tr.restriction_type LIKE 'only_%'
                AND tr.applies_to IS NULL AND tr.condition IS NULL
                AND edge_graph.to_edge <> tr.to_edge_id
            )
        """)
