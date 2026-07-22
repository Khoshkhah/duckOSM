"""
Build-time invariant validation.

Runs after a build/clip (driven by the `validation:` config block) and raises
ValidationError when `fail_on_error` and a check fails — the "fail loudly" safety net
that the per-way filter + component clean-up are supposed to satisfy.

Checks (per mode):
  - single_component      : the graph is one dominant weakly-connected component
  - no_stranded_named     : no NAMED edge sits outside the largest component
  - edge_id_stable        : reserved (needs a fixture/parent baseline) — reported as skipped
  - unique_node_id        : no duplicate node_id in nodes (catches colliding virtual nodes)
  - way_length_conserved  : no interior stretch of a kept way is missing while a kept edge of the
                            same way bridges its endpoints — the exact signature of a parallel arc
                            silently deleted instead of split (implemented topologically over the
                            way refs, which implies per-way length conservation). See
                            docs/design/split_same_direction_parallels.md.
  - layer_without_structure : WARN-only — edges tagged layer≠0 with no bridge/tunnel tag (an OSM
                            tagging smell; such edges get grade-separated draw order but no 3D
                            deck). Never fails the build.

A check result's `ok` is True (OK), False (FAIL — fatal under fail_on_error) or None
(WARN — reported but never fatal).
"""
import logging
from collections import Counter

logger = logging.getLogger("duckosm")


class ValidationError(Exception):
    pass


