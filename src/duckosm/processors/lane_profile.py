"""LaneProfile — every directed driving edge's lanes, decided once (docs/design/gmns_lane_profile.md).

``driving.lane_profile``: one row per lane of each edge of ``driving.edges`` and of each bus-only edge of
``driving.private_edges`` (``access = 'bus'``), left to right as GMNS numbers them: a lane left of the left-most through lane
-1, the motor lanes 1..n, a lane right of them n+1. ``use`` (``auto`` / ``bus`` / ``bike`` / ``bus,bike``), ``width_m``,
``turn`` (its ``turn:lanes`` entry, only on the edge where its way ends in its direction: OSM's arrows describe the lanes there,
not at a side street part way along), ``source`` (the tag or rule that put the lane there with its use) and ``width_source``.

The motor lane count is the edge's ``lanes`` (RoadFilter's reading of ``lanes`` / ``lanes:forward`` / ``lanes:backward``, the
contraflow bus lane taken off, ``osm_overrides`` applied), or ``turn:lanes``' count when it has more entries, or, for an
untagged one-way edge, the count of the road it plainly continues (``gmns._inherit_lanes``). A count, a use or a width that
no tag gives is a default and says so (``source`` / ``width_source`` ``default``): nothing is filled in silently.
Bike lanes are not counted in ``lanes`` (OSM, and SUMO's OSM import): ``cycleway:right=lane`` adds one right of the motor
lanes, ``cycleway:left=lane`` on a one-way edge (with the traffic) one left of them.
"""
import logging

from duckosm.processors.base import BaseProcessor

logger = logging.getLogger("duckosm.lane_profile")

LANE_W = 3.25                       # metres: a motor lane without a width tag (gmns._DEFAULT_LANE_W)
LANE_W_BY_CLASS = {"service": 2.5, "residential": 3.0, "living_street": 3.0, "unclassified": 3.0}
BIKE_W = 1.5                        # an on-road bike lane without a width tag
YES = ("designated", "yes")


def _split(s):
    return s.split("|") if s else []


def _num(s):
    try:
        return float(str(s).strip())
    except (TypeError, ValueError):
        return None


def _pick(tags, base, reverse):
    """A per-direction tag: ``base:backward`` on the reverse edge; ``base:forward``, else ``base``, on the forward one."""
    return tags.get(base + ":backward") if reverse else (tags.get(base + ":forward") or tags.get(base))


def lanes_of(tags, *, reverse=False, oneway=False, highway=None, n_motor=1, n_source="default", bus_only=False, shared_bike=False, way_end=True):
    """The lanes of one directed edge, left to right: ``[{lane_num, use, width_m, turn, source, width_source}]``.

    ``tags``: its OSM way's tags; ``reverse``: the edge runs against the way's drawing; ``oneway``: the driving network's one-way;
    ``n_motor`` / ``n_source``: its motor lane count and where that came from; ``bus_only``: a bus-only edge (a contraflow bus
    lane, a bus-only road: one lane), ``shared_bike``: bikes share it; ``way_end``: the edge ends where its way ends (in its direction),
    the only edge whose lanes carry the arrows (the lane count from ``turn:lanes`` holds on every edge of the way)."""
    tags = tags or {}
    cls = (highway or "").split(";")[0].removesuffix("_link")
    if bus_only:
        w = _num((_split(_pick(tags, "width:lanes", reverse)) or [None])[0])
        return [{"lane_num": 1, "use": "bus,bike" if shared_bike else "bus", "width_m": w or LANE_W, "turn": None,
                 "source": "access=bus", "width_source": "width:lanes" if w else "default"}]
    turns, widths = _split(_pick(tags, "turn:lanes", reverse)), _split(_pick(tags, "width:lanes", reverse))
    n = max(len(turns), int(n_motor or 1), 1)
    src = "turn:lanes" if len(turns) > int(n_motor or 1) else n_source
    bikes, psvs, buses = (_split(_pick(tags, k, reverse)) for k in ("bicycle:lanes", "psv:lanes", "bus:lanes"))
    out = []
    for i in range(n):
        use, why = "auto", src
        if i < len(bikes) and bikes[i] in YES:
            use, why = "bike", "bicycle:lanes"
        elif (i < len(psvs) and psvs[i] in YES) or (i < len(buses) and buses[i] in YES):
            use, why = "bus", "psv:lanes" if i < len(psvs) and psvs[i] in YES else "bus:lanes"
        w = _num(widths[i]) if i < len(widths) else None
        out.append({"lane_num": i + 1, "use": use, "width_m": w or LANE_W_BY_CLASS.get(cls, LANE_W),
                    "turn": turns[i] if way_end and i < len(turns) and turns[i] not in ("", "none") else None,
                    "source": why, "width_source": "width:lanes" if w else "default"})
    # bus lanes counted (lanes:bus) or by side (busway:<side>=lane) when no per-lane tag says which: the right-most motor lanes
    if not any(x["use"] == "bus" for x in out):
        side = "left" if reverse else "right"                      # this direction's right side, seen along the way
        k = int(_num(_pick(tags, "lanes:bus", reverse)) or _num(_pick(tags, "lanes:psv", reverse)) or 0)
        why = "lanes:bus" if k else None
        for key in (f"busway:{side}", "busway:both", "busway"):
            if not k and tags.get(key) == "lane":
                k, why = 1, key
        for x in out[len(out) - k:] if 0 < k < len(out) else []:      # ponytail: a bus count >= the lanes leaves them all motor (no car lane left); listed by the checks later
            x["use"], x["source"] = "bus", why
    # a bus lane bikes share (cycleway:<this side>=share_busway): its right-most bus lane
    side = "left" if reverse else "right"
    if "share_busway" in (tags.get(f"cycleway:{side}"), tags.get("cycleway:both"), tags.get("cycleway")):
        for x in reversed(out):
            if x["use"] == "bus":
                x["use"], x["source"] = "bus,bike", f"{x['source']} + cycleway share_busway"
                break
    # bike lanes beside the motor lanes (not counted in lanes): right of travel; left of a one-way's travel (with the traffic)
    if "designated" not in bikes:
        for key in (f"cycleway:{side}", "cycleway:both", "cycleway"):
            if tags.get(key) == "lane":
                w = _num(tags.get(f"{key}:width")) or _num(tags.get("cycleway:width"))
                out.append({"lane_num": n + 1, "use": "bike", "width_m": w or BIKE_W, "turn": None, "source": key,
                            "width_source": f"{key}:width" if w else "default"})
                break
        if oneway and not reverse and tags.get("oneway:bicycle") != "no":
            for key in ("cycleway:left", "cycleway:both"):
                if tags.get(key) == "lane" and tags.get(f"{key}:oneway") not in ("-1", "no"):
                    w = _num(tags.get(f"{key}:width")) or _num(tags.get("cycleway:width"))
                    out.insert(0, {"lane_num": -1, "use": "bike", "width_m": w or BIKE_W, "turn": None, "source": key,
                                   "width_source": f"{key}:width" if w else "default"})
                    break
    return out


