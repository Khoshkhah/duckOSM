"""
Connected-component clean-up.

Keeps the largest weakly-connected component of the current mode's graph and drops
disconnected fragments (boundary-crossing stubs, isolated pockets). Runs after the
edge_graph is built (PBF mode) or after a duckdb clip. Operates on the tables of the
active schema (`USE <mode>`): edges / nodes / edge_graph / turn_restrictions.

Weakly-connected components are computed in Python via union-find on (source, target) —
fast for the area-sized graphs this runs on. `connectivity_rescue` and
`strongly_connected` are accepted but not yet implemented (logged as a no-op).
"""
import logging
from collections import Counter

from duckosm.processors.base import BaseProcessor

logger = logging.getLogger("duckosm")


class ComponentFilter(BaseProcessor):
    def __init__(self, con, keep_largest=True, min_component_edges=1,
                 connectivity_rescue=False, strongly_connected=False):
        super().__init__(con)
        self.keep_largest = keep_largest
        self.min_component_edges = max(1, int(min_component_edges))
        self.connectivity_rescue = connectivity_rescue
        self.strongly_connected = strongly_connected

    def run(self) -> None:
        if self.connectivity_rescue or self.strongly_connected:
            logger.warning("  ComponentFilter: connectivity_rescue / strongly_connected not yet "
                           "implemented — using weakly-connected keep-largest")

        edges = self.fetchall("SELECT edge_id, source, target FROM edges")
        if not edges:
            return

        parent = {}

        def find(x):
            parent.setdefault(x, x)
            root = x
            while parent[root] != root:
                root = parent[root]
            while parent[x] != root:
                parent[x], x = root, parent[x]
            return root

        for _, s, t in edges:
            parent[find(s)] = find(t)
        comp = {eid: find(s) for eid, s, t in edges}
        sizes = Counter(comp.values())
        largest = max(sizes, key=sizes.get)

        if self.keep_largest:
            keep_roots = {largest}
        else:
            keep_roots = {r for r, n in sizes.items() if n >= self.min_component_edges}
            keep_roots.add(largest)

        drop_edges = [eid for eid, r in comp.items() if r not in keep_roots]
        if not drop_edges:
            logger.info(f"  ComponentFilter: {len(sizes)} component(s), all kept "
                        f"({len(edges):,} edges)")
            return

        self.execute("CREATE OR REPLACE TEMP TABLE _drop_edges (edge_id BIGINT)")
        self.con.executemany("INSERT INTO _drop_edges VALUES (?)", [(e,) for e in drop_edges])
        self.execute("DELETE FROM edges WHERE edge_id IN (SELECT edge_id FROM _drop_edges)")
        for tbl, cols in (("edge_graph", ("from_edge", "to_edge")),
                          ("turn_restrictions", ("from_edge_id", "to_edge_id"))):
            try:
                cond = " OR ".join(f"{c} IN (SELECT edge_id FROM _drop_edges)" for c in cols)
                self.execute(f"DELETE FROM {tbl} WHERE {cond}")
            except Exception:
                pass                                          # table may not exist for this build
        self.execute("DELETE FROM nodes WHERE node_id NOT IN "
                     "(SELECT source FROM edges UNION SELECT target FROM edges)")
        self.execute("DROP TABLE IF EXISTS _drop_edges")

        kept = len(edges) - len(drop_edges)
        logger.info(f"  ComponentFilter: kept largest {kept:,}/{len(edges):,} edges; "
                    f"dropped {len(drop_edges):,} edges in {len(sizes) - 1} smaller component(s)")
