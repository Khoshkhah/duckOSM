"""OsmOverrides — apply local corrections for known OSM data errors to the `ways` table.

Some OSM source data is wrong or ambiguous (missing `oneway`, undercounted `lanes`, …). When a fix
upstream in OpenStreetMap isn't practical, this processor patches the affected ways locally, from a
single global rules file, so every rebuild reproduces the correction. See `docs/known_osm_issues.md`.

**Where it runs matters.** It executes AFTER `RoadFilter` (the `ways` table exists, with
`oneway` / `lanes_fwd` / `lanes_bwd`) and BEFORE `GraphBuilder` — because `oneway` is *topological*
(it decides whether a reverse-twin edge is created) and lane counts feed the forward/reverse edges.
`ways` is rebuilt per mode, so this runs once per mode.

Rules file (YAML), keyed by OSM `osm_id`; each is a silent no-op in areas that don't contain the way:

    overrides:
      - osm_id: 4392632        # oneway fix
        oneway: true
        note: "..."
      - osm_id: 507979055      # lane-count fix (per direction)
        lanes: 3
        note: "..."

Fields: `oneway` (bool) · `lanes` (int, per-direction — sets both directions) ·
`lanes_forward` / `lanes_backward` (int, for asymmetric roads) · `layer` (signed int — vertical
stacking level; overrides a wrong OSM `layer` tag, e.g. an at-grade way mis-tagged `layer=-1`).
"""
import logging
from pathlib import Path

import yaml

from duckosm.processors.base import BaseProcessor

logger = logging.getLogger("duckosm.osm_overrides")

_BOOL = {True: "TRUE", False: "FALSE"}


class OsmOverrides(BaseProcessor):
    """Patch the `ways` table with local OSM corrections from a global rules file."""

    def __init__(self, con, overrides_path):
        super().__init__(con)
        self.path = Path(overrides_path) if overrides_path else None

    def run(self) -> int:
        """Apply matching overrides to `ways`; return how many were applied (ways present here)."""
        if not self.path or not self.path.exists():
            return 0
        rules = (yaml.safe_load(self.path.read_text()) or {}).get("overrides") or []
        applied = 0
        for r in rules:
            osm_id = r.get("osm_id")
            if osm_id is None:
                continue
            sets = self._assignments(r)
            if not sets:
                continue
            # Only counts as "applied" when the way is actually in this area/mode's ways table —
            # a global rule legitimately matches nothing in most areas.
            if not self.fetchone(f"SELECT count(*) FROM ways WHERE osm_id = {int(osm_id)}")[0]:
                continue
            self.execute(f"UPDATE ways SET {', '.join(sets)} WHERE osm_id = {int(osm_id)}")
            applied += 1
            note = f"  ({r['note']})" if r.get("note") else ""
            logger.info(f"  override osm_id={int(osm_id)}: {', '.join(sets)}{note}")
        if applied:
            logger.info(f"  applied {applied}/{len(rules)} OSM override(s) to `ways`")
        return applied

    @staticmethod
    def _assignments(rule) -> list[str]:
        """Build the SET clauses for one rule (values are cast to int/bool — injection-safe)."""
        sets: list[str] = []
        if "oneway" in rule and rule["oneway"] is not None:
            sets.append(f"oneway = {_BOOL[bool(rule['oneway'])]}")
        if rule.get("lanes") is not None:                      # per-direction, both ways
            n = int(rule["lanes"]); sets += [f"lanes_fwd = {n}", f"lanes_bwd = {n}"]
        if rule.get("lanes_forward") is not None:
            sets.append(f"lanes_fwd = {int(rule['lanes_forward'])}")
        if rule.get("lanes_backward") is not None:
            sets.append(f"lanes_bwd = {int(rule['lanes_backward'])}")
        if rule.get("layer") is not None:                      # vertical stacking level (signed int)
            sets.append(f"layer = '{int(rule['layer'])}'")     # `ways.layer` is the raw tag string
        return sets
