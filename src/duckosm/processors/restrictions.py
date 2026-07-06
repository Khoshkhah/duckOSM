"""
Restriction processor - extracts and maps turn restrictions.

Also injects **synthetic** turn restrictions from the OSM-overrides file (`turn_restrictions:` in
`osm_overrides/osm_overrides.yaml`) for junctions where OSM is missing a `type=restriction` relation
that physically exists — see `docs/design/turn-restriction-overrides.md` and
`osm_overrides/known_osm_issues.md` #6.
Synthetic rules ride the same `from_way → via_node → to_way` mapping as OSM restrictions, so merged and
reverse edges are handled identically, and the edge graph / routing / GMNS / exports all honour them.
"""
import logging
from pathlib import Path

from duckosm.processors.base import BaseProcessor

logger = logging.getLogger("duckosm.restrictions")

# OSM `restriction` values the edge graph understands (no_* remove the turn; only_* mandate it).
_ALLOWED_RESTRICTIONS = {
    "no_u_turn", "no_left_turn", "no_right_turn", "no_straight_on", "no_entry", "no_exit",
    "only_straight_on", "only_left_turn", "only_right_turn", "only_u_turn",
}


class RestrictionProcessor(BaseProcessor):
    """
    Extract turn restrictions from OSM relations (+ synthetic overrides).

    Creates:
        - turn_restrictions: Restriction rules mapped to edge IDs
    """

    def __init__(self, con, overrides_path=None):
        super().__init__(con)
        self.overrides_path = Path(overrides_path) if overrides_path else None

    def run(self) -> None:
        """Extract and process restrictions."""
        self._extract_raw_restrictions()
        self._map_to_edges()
    
    def _extract_raw_restrictions(self) -> None:
        """Extract restriction relations from raw OSM data."""
        # Use map_extract for reliable tag access in DuckDB
        self.execute("""
            CREATE OR REPLACE TABLE restrictions_raw AS
            SELECT 
                osm_id AS restriction_id,
                map_extract(tags, 'restriction')[1] AS restriction_type,
                refs,
                ref_roles,
                ref_types
            FROM raw.relations
            WHERE map_extract(tags, 'type')[1] = 'restriction'
            AND map_extract(tags, 'restriction')[1] IS NOT NULL
        """)
    
    def _map_to_edges(self) -> None:
        """Map restrictions to edge IDs."""
        # Unnest the parallel arrays to get from/via/to
        self.execute("""
            CREATE OR REPLACE TABLE restrictions_unnested AS
            SELECT 
                restriction_id,
                restriction_type,
                UNNEST(refs) AS ref_id,
                UNNEST(ref_roles) AS role,
                UNNEST(ref_types) AS ref_type
            FROM restrictions_raw
        """)
        
        # Pivot to get from_way, via_node, to_way
        self.execute("""
            CREATE OR REPLACE TABLE restrictions_pivoted AS
            SELECT 
                restriction_id,
                restriction_type,
                MAX(CASE WHEN role = 'from' AND ref_type = 'way' THEN ref_id END) AS from_way,
                MAX(CASE WHEN role = 'via' AND ref_type = 'node' THEN ref_id END) AS via_node,
                MAX(CASE WHEN role = 'to' AND ref_type = 'way' THEN ref_id END) AS to_way
            FROM restrictions_unnested
            GROUP BY restriction_id, restriction_type
        """)
        
        # Add any synthetic restrictions (from osm_overrides.yaml) — same shape, so they map below too.
        self._inject_overrides()

        # Map to edge IDs by the way that is INCIDENT to the via node — i.e. each edge's END
        # segment, not its single representative osm_id. A merged edge spans several ways and
        # keeps only one representative osm_id (its source-end member), so matching `from_way`
        # against `edges.osm_id` silently drops any restriction whose way is a non-representative
        # member (and every reverse edge, whose source-end way is the forward twin's last
        # member). Instead read the incident way from `refs` (the full stitched node list, on
        # every edge incl. merged/reverse) via way_nodes: the from edge ends at via_node, so its
        # target-end way must equal from_way; the to edge starts at via_node, so its source-end
        # way must equal to_way. Correct for singleton, merged, and reverse edges alike.
        self.execute("""
            CREATE OR REPLACE TABLE turn_restrictions AS
            WITH src_way AS (   -- the way at each edge's SOURCE end (first refs pair)
                SELECT e.edge_id, e.source AS node, w1.way_id
                FROM edges e
                JOIN way_nodes w1 ON w1.node_id = e.refs[1]
                JOIN way_nodes w2 ON w2.way_id = w1.way_id
                                 AND w2.node_id = e.refs[2] AND abs(w2.seq - w1.seq) = 1
            ),
            tgt_way AS (        -- the way at each edge's TARGET end (last refs pair)
                SELECT e.edge_id, e.target AS node, w1.way_id
                FROM edges e
                JOIN way_nodes w1 ON w1.node_id = e.refs[len(e.refs) - 1]
                JOIN way_nodes w2 ON w2.way_id = w1.way_id
                                 AND w2.node_id = e.refs[len(e.refs)] AND abs(w2.seq - w1.seq) = 1
            )
            SELECT DISTINCT
                r.restriction_id,
                r.restriction_type,
                r.via_node,
                fe.edge_id AS from_edge_id,
                te.edge_id AS to_edge_id
            FROM restrictions_pivoted r
            -- From edge: ends at via_node, its target-end way is from_way
            JOIN tgt_way fe ON fe.node = r.via_node AND fe.way_id = r.from_way
            -- To edge: starts at via_node, its source-end way is to_way
            JOIN src_way te ON te.node = r.via_node AND te.way_id = r.to_way
        """)

        # Cleanup temp tables
        self.execute("DROP TABLE IF EXISTS restrictions_raw")
        self.execute("DROP TABLE IF EXISTS restrictions_unnested")
        self.execute("DROP TABLE IF EXISTS restrictions_pivoted")

    def _inject_overrides(self) -> int:
        """Insert synthetic `turn_restrictions:` rules from the overrides file into
        `restrictions_pivoted` (so the mapping below turns them into edge-level restrictions). Rules
        whose ways/node aren't in this area simply map to no edges — a global no-op. Returns the count
        of rules injected (a synthetic negative `restriction_id` keeps them clear of real relation ids)."""
        if not self.overrides_path or not self.overrides_path.exists():
            return 0
        import yaml
        rules = (yaml.safe_load(self.overrides_path.read_text()) or {}).get("turn_restrictions") or []
        vals = []
        for i, r in enumerate(rules):
            try:
                fw, vn, tw = int(r["from_way"]), int(r["via_node"]), int(r["to_way"])
            except (KeyError, TypeError, ValueError):
                continue
            rt = str(r.get("restriction", "no_u_turn"))
            if rt not in _ALLOWED_RESTRICTIONS:                 # allow-list → injection-safe string
                logger.warning(f"  turn_restriction override skipped: unknown restriction '{rt}'")
                continue
            vals.append(f"({-(i + 1)}::BIGINT,'{rt}',{fw}::BIGINT,{vn}::BIGINT,{tw}::BIGINT)")
        if not vals:
            return 0
        self.execute(
            "INSERT INTO restrictions_pivoted (restriction_id, restriction_type, from_way, via_node, to_way) "
            f"SELECT * FROM (VALUES {','.join(vals)}) "
            "AS t(restriction_id, restriction_type, from_way, via_node, to_way)")
        logger.info(f"  injected {len(vals)} synthetic turn-restriction override(s)")
        return len(vals)
