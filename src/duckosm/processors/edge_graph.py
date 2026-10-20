"""
Edge graph builder processor - builds edge adjacency graph.
"""

import logging

from duckosm.processors.base import BaseProcessor

logger = logging.getLogger("duckosm.edge_graph")

# who uses the mode's own rows of edge_graph (its `uses` column); rows into or out of a bus-only edge
# (driving's private_edges, access 'bus') are 'bus' (docs/design/bus_only_edges.md)
ROUTED_USES = {"driving": "car", "walking": "walk", "cycling": "bike"}
BRIDGE_MAX_TURN_DEG = 90.0   # on a bridge no move turns more than this: no U-turn, no turn back (only merges and forks)


RESTRICTED_USES = ("bus", "private", "pedestrian", "delivery")   # the `access` labels of private_edges that edge_graph has rows for


def check_uses(uses):
    """``uses`` (None, a comma string or a list of labels) as a tuple of labels of :data:`RESTRICTED_USES`; a ``ValueError`` names the others."""
    if not uses:
        return ()
    items = [u.strip() for u in (uses.split(",") if isinstance(uses, str) else uses) if u and u.strip()]
    bad = [u for u in items if u not in RESTRICTED_USES]
    if bad:
        raise ValueError(f"unknown uses {bad}: choose from {', '.join(RESTRICTED_USES)}")
    return tuple(dict.fromkeys(items))


def routed_graph(con, mode, uses=None):
    """The rows of ``<mode>.edge_graph`` the mode routes on, as a FROM-able SQL relation: the rows of its
    own edges (``uses`` = :data:`ROUTED_USES`), and no others. ``uses``: more labels to take (``bus``, ``private``,
    ``pedestrian``, ``delivery``): the turns into and out of those restricted edges, to look at the graph, not to route
    the mode on (docs/design/restricted_edge_graph_rows.md). A graph built before the ``uses`` column holds only the
    mode's own rows."""
    extra = check_uses(uses)
    has = con.execute("SELECT count(*) FROM information_schema.columns WHERE table_schema = ? "
                      "AND table_name = 'edge_graph' AND column_name = 'uses'", [mode]).fetchone()[0]
    if not has:
        return f"{mode}.edge_graph"
    wanted = ", ".join(f"'{u}'" for u in (ROUTED_USES[mode], *extra))
    return f"(SELECT * FROM {mode}.edge_graph WHERE uses IN ({wanted}))"


