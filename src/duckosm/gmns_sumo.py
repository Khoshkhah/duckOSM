"""Which lane goes to which, and the path through each junction, from SUMO netconvert (docs/design/gmns_lane_profile.md, step 3).

``lanes_from_sumo(con, sch)`` replaces the GMNS lane rules for the driving network: it writes the network to SUMO (``to_sumo``: the
lanes of ``driving.lane_profile``, the legal turns of ``edge_graph``, the bus-only edges and the buses' turns), lets netconvert
build it, and reads back

- each movement's lane ranges from netconvert's connections: one row per run of lanes that go on side by side (the first keeps the
  movement's id, a further run ``<mvmt_id>-2`` ...; a bike lane's ``<mvmt_id>-b<lane>``, ``allowed_uses`` bike);
- ``lane_connector``: one row per lane pair, the path through the junction (SUMO's internal lanes);
- ``lane.geom``: SUMO's lane line, which stops where the junction begins; ``lane.geom_full`` keeps duckOSM's line to the node.

What it cannot map is written to ``<sch>.lane_check`` (``kind``, ``id``, ``detail``), never filled in: a link whose lane count SUMO
built differently, a movement netconvert gave no lane connection (it leaves out some U-turns).
"""
import logging
import tempfile
from collections import defaultdict

logger = logging.getLogger("duckosm")


def _read_net(net):
    """``(to_ll, lanes, conns, nxt)`` of a ``.net.xml``: ``to_ll(shape)`` -> [(lon, lat)]; ``lanes`` {lane id: (shape, width)} of every
    lane (normal and internal); ``conns`` [(from, fromLane, to, toLane, via)] out of the normal edges; ``nxt`` {(internal edge, lane): via}
    for an internal lane that goes on into a further internal lane."""
    import xml.etree.ElementTree as ET

    from pyproj import Transformer
    ox = oy = 0.0
    tr = None
    lanes, conns, nxt = {}, [], {}
    for _, el in ET.iterparse(net):
        if el.tag == "location":
            ox, oy = (float(v) for v in el.get("netOffset").split(","))
            tr = Transformer.from_crs(el.get("projParameter"), "EPSG:4326", always_xy=True)
        elif el.tag == "lane":
            pts = [tuple(float(v) for v in p.split(",")[:2]) for p in (el.get("shape") or "").split()]
            lanes[el.get("id")] = (pts, float(el.get("width") or 3.2))
        elif el.tag == "connection":
            f = el.get("from")
            if f.startswith(":"):
                if el.get("via"):
                    nxt[(f, el.get("fromLane"))] = el.get("via")
            else:
                conns.append((f, int(el.get("fromLane")), el.get("to"), int(el.get("toLane")), el.get("via")))
    if tr is None:
        raise ValueError(f"{net}: no <location> (no projection to read the shapes back)")

    def to_ll(shape):
        return [tr.transform(x - ox, y - oy) for x, y in shape]
    return to_ll, lanes, conns, nxt


def _path(via, lanes, nxt):
    """The shape through the junction: the internal lane ``via`` and the internal lanes it goes on into."""
    pts, seen = [], set()
    while via and via not in seen:
        seen.add(via)
        shape = lanes.get(via, ([], 0))[0]
        pts += shape[1:] if pts and shape and shape[0] == pts[-1] else shape
        edge, _, k = via.rpartition("_")
        via = nxt.get((edge, k))
    return pts


def _runs(pairs):
    """Lane pairs ``[(a, b)]`` -> runs that go on side by side (a and b both one more each step), each ``[(a, b), ...]``."""
    out = []
    for a, b in sorted(set(pairs)):
        if out and out[-1][-1] == (a - 1, b - 1):
            out[-1].append((a, b))
        else:
            out.append([(a, b)])
    return out


