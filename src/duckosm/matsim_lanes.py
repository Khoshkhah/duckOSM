"""
MATSim turn **lanes** (``lanes.xml``, laneDefinitions_v2.0) and traffic **signals**
(``signalSystems`` / ``signalGroups`` / ``signalControl`` v2.0) from a GMNS db — the detailed-
intersection companion to the [network export](matsim.py). See https://khoshkhah.github.io/duckOSM/exports/matsim/#lanes-and-signals.

Source is a GMNS db because ``gmns_<mode>.movement`` already is the lane→turn→downstream model
(``ib_link_id`` → ``ob_link_id``, ``turn:lanes`` in ``start_ib_lane``/``end_ib_lane``, restrictions
honoured), and ``node.ctrl_type='signal'`` marks the signalised nodes. ``link_id`` **is** ``edge_id``,
so these files line up with a ``network.xml`` from the same build.

    from duckosm import to_matsim_lanes
    to_matsim_lanes("sodermalm_pbf_gmns.duckdb", out_dir=".")     # lanes.xml + the 3 signals files
"""
import logging
from collections import defaultdict
from pathlib import Path

logger = logging.getLogger("duckosm")

_NS = "http://www.matsim.org/files/dtd"
_XSI = "http://www.w3.org/2001/XMLSchema-instance"


def _root(tag, xsd):
    return (f'<{tag} xmlns="{_NS}" xmlns:xsi="{_XSI}" '
            f'xsi:schemaLocation="{_NS} {_NS}/{xsd}">')


def _orient(codes):
    """N–S vs E–W phase from movement codes (NB/SB → NS approach, EB/WB → EW)."""
    ns = sum(1 for c in codes if c and c[0] in "NS")
    ew = sum(1 for c in codes if c and c[0] in "EW")
    return "NS" if ns >= ew else "EW"


def _lane_groups(movements, n_lanes):
    """→ [(represented_lanes, [ob_links])]: split by turn:lanes when tagged, else one all-lanes group."""
    if any(sib is not None for _, _, sib, _, _ in movements):
        groups = defaultdict(set)
        for _, ob, sib, eib, _ in movements:
            lo = int(sib or 1); hi = int(eib or sib or 1)
            groups[(lo, hi)].add(ob)
        return [(max(1, hi - lo + 1), sorted(obs)) for (lo, hi), obs in sorted(groups.items())]
    return [(n_lanes, sorted({ob for _, ob, _, _, _ in movements}))]


def _write_lanes(out_dir, by_ib, links):
    o = ['<?xml version="1.0" encoding="UTF-8"?>', _root("laneDefinitions", "laneDefinitions_v2.0.xsd")]
    n = 0
    for ib, movements in by_ib.items():
        n_lanes, per_cap, length = links.get(ib, (1, 800.0, 50.0))
        o.append(f'\t<lanesToLinkAssignment linkIdRef="{ib}">')
        for k, (rep, obs) in enumerate(_lane_groups(movements, n_lanes), 1):
            o.append(f'\t\t<lane id="{ib}.{k}">')
            o.append('\t\t\t<leadsTo>')
            o += [f'\t\t\t\t<toLink refId="{ob}"/>' for ob in obs]
            o.append('\t\t\t</leadsTo>')
            o.append(f'\t\t\t<representedLanes number="{rep}"/>')
            o.append(f'\t\t\t<capacity vehiclesPerHour="{per_cap * rep:.1f}"/>')
            o.append(f'\t\t\t<startsAt meterFromLinkEnd="{min(length, 45.0):.1f}"/>')
            o.append('\t\t\t<alignment>0</alignment>')
            o.append('\t\t</lane>')
        o.append('\t</lanesToLinkAssignment>')
        n += 1
    o.append('</laneDefinitions>\n')
    (Path(out_dir) / "lanes.xml").write_text("\n".join(o), encoding="utf-8")
    return n