class EdgeGraphBuilder(BaseProcessor):
    """
    Build edge adjacency graph for routing.
    
    Creates:
        - edge_graph: Pairs of connected edges (from_edge -> to_edge), open to all traffic
        - turn_permission: what edge_graph does not say: turns open only to some vehicles, and turns
          banned only for some vehicles or only at some times (docs/design/turn_permissions.md)
    
    Two edges are connected if the target of one is the source of another.
    Turn restrictions are excluded. Each row says who may use it (``uses``): the mode's own
    (``ROUTED_USES``, e.g. 'car') between two of its edges; the ``access`` label of a restricted edge
    (``private_edges``: 'bus', 'private', 'pedestrian', 'delivery') into or out of it, by the same rules; a
    turn between two restricted edges of different labels has no row. Every reader takes the mode's own rows
    by default and has a ``uses`` option for the others (:func:`routed_graph`, docs/design/restricted_edge_graph_rows.md).
    """
    
    def __init__(self, con, mode):
        super().__init__(con)
        self.mode = mode                         # the tables are the active schema's (USE <mode>)

    def run(self) -> None:
        """Build edge graph."""
        self._create_edge_graph()
        self._remove_bridge_turnbacks()
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
        if has_bus:          # the same join over the edges and the restricted edges, the rows with a restricted edge, labelled by its access
            labels = ", ".join(f"'{u}'" for u in RESTRICTED_USES)
            self.execute(f"""
                INSERT INTO edge_graph
                WITH e AS (SELECT edge_id, source, target, cost_s, NULL AS lab FROM edges
                           UNION ALL SELECT edge_id, source, target, cost_s, access FROM private_edges WHERE access IN ({labels}))
                SELECT e1.edge_id, e2.edge_id, e2.edge_id, e1.cost_s, coalesce(e1.lab, e2.lab)
                FROM e e1 INNER JOIN e e2 ON e1.target = e2.source
                WHERE e1.edge_id != e2.edge_id AND (e1.lab IS NOT NULL OR e2.lab IS NOT NULL)
                  AND (e1.lab IS NULL OR e2.lab IS NULL OR e1.lab = e2.lab)     -- no user takes two different restricted edges
            """)
    
    def _remove_bridge_turnbacks(self) -> None:
        """On a bridge there is no U-turn and no turn back: from one bridge edge to the next at their shared node, a move that turns more
        than BRIDGE_MAX_TURN_DEG is removed (a bridge has only merges and forks: a median or a barrier stands between its two directions).
        Without it a two-way bridge road mapped as one way got a U-turn at every node where a ramp leaves, and SUMO gave that U-turn the
        left lane, so the lanes after the fork were matched one place to the side (Granville Bridge, 9 such moves; urbanstyle 2026-10-19)."""
        has = self.fetchone("""SELECT count(*) FROM information_schema.columns WHERE table_schema = current_schema()
                               AND table_name = 'edges' AND column_name = 'bridge'""")[0]
        if not has:
            return
        n = self.fetchone(f"""
            WITH h AS (
                SELECT edge_id, coalesce(bridge, 'no') NOT IN ('no', '') AS on_bridge,
                       -- the heading at the edge's end and at its start (degrees, x east y north, true metres at its latitude)
                       degrees(atan2(ST_Y(ST_EndPoint(geometry)) - ST_Y(ST_PointN(geometry, (ST_NPoints(geometry) - 1)::INTEGER)),
                                     (ST_X(ST_EndPoint(geometry)) - ST_X(ST_PointN(geometry, (ST_NPoints(geometry) - 1)::INTEGER)))
                                     * cos(radians(ST_Y(ST_EndPoint(geometry)))))) AS h_end,
                       degrees(atan2(ST_Y(ST_PointN(geometry, 2)) - ST_Y(ST_StartPoint(geometry)),
                                     (ST_X(ST_PointN(geometry, 2)) - ST_X(ST_StartPoint(geometry)))
                                     * cos(radians(ST_Y(ST_StartPoint(geometry)))))) AS h_start
                FROM edges WHERE ST_NPoints(geometry) >= 2)
            SELECT count(*) FROM edge_graph g JOIN h a ON a.edge_id = g.from_edge JOIN h b ON b.edge_id = g.to_edge
            WHERE a.on_bridge AND b.on_bridge
              AND abs(((b.h_start - a.h_end + 540) % 360) - 180) > {BRIDGE_MAX_TURN_DEG}""")[0]
        self.execute(f"""
            DELETE FROM edge_graph WHERE (from_edge, to_edge) IN (
            WITH h AS (
                SELECT edge_id, coalesce(bridge, 'no') NOT IN ('no', '') AS on_bridge,
                       degrees(atan2(ST_Y(ST_EndPoint(geometry)) - ST_Y(ST_PointN(geometry, (ST_NPoints(geometry) - 1)::INTEGER)),
                                     (ST_X(ST_EndPoint(geometry)) - ST_X(ST_PointN(geometry, (ST_NPoints(geometry) - 1)::INTEGER)))
                                     * cos(radians(ST_Y(ST_EndPoint(geometry)))))) AS h_end,
                       degrees(atan2(ST_Y(ST_PointN(geometry, 2)) - ST_Y(ST_StartPoint(geometry)),
                                     (ST_X(ST_PointN(geometry, 2)) - ST_X(ST_StartPoint(geometry)))
                                     * cos(radians(ST_Y(ST_StartPoint(geometry)))))) AS h_start
                FROM edges WHERE ST_NPoints(geometry) >= 2)
            SELECT g.from_edge, g.to_edge FROM edge_graph g JOIN h a ON a.edge_id = g.from_edge JOIN h b ON b.edge_id = g.to_edge
            WHERE a.on_bridge AND b.on_bridge
              AND abs(((b.h_start - a.h_end + 540) % 360) - 180) > {BRIDGE_MAX_TURN_DEG})""")
        if n:
            logger.info(f"[{self.mode}] {n:,} turn-backs on bridges removed from the edge graph (no U-turn on a bridge)")

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
