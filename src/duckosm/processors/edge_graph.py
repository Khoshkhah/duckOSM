"""
Edge graph builder processor - builds edge adjacency graph.
"""

from duckosm.processors.base import BaseProcessor


class EdgeGraphBuilder(BaseProcessor):
    """
    Build edge adjacency graph for routing.
    
    Creates:
        - edge_graph: Pairs of connected edges (from_edge -> to_edge)
    
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
        """Remove edges that violate turn restrictions."""
        # Check if turn_restrictions table exists in current schema
        result = self.fetchone("""
            SELECT COUNT(*) FROM information_schema.tables 
            WHERE table_name = 'turn_restrictions'
            AND table_schema = current_schema()
        """)
        
        if result[0] == 0:
            return
        
        # Remove restricted turns
        self.execute("""
            DELETE FROM edge_graph
            WHERE EXISTS (
                SELECT 1 FROM turn_restrictions tr
                WHERE tr.from_edge_id = edge_graph.from_edge
                AND tr.to_edge_id = edge_graph.to_edge
                AND tr.restriction_type LIKE 'no_%'
            )
        """)