def _write_signals(out_dir, by_ib, sig_nodes, cycle_s):
    # per signalised node → its approaches (inbound link, its ob-links, N–S/E–W orientation)
    node_appr = defaultdict(list)
    for ib, movements in by_ib.items():
        node = movements[0][0]
        if node not in sig_nodes:
            continue
        obs = sorted({ob for _, ob, _, _, _ in movements})
        orient = _orient([c for _, _, _, _, c in movements])
        node_appr[node].append((ib, obs, orient))

    ss = ['<?xml version="1.0" encoding="UTF-8"?>', _root("signalSystems", "signalSystems_v2.0.xsd")]
    sg = ['<?xml version="1.0" encoding="UTF-8"?>', _root("signalGroups", "signalGroups_v2.0.xsd")]
    sc = ['<?xml version="1.0" encoding="UTF-8"?>', _root("signalControl", "signalControl_v2.0.xsd")]
    half, allred = cycle_s // 2, 3
    phase = {"NS": (0, half - allred), "EW": (half, cycle_s - allred)}   # default 2-phase fixed-time
    for node, appr in node_appr.items():
        ss.append(f'\t<signalSystem id="{node}">')
        ss.append('\t\t<signals>')
        sg.append(f'\t<signalSystem refId="{node}">')
        sc.append(f'\t<signalSystem refId="{node}">')
        sc.append('\t\t<signalSystemController>')
        sc.append('\t\t\t<controllerIdentifier>DefaultPlanbasedSignalSystemController</controllerIdentifier>')
        sc.append('\t\t\t<signalPlan id="1">')
        sc.append(f'\t\t\t\t<cycleTime sec="{cycle_s}"/>')
        sc.append('\t\t\t\t<offset sec="0"/>')
        for ib, obs, orient in appr:
            sid = f"{node}.{ib}"
            ss.append(f'\t\t\t<signal id="{sid}" linkIdRef="{ib}">')
            ss.append('\t\t\t\t<turningMoveRestrictions>')
            ss += [f'\t\t\t\t\t<toLink refId="{ob}"/>' for ob in obs]
            ss.append('\t\t\t\t</turningMoveRestrictions>')
            ss.append('\t\t\t</signal>')
            sg.append(f'\t\t<signalGroup id="{sid}">')
            sg.append(f'\t\t\t<signal refId="{sid}"/>')
            sg.append('\t\t</signalGroup>')
            on, drop = phase[orient]
            sc.append(f'\t\t\t\t<signalGroupSettings refId="{sid}">')
            sc.append(f'\t\t\t\t\t<onset sec="{on}"/>')
            sc.append(f'\t\t\t\t\t<dropping sec="{drop}"/>')
            sc.append('\t\t\t\t</signalGroupSettings>')
        ss.append('\t\t</signals>')
        ss.append('\t</signalSystem>')
        sg.append('\t</signalSystem>')
        sc.append('\t\t\t</signalPlan>')
        sc.append('\t\t</signalSystemController>')
        sc.append('\t</signalSystem>')
    ss.append('</signalSystems>\n'); sg.append('</signalGroups>\n'); sc.append('</signalControl>\n')
    for name, lines in (("signalSystems", ss), ("signalGroups", sg), ("signalControl", sc)):
        (Path(out_dir) / f"{name}.xml").write_text("\n".join(lines), encoding="utf-8")
    return len(node_appr)


def to_matsim_lanes(gmns_db, out_dir=".", mode="driving", signals=True, cycle_s=90):
    """Write MATSim ``lanes.xml`` (+ ``signalSystems``/``signalGroups``/``signalControl`` when
    ``signals``) from a GMNS db's ``gmns_<mode>`` movement/link/node tables. Returns counts."""
    import duckdb

    con = duckdb.connect(str(gmns_db), read_only=True)
    con.execute("INSTALL spatial; LOAD spatial;")
    g = f"gmns_{mode}"
    if con.execute("SELECT count(*) FROM duckdb_tables() WHERE schema_name=? AND table_name='movement'",
                   [g]).fetchone()[0] == 0:
        con.close()
        raise ValueError(f"no '{g}.movement' in {gmns_db} — build a GMNS db first (duckosm gmns)")

    links = {lid: (max(int(lanes or 1), 1), float(cap or 800), float(ln or 50)) for lid, lanes, cap, ln in
             con.execute(f"SELECT link_id, lanes, capacity, length FROM {g}.link").fetchall()}
    by_ib = defaultdict(list)
    for node, ib, ob, sib, eib, code in con.execute(
            f"SELECT node_id, ib_link_id, ob_link_id, start_ib_lane, end_ib_lane, mvmt_code "
            f"FROM {g}.movement WHERE ib_link_id IS NOT NULL AND ob_link_id IS NOT NULL").fetchall():
        by_ib[ib].append((node, ob, sib, eib, code))
    sig_nodes = {r[0] for r in con.execute(f"SELECT node_id FROM {g}.node WHERE ctrl_type='signal'").fetchall()}
    con.close()

    out_dir = Path(out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)
    result = {"lanes_links": _write_lanes(out_dir, by_ib, links)}
    if signals:
        result["signal_systems"] = _write_signals(out_dir, by_ib, sig_nodes, cycle_s)
    logger.info(f"MATSim lanes[{mode}]: {result['lanes_links']:,} lane assignments"
                + (f", {result['signal_systems']} signal systems (cycle {cycle_s}s)" if signals else "")
                + f" -> {out_dir}")
    return result
