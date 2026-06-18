"""
Build-time invariant validation.

Runs after a build/clip (driven by the `validation:` config block) and raises
ValidationError when `fail_on_error` and a check fails — the "fail loudly" safety net
that the per-way filter + component clean-up are supposed to satisfy.

Checks (per mode):
  - single_component   : the graph is one dominant weakly-connected component
  - no_stranded_named  : no NAMED edge sits outside the largest component
  - edge_id_stable     : reserved (needs a fixture/parent baseline) — reported as skipped
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

        for check, ok, detail in results:
            (logger.info if ok else logger.warning)(
                f"  validate[{self.mode}] {check}: {'OK' if ok else 'FAIL'} — {detail}")

        failed = [(c, d) for c, ok, d in results if not ok]
        if failed and self.cfg.fail_on_error:
            raise ValidationError(
                f"[{self.mode}] {len(failed)} validation check(s) failed: "
                + "; ".join(f"{c}: {d}" for c, d in failed))
        return results