def lanes_from_sumo(con, sch, source="s", netconvert_bin=None, work_dir=None):
    """See the module docstring. ``con``: the GMNS connection, the duckOSM db attached as ``source``; ``sch``: ``gmns_driving``, its
    ``lane`` and ``movement`` tables built. Returns a summary dict."""
    from duckosm.sumo import to_sumo
    cur = con.cursor()
    cur.execute(f"USE {source}")
    with tempfile.TemporaryDirectory() as tmp:
        net = to_sumo(cur, work_dir or tmp, mode="driving", net_name="gmns", netconvert_bin=netconvert_bin, bus_edges=True)["net"]
        to_ll, lanes, conns, nxt = _read_net(net)
    cur.close()
    check = []
    con.execute(f"CREATE OR REPLACE TABLE {sch}.lane_check(kind VARCHAR, id VARCHAR, detail VARCHAR)")

    # our lanes of each link, left to right, and SUMO's (index from the right)
    ours = defaultdict(list)
    use_of, lid_of, width_of = {}, {}, {}
    for lk, num, use, lid, w in con.execute(f"SELECT link_id, lane_num, allowed_uses, lane_id, width FROM {sch}.lane ORDER BY link_id, lane_num").fetchall():
        ours[lk].append(num)
        use_of[(lk, num)], lid_of[(lk, num)], width_of[lid] = use, lid, w
    n_sumo = defaultdict(int)
    for lane_id in lanes:
        if not lane_id.startswith(":"):
            n_sumo[int(lane_id.rpartition("_")[0])] += 1
    bad = {lk for lk in ours if n_sumo.get(lk) != len(ours[lk])}
    for lk in sorted(bad):
        check.append(("lane count", str(lk), f"duckOSM {len(ours[lk])} lanes, SUMO {n_sumo.get(lk, 0)}"))

    def num(edge, idx):                            # SUMO's lane index (from the right) -> our lane number
        ls = ours[edge]
        return ls[len(ls) - 1 - idx]

    def sumo_lane(edge, n):                        # our lane number -> SUMO's lane id
        return f"{edge}_{len(ours[edge]) - 1 - ours[edge].index(n)}"

    # the movements' lane ranges from the connections
    by_pair = defaultdict(list)
    for f, fl, t, tl, via in conns:
        f, t = int(f), int(t)
        if f in bad or t in bad or f not in ours or t not in ours:
            continue
        by_pair[(f, t)].append((num(f, fl), num(t, tl), via))
    mv = con.execute(f"SELECT * FROM {sch}.movement").df()
    cols = list(mv.columns)
    rows, connectors, missing = [], [], 0
    for r in mv.to_dict("records"):
        got = by_pair.get((r["ib_link_id"], r["ob_link_id"]), [])
        if not got:
            rows.append(r)
            missing += 1
            check.append(("no lane connection", r["mvmt_id"], f"{r['type']} {r['ib_link_id']} -> {r['ob_link_id']}: netconvert built none"))
            continue
        bike = [(a, b) for a, b, _ in got if use_of.get((r["ib_link_id"], a)) == "bike" or use_of.get((r["ob_link_id"], b)) == "bike"]
        motor = [(a, b) for a, b, _ in got if (a, b) not in bike]
        mid_of = {}                                    # lane pair -> the movement row it belongs to
        for k, run in enumerate(_runs(motor)):
            m = r["mvmt_id"] if k == 0 else f"{r['mvmt_id']}-{k + 1}"
            rows.append({**r, "mvmt_id": m, "start_ib_lane": run[0][0], "end_ib_lane": run[-1][0], "start_ob_lane": run[0][1], "end_ob_lane": run[-1][1]})
            mid_of.update({p: m for p in run})
        if not motor:                                  # bike lanes only: the movement row stays, without car lanes
            rows.append({**r, "start_ib_lane": None, "end_ib_lane": None, "start_ob_lane": None, "end_ob_lane": None})
        for a, b in bike:
            m = f"{r['mvmt_id']}-b{a}"
            rows.append({**r, "mvmt_id": m, "start_ib_lane": a, "end_ib_lane": a, "start_ob_lane": b, "end_ob_lane": b, "allowed_uses": "bike"})
            mid_of[(a, b)] = m
        for a, b, via in got:
            la, lb = lid_of[(r["ib_link_id"], a)], lid_of[(r["ob_link_id"], b)]
            shape = _path(via, lanes, nxt)
            if len(shape) < 2:                         # no internal lane (a straight join): from the lane's end to the next one's start
                ea, eb = lanes.get(sumo_lane(r["ib_link_id"], a), ([], 0))[0], lanes.get(sumo_lane(r["ob_link_id"], b), ([], 0))[0]
                shape = [ea[-1], eb[0]] if ea and eb else []
            if len(shape) >= 2:
                connectors.append((f"{la}>{lb}", mid_of[(a, b)], la, lb, min(width_of.get(la) or 3.25, width_of.get(lb) or 3.25),
                                   "LINESTRING (" + ", ".join(f"{x:.8f} {y:.8f}" for x, y in to_ll(shape)) + ")"))
    import pandas as pd
    out = pd.DataFrame(rows, columns=cols)
    con.execute(f"DELETE FROM {sch}.movement")
    con.register("_mv_sumo", out)
    con.execute(f"INSERT INTO {sch}.movement SELECT * FROM _mv_sumo")
    con.unregister("_mv_sumo")

    # the lanes: SUMO's line (to the junction) as geom, duckOSM's line to the node as geom_full
    con.execute(f"ALTER TABLE {sch}.lane ADD COLUMN IF NOT EXISTS geom_full GEOMETRY")
    con.execute(f"UPDATE {sch}.lane SET geom_full = geom")
    cut = []
    for lk, nums in ours.items():
        if lk in bad:
            continue
        for n in nums:
            shape = lanes.get(sumo_lane(lk, n), ([], 0))[0]
            if len(shape) >= 2:
                cut.append((lid_of[(lk, n)], "LINESTRING (" + ", ".join(f"{x:.8f} {y:.8f}" for x, y in to_ll(shape)) + ")"))
    con.register("_cut_sumo", pd.DataFrame(cut, columns=["lane_id", "wkt"]))
    con.execute(f"UPDATE {sch}.lane l SET geom = ST_GeomFromText(c.wkt) FROM _cut_sumo c WHERE c.lane_id = l.lane_id")
    con.unregister("_cut_sumo")
    con.execute(f"""CREATE OR REPLACE TABLE {sch}.lane_connector(connector_id VARCHAR, mvmt_id VARCHAR, from_lane_id VARCHAR,
                    to_lane_id VARCHAR, width DOUBLE, geom GEOMETRY)""")
    if connectors:
        con.register("_con_sumo", pd.DataFrame(connectors, columns=["connector_id", "mvmt_id", "from_lane_id", "to_lane_id", "width", "wkt"]))
        con.execute(f"INSERT INTO {sch}.lane_connector SELECT connector_id, mvmt_id, from_lane_id, to_lane_id, width, ST_GeomFromText(wkt) FROM _con_sumo")
        con.unregister("_con_sumo")
    if check:
        con.executemany(f"INSERT INTO {sch}.lane_check VALUES (?, ?, ?)", check)
    summary = {"movements": len(mv), "rows": len(out), "connectors": len(connectors), "no_connection": missing, "lane_count_mismatch": len(bad)}
    logger.info(f"GMNS[driving] lanes from SUMO: {summary}; anything not mapped is in {sch}.lane_check")
    return summary