class LaneProfile(BaseProcessor):
    """Write ``driving.lane_profile`` (see the module docstring). ``overrides``: the OSM ways whose lane count an
    ``osm_overrides.yaml`` rule sets (their ``source`` is ``override``)."""

    def __init__(self, con, overrides=()):
        super().__init__(con)
        self.overrides = set(overrides)

    def run(self) -> int:
        from duckosm.gmns import _inherit_lanes          # the one inheritance rule (moves here when GMNS reads this table)

        cols = {c for (c,) in self.fetchall("SELECT column_name FROM duckdb_columns() WHERE schema_name = 'driving' AND table_name = 'edges'")}
        rev = "e.is_reverse" if "is_reverse" in cols else "false"
        one = "e.oneway" if "oneway" in cols else "false"
        rows = self.fetchall(f"""SELECT e.edge_id, e.osm_id, {rev}, {one}, e.highway, e.lanes, e.source, e.target, w.tags,
                                        w.refs[1], w.refs[-1]
                                 FROM driving.edges e LEFT JOIN raw.ways w ON w.osm_id = abs(e.osm_id)""")
        lane_tags = ("lanes", "lanes:forward", "lanes:backward", "turn:lanes", "width:lanes")
        info = [dict(edge_id=r[0], source=r[6], target=r[7], cls=(r[4] or "").split(";")[0].removesuffix("_link"), lanes=r[5],
                     tagged=bool(r[8]) and any(k in r[8] for k in lane_tags) or abs(r[1] or 0) in self.overrides, oneway=bool(r[3]))
                for r in rows]
        inherited = _inherit_lanes(info)
        out = []
        for (eid, osm, reverse, oneway, hw, lanes, _s, target, tags, first, last), e in zip(rows, info, strict=True):
            tags = tags or {}
            if abs(osm or 0) in self.overrides:
                src = "override"
            elif eid in inherited:
                src = "inherited"
            elif tags.get("lanes:backward" if reverse else "lanes:forward"):
                src = "lanes:backward" if reverse else "lanes:forward"
            elif tags.get("lanes"):
                src = "lanes"
            else:
                src = "default"
            for x in lanes_of(tags, reverse=bool(reverse), oneway=bool(oneway), highway=hw, n_motor=inherited.get(eid, lanes), n_source=src,
                              way_end=target == (first if reverse else last)):
                out.append((eid, x["lane_num"], x["use"], x["width_m"], x["turn"], x["source"], x["width_source"]))
        if "private_edges" in {t for (t,) in self.fetchall("SELECT table_name FROM duckdb_tables() WHERE schema_name = 'driving'")}:
            cycling = {e for (e,) in self.fetchall("SELECT edge_id FROM cycling.edges")} if self.fetchone(
                "SELECT count(*) FROM duckdb_tables() WHERE schema_name = 'cycling' AND table_name = 'edges'")[0] else set()
            for eid, reverse, hw, tags in self.fetchall("""SELECT p.edge_id, p.is_reverse, p.highway, w.tags FROM driving.private_edges p
                                                           LEFT JOIN raw.ways w ON w.osm_id = abs(p.osm_id) WHERE p.access = 'bus'"""):
                tags = tags or {}
                # bikes share a bus-only edge: oneway:bicycle=no, a share_busway, or the edge is in the cycling network (as GMNS reads it)
                shared = tags.get("oneway:bicycle") == "no" or eid in cycling or "share_busway" in (tags.get("cycleway"), tags.get("cycleway:left"), tags.get("cycleway:right"))
                for x in lanes_of(tags, reverse=bool(reverse), highway=hw, bus_only=True, shared_bike=shared):
                    out.append((eid, x["lane_num"], x["use"], x["width_m"], x["turn"], x["source"], x["width_source"]))
        self.execute("""CREATE OR REPLACE TABLE driving.lane_profile (edge_id BIGINT, lane_num INTEGER, use VARCHAR, width_m DOUBLE,
                        turn VARCHAR, source VARCHAR, width_source VARCHAR)""")
        if out:
            self.con.executemany("INSERT INTO driving.lane_profile VALUES (?, ?, ?, ?, ?, ?, ?)", out)
        counts = dict(self.fetchall("SELECT source, count(*) FROM driving.lane_profile GROUP BY 1"))
        logger.info(f"lane profile: {len(out):,} lanes; by source: " + ", ".join(f"{k} {v:,}" for k, v in sorted(counts.items(), key=lambda kv: -kv[1])))
        return len(out)