class Validator:
    def __init__(self, con, mode, vcfg):
        self.con = con
        self.mode = mode
        self.cfg = vcfg

    def run(self) -> list:
        rows = self.con.execute(
            f"SELECT edge_id, source, target, name FROM {self.mode}.edges").fetchall()
        results = []

        if rows:
            parent = {}

            def find(x):
                parent.setdefault(x, x)
                r = x
                while parent[r] != r:
                    r = parent[r]
                while parent[x] != r:
                    parent[x], x = r, parent[x]
                return r

            for _, s, t, _ in rows:
                parent[find(s)] = find(t)
            comp = {eid: find(s) for eid, s, t, nm in rows}
            sizes = Counter(comp.values())
            largest = max(sizes, key=sizes.get)
            n = len(rows)

            if self.cfg.assert_single_component:
                frac = sizes[largest] / n
                ok = len(sizes) == 1 or frac >= 0.999
                results.append(("single_component", ok,
                                f"{len(sizes)} component(s), largest {sizes[largest]:,}/{n:,} "
                                f"({frac:.1%})"))
            if self.cfg.assert_no_stranded_named:
                stranded = [eid for eid, s, t, nm in rows if nm and comp[eid] != largest]
                results.append(("no_stranded_named", not stranded,
                                f"{len(stranded)} named edge(s) outside the largest component"))

        if self.cfg.assert_edge_id_stable:
            results.append(("edge_id_stable", True, "skipped (no baseline configured)"))

        if self.cfg.assert_unique_node_id:
            dups = self.con.execute(f"""
                SELECT count(*) FROM (
                    SELECT node_id FROM {self.mode}.nodes
                    GROUP BY node_id HAVING count(*) > 1)""").fetchone()[0]
            results.append(("unique_node_id", dups == 0,
                            f"{dups} duplicate node_id(s) in nodes"))

        # `table_catalog = current_database()` is load-bearing: a CLIP build runs with the parent
        # ATTACHED, and information_schema spans every attached database. Without it this saw
        # `parent.driving.ways`, concluded the clipped db had a ways table, and the check below then
        # died with "Catalog Error: Table with name ways does not exist" — a clip that produced a
        # perfectly good database still exited non-zero.
        has_ways = bool(self.con.execute(
            "SELECT 1 FROM information_schema.tables "
            "WHERE table_catalog = current_database() "
            "AND table_schema = ? AND table_name = 'ways'", [self.mode]).fetchone())
        if self.cfg.assert_way_length_conserved and not has_ways:
            results.append(("way_length_conserved", True,
                            "skipped (no per-mode ways table — clip build)"))
        elif self.cfg.assert_way_length_conserved:
            # A maximal run of way refs covered by NO kept forward edge of that way, bounded by
            # two covered nodes that a kept edge of the same way connects directly — geometry was
            # deleted in favour of a parallel arc. Immune to clip/component confounders: a
            # boundary-cut tail has no bounding pair, a dropped fragment has no bridging edge.
            lost = self.con.execute(f"""
                WITH covered AS (
                    SELECT DISTINCT osm_id, unnest(refs) AS node_id
                    FROM {self.mode}.edges WHERE NOT is_reverse AND osm_id > 0
                ), marked AS (
                    SELECT wr.osm_id, wr.node_id, wr.seq, (c.node_id IS NULL) AS missing
                    FROM (SELECT w.osm_id, u.node_id, u.seq
                          FROM {self.mode}.ways w,
                               unnest(w.refs) WITH ORDINALITY AS u(node_id, seq)) wr
                    LEFT JOIN covered c
                      ON c.osm_id = wr.osm_id AND c.node_id = wr.node_id
                ), runs AS (
                    SELECT osm_id, min(seq) AS s0, max(seq) AS s1
                    FROM (SELECT osm_id, seq,
                                 seq - row_number() OVER (PARTITION BY osm_id ORDER BY seq) AS grp
                          FROM marked WHERE missing)
                    GROUP BY osm_id, grp
                )
                SELECT count(*)
                FROM runs r
                JOIN marked a ON a.osm_id = r.osm_id AND a.seq = r.s0 - 1 AND NOT a.missing
                JOIN marked b ON b.osm_id = r.osm_id AND b.seq = r.s1 + 1 AND NOT b.missing
                WHERE a.node_id <> b.node_id
                  AND EXISTS (
                    SELECT 1 FROM {self.mode}.edges e
                    WHERE e.osm_id = r.osm_id AND NOT e.is_reverse
                      AND least(e.source, e.target) = least(a.node_id, b.node_id)
                      AND greatest(e.source, e.target) = greatest(a.node_id, b.node_id))
                """).fetchone()[0]
            results.append(("way_length_conserved", lost == 0,
                            f"{lost} deleted parallel arc(s) (way stretch missing while a kept "
                            "edge bridges its endpoints)"))

        if self.cfg.warn_layer_without_structure:
            cols = {r[0] for r in self.con.execute(
                "SELECT column_name FROM information_schema.columns "
                "WHERE table_catalog = current_database() "
                "AND table_schema = ? AND table_name = 'edges'", [self.mode]).fetchall()}
            if {"layer", "bridge", "tunnel"} <= cols:
                # bridge/tunnel='no' is an explicit negative — still no structure.
                n = self.con.execute(f"""
                    SELECT count(*) FROM {self.mode}.edges
                    WHERE COALESCE(TRY_CAST(layer AS INTEGER), 0) <> 0
                      AND COALESCE(bridge, 'no') = 'no'
                      AND COALESCE(tunnel, 'no') = 'no'""").fetchone()[0]
                results.append(("layer_without_structure", None if n else True,
                                f"{n} edge(s) with layer≠0 but no bridge/tunnel tag "
                                "(OSM tagging; drawn grade-separated, no 3D deck)"))
            else:
                results.append(("layer_without_structure", True,
                                "skipped (no layer/bridge/tunnel columns)"))

        for check, ok, detail in results:
            (logger.info if ok else logger.warning)(
                f"  validate[{self.mode}] {check}: "
                f"{'OK' if ok else 'WARN' if ok is None else 'FAIL'} — {detail}")

        failed = [(c, d) for c, ok, d in results if ok is False]
        if failed and self.cfg.fail_on_error:
            raise ValidationError(
                f"[{self.mode}] {len(failed)} validation check(s) failed: "
                + "; ".join(f"{c}: {d}" for c, d in failed))
        return results
