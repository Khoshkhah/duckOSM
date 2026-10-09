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
        self._n_turn_rules = 0
        self._extract_raw_restrictions()
        self._map_to_edges()
        if self._n_turn_rules:                    # synthetic rules carry negative restriction_ids
            matched = self.fetchone(
                "SELECT count(DISTINCT restriction_id) FROM turn_restrictions WHERE restriction_id < 0")[0]
            rule = "turn rule" if self._n_turn_rules == 1 else "turn rules"
            logger.info(f"  OSM fixes ({self.overrides_path.name}): "
                        f"{matched} of {self._n_turn_rules} {rule} matched this area")
    
    def _extract_raw_restrictions(self) -> None:
        """Extract restriction relations from raw OSM data: one row per rule a relation carries
        (docs/design/turn_permissions.md). `restriction=*` binds every vehicle, always;
        `restriction:conditional=* @ (...)` only while the condition holds; `restriction:<vehicle>=*`
        only that vehicle class. `except=*` (the vehicles exempt) goes with every rule of the relation."""
        self.execute("""
            CREATE OR REPLACE TABLE restrictions_raw AS
            WITH kv AS (
                SELECT osm_id, unnest(map_keys(tags)) AS k, unnest(map_values(tags)) AS v,
                       map_extract(tags, 'except')[1] AS except_vehicles, refs, ref_roles, ref_types
                FROM raw.relations
                WHERE map_extract(tags, 'type')[1] = 'restriction'
            )
            SELECT
                osm_id AS restriction_id,
                CASE WHEN k = 'restriction:conditional' THEN trim(split_part(v, '@', 1)) ELSE trim(v) END AS restriction_type,
                except_vehicles,
                CASE WHEN k LIKE 'restriction:%' AND k <> 'restriction:conditional' THEN substr(k, 13) END AS applies_to,
                CASE WHEN k = 'restriction:conditional'
                     THEN trim(regexp_replace(trim(split_part(v, '@', 2)), '^\\(|\\)$', '', 'g')) END AS condition,
                refs,
                ref_roles,
                ref_types
            FROM kv
            WHERE (k = 'restriction' OR (k LIKE 'restriction:%' AND k NOT LIKE '%:%:%'))
              AND v IS NOT NULL
        """)
    
    def _map_to_edges(self) -> None:
        """Map restrictions to edge IDs."""
        # Unnest the parallel arrays to get from/via/to
        self.execute("""
            CREATE OR REPLACE TABLE restrictions_unnested AS
            SELECT
                restriction_id,
                restriction_type, except_vehicles, applies_to, condition,
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
                restriction_type, except_vehicles, applies_to, condition,
                MAX(CASE WHEN role = 'from' AND ref_type = 'way' THEN ref_id END) AS from_way,
                MAX(CASE WHEN role = 'via' AND ref_type = 'node' THEN ref_id END) AS via_node,
                MAX(CASE WHEN role = 'to' AND ref_type = 'way' THEN ref_id END) AS to_way
            FROM restrictions_unnested
            GROUP BY restriction_id, restriction_type, except_vehicles, applies_to, condition
        """)
        
        # Add any synthetic restrictions (from osm_overrides.yaml) — same shape, so they map below too.
        self._n_turn_rules = self._inject_overrides()

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
            CREATE OR REPLACE TEMP TABLE src_way AS     -- the way at each edge's SOURCE end (first refs pair)
                SELECT e.edge_id, e.source AS node, w1.way_id
                FROM edges e
                JOIN way_nodes w1 ON w1.node_id = e.refs[1]
                JOIN way_nodes w2 ON w2.way_id = w1.way_id
                                 AND w2.node_id = e.refs[2] AND abs(w2.seq - w1.seq) = 1
        """)
        self.execute("""
            CREATE OR REPLACE TEMP TABLE tgt_way AS     -- the way at each edge's TARGET end (last refs pair)
                SELECT e.edge_id, e.target AS node, w1.way_id
                FROM edges e
                JOIN way_nodes w1 ON w1.node_id = e.refs[len(e.refs) - 1]
                JOIN way_nodes w2 ON w2.way_id = w1.way_id
                                 AND w2.node_id = e.refs[len(e.refs)] AND abs(w2.seq - w1.seq) = 1
        """)
        self.execute("""
            CREATE OR REPLACE TABLE turn_restrictions AS
            SELECT DISTINCT
                r.restriction_id,
                r.restriction_type,
                r.except_vehicles,      -- vehicles exempt from the rule (OSM except=, ';'-separated), NULL: none
                r.applies_to,           -- the one vehicle class it binds (restriction:<vehicle>), NULL: all
                r.condition,            -- when it binds (restriction:conditional), NULL: always
                r.via_node,
                fe.edge_id AS from_edge_id,
                te.edge_id AS to_edge_id
            FROM restrictions_pivoted r
            -- From edge: ends at via_node, its target-end way is from_way
            JOIN tgt_way fe ON fe.node = r.via_node AND fe.way_id = r.from_way
            -- To edge: starts at via_node, its source-end way is to_way
            JOIN src_way te ON te.node = r.via_node AND te.way_id = r.to_way
        """)

        self._map_via_ways()

        # Cleanup temp tables
        self.execute("DROP TABLE IF EXISTS src_way")
        self.execute("DROP TABLE IF EXISTS tgt_way")
        self.execute("DROP TABLE IF EXISTS restrictions_raw")
        self.execute("DROP TABLE IF EXISTS restrictions_unnested")
        self.execute("DROP TABLE IF EXISTS restrictions_pivoted")

    def _map_via_ways(self) -> None:
        """Restrictions with a via WAY (from way, along the via way(s), onto the to way) ban or mandate a path, not one turn: each
        becomes one row per edge path in `turn_path_restrictions` (from_edge, via_edges, to_edge), with its exceptions, vehicle
        class and condition. edge_graph cannot carry them (a pair of turns banned alone would ban legal moves too); exporters that
        can say a path use them (docs/design/turn_permissions.md)."""
        self.execute("""CREATE OR REPLACE TABLE turn_path_restrictions (restriction_id BIGINT, restriction_type VARCHAR,
                         except_vehicles VARCHAR, applies_to VARCHAR, condition VARCHAR, from_edge BIGINT, via_edges BIGINT[], to_edge BIGINT)""")
        rules = self.fetchall("""
            SELECT restriction_id, restriction_type, except_vehicles, applies_to, condition,
                   refs[list_position(ref_roles, 'from')] AS from_way,
                   list_transform(list_filter(range(1, len(refs) + 1), i -> ref_roles[i] = 'via' AND ref_types[i]::VARCHAR = 'way'),
                                  i -> refs[i]) AS via_ways,
                   refs[list_position(ref_roles, 'to')] AS to_way
            FROM restrictions_raw
            WHERE len(list_filter(range(1, len(refs) + 1), i -> ref_roles[i] = 'via' AND ref_types[i]::VARCHAR = 'way')) > 0""")
        if not rules:
            return
        ways = {w for r in rules for w in (r[5], r[7], *r[6])}
        ends = {}            # edge -> (source, target, way at the source end, way at the target end)
        for eid, src, tgt, sw, tw in self.fetchall(f"""
                SELECT e.edge_id, e.source, e.target, s.way_id, t.way_id FROM edges e
                LEFT JOIN src_way s USING (edge_id) LEFT JOIN tgt_way t USING (edge_id)
                WHERE s.way_id IN ({','.join(map(str, ways))}) OR t.way_id IN ({','.join(map(str, ways))})"""):
            ends[eid] = (src, tgt, sw, tw)
        out_of = {}
        for eid, (src, *_ ) in ends.items():
            out_of.setdefault(src, []).append(eid)
        rows = []
        for rid, rtype, exc, veh, cond, fw, vws, tw in rules:
            vset = set(vws)
            for fe, (_, ftgt, _, ftw) in ends.items():
                if ftw != fw:
                    continue
                todo = [(ftgt, [], {ftgt})]                      # along edges of the via way(s), never through a node twice
                while todo:
                    node, path, seen = todo.pop()
                    for e in out_of.get(node, []):
                        esrc, etgt, esw, etw = ends[e]
                        if path and esw == tw:                   # onto the to way, having passed along the via way(s)
                            rows.append((rid, rtype, exc, veh, cond, fe, list(path), e))
                        if (esw in vset or etw in vset) and etgt not in seen and len(path) < 6:
                            todo.append((etgt, path + [e], seen | {etgt}))
        if rows:
            self.con.executemany("INSERT INTO turn_path_restrictions VALUES (?, ?, ?, ?, ?, ?, ?, ?)", rows)
        n = len({r[0] for r in rows})
        logger.info(f"  via-way restrictions: {n} of {len({r[0] for r in rules})} matched edge paths ({len(rows)} paths)")

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
        return len(vals)
