"""
railML 2.4 **infrastructure** export — a *new* rail-network extraction from raw OSM (duckOSM has no
rail mode of its own), emitting the open format that feeds OpenTrack / RailSys / FBS / Viriato.
See docs/railml_export.md.

Pipeline: pull ``railway`` ways from ``raw.ways``, split them at shared/switch nodes into **tracks**,
wire the **topology** (connections / buffer stops / open ends at shared nodes), and attach
**switches**, **signals**, and **OCPs** (stations). Infrastructure only — no timetable/rollingstock.

    from duckosm import to_railml
    to_railml("sodermalm_pbf.duckdb", "sodermalm.railml.xml")
"""
import logging
import math
from collections import Counter, defaultdict
from pathlib import Path
from xml.sax.saxutils import quoteattr

logger = logging.getLogger("duckosm")

_RAIL = ("rail", "light_rail", "subway", "tram", "narrow_gauge", "funicular")
_NS = "https://www.railml.org/schemas/2013"


def _mlen(pts):
    """[(lon, lat)] → (cumulative metres per vertex, total metres) via equirectangular approx."""
    cum = [0.0]
    for (x0, y0), (x1, y1) in zip(pts, pts[1:]):
        kx = math.cos(math.radians((y0 + y1) / 2))
        cum.append(cum[-1] + math.hypot((x1 - x0) * kx, y1 - y0) * 111320)
    return cum, cum[-1]


def to_railml(source, out_path, rail_types=_RAIL):
    """Write a railML 2.4 infrastructure file from a built duckOSM db's ``raw`` schema. Returns
    ``{"tracks": n, "switches": n, "signals": n, "ocps": n}``."""
    import duckdb

    con = duckdb.connect(str(source), read_only=True)
    con.execute("INSTALL spatial; LOAD spatial;")
    if con.execute("SELECT count(*) FROM duckdb_tables() WHERE schema_name='raw' AND table_name='ways'").fetchone()[0] == 0:
        con.close()
        raise ValueError(f"no 'raw.ways' in {source} — need a built duckOSM db (with the raw OSM schema)")
    types = ",".join(f"'{t}'" for t in rail_types)
    ways = con.execute(f"SELECT osm_id, tags['railway'], refs FROM raw.ways "
                       f"WHERE tags['railway'] IN ({types}) AND refs IS NOT NULL").fetchall()
    nodes = {}   # node_id -> (lon, lat, railway_tag)
    for nid, lon, lat, rw in con.execute(
            f"SELECT osm_id, lon, lat, tags['railway'] FROM raw.nodes WHERE osm_id IN "
            f"(SELECT unnest(refs) FROM raw.ways WHERE tags['railway'] IN ({types}))").fetchall():
        nodes[nid] = (lon, lat, rw)
    ocps = con.execute("SELECT osm_id, lon, lat, tags['name'] FROM raw.nodes "
                       "WHERE tags['railway'] IN ('station','halt')").fetchall()
    con.close()

    # split each way at shared (junction) or switch nodes → track segments
    occ = Counter(r for _, _, refs in ways for r in refs)
    is_split = lambda n: occ[n] >= 2 or (nodes.get(n) and nodes[n][2] == "switch")
    tracks = []   # (id, kind, [node ids], pts, cum, total)
    for oid, rw, refs in ways:
        refs = [r for r in refs if r in nodes]
        if len(refs) < 2:
            continue
        segs, cur = [], [refs[0]]
        for r in refs[1:]:
            cur.append(r)
            if is_split(r) and r != refs[-1]:
                segs.append(cur); cur = [r]
        segs.append(cur)
        for s in segs:
            if len(s) < 2:
                continue
            pts = [(nodes[n][0], nodes[n][1]) for n in s]
            cum, total = _mlen(pts)
            if total <= 0:
                continue
            tracks.append((f"tr_{oid}_{s[0]}_{s[-1]}", rw, s, pts, cum, total))

    # topology: gather the track-ends meeting at each node, then wire connections
    node_ends = defaultdict(list)   # node -> [(track_idx, 'begin'|'end')]
    for i, (tid, rw, s, pts, cum, total) in enumerate(tracks):
        node_ends[s[0]].append((i, "begin"))
        node_ends[s[-1]].append((i, "end"))
    end_xml, switch_nodes = {}, set()
    cn = 0
    for node, ends in node_ends.items():
        rw = nodes.get(node, (None, None, None))[2]
        if len(ends) == 1:
            end_xml[ends[0]] = (f'<bufferStop id="bs_{node}"/>' if rw == "buffer_stop"
                                else f'<openEnd id="oe_{node}"/>')
            continue
        if len(ends) >= 3 or rw == "switch":
            switch_nodes.add(node)
        i = 0
        while i + 1 < len(ends):
            a, b = ends[i], ends[i + 1]
            end_xml[a] = f'<connection id="cn{cn}" ref="cn{cn+1}"/>'
            end_xml[b] = f'<connection id="cn{cn+1}" ref="cn{cn}"/>'
            cn += 2; i += 2
        if i < len(ends):                        # leftover odd end at a junction
            end_xml[ends[i]] = f'<openEnd id="oe_{node}_{ends[i][0]}"/>'

    # switches / signals attached to their owning track (first incident track for a switch node)
    switch_of = defaultdict(list)   # track_idx -> [(node, pos)]
    seen_switch = set()
    signal_of = defaultdict(list)   # track_idx -> [(node, pos)]
    n_signals = 0
    for i, (tid, rw, s, pts, cum, total) in enumerate(tracks):
        for k, n in enumerate(s):
            tag = nodes.get(n, (None, None, None))[2]
            if tag == "signal":
                signal_of[i].append((n, cum[k])); n_signals += 1
            if n in switch_nodes and n not in seen_switch:
                switch_of[i].append((n, cum[k])); seen_switch.add(n)

    # ---- emit railML 2.4 ----
    o = ['<?xml version="1.0" encoding="UTF-8"?>',
         f'<railml xmlns="{_NS}" version="2.4">', '\t<infrastructure id="is_1">']
    o.append('\t\t<operationControlPoints>')
    for nid, lon, lat, name in ocps:
        o.append(f'\t\t\t<ocp id="ocp_{nid}" name={quoteattr(name or f"ocp_{nid}")}>')
        o.append(f'\t\t\t\t<geoCoord coord="{lon:.6f} {lat:.6f}" epsgCode="4326"/>')
        o.append('\t\t\t</ocp>')
    o.append('\t\t</operationControlPoints>')
    o.append('\t\t<tracks>')
    for i, (tid, rw, s, pts, cum, total) in enumerate(tracks):
        kind = "mainTrack" if rw in ("rail", "subway", "light_rail") else "secondaryTrack"
        begin_x = end_xml.get((i, "begin")) or f'<openEnd id="oeb_{i}"/>'
        end_x = end_xml.get((i, "end")) or f'<openEnd id="oee_{i}"/>'
        o.append(f'\t\t\t<track id="{tid}" type="{kind}">')
        o.append('\t\t\t\t<trackTopology>')
        o.append(f'\t\t\t\t\t<trackBegin id="tb_{i}" pos="0.0">{begin_x}</trackBegin>')
        o.append(f'\t\t\t\t\t<trackEnd id="te_{i}" pos="{total:.3f}">{end_x}</trackEnd>')
        o.append('\t\t\t\t</trackTopology>')
        if switch_of.get(i):
            o.append('\t\t\t\t<trackElements>')
            o.append('\t\t\t\t\t<switches>')
            for node, pos in switch_of[i]:
                o.append(f'\t\t\t\t\t\t<switch id="sw_{node}" pos="{pos:.3f}" name="sw_{node}"/>')
            o.append('\t\t\t\t\t</switches>')
            o.append('\t\t\t\t</trackElements>')
        if signal_of.get(i):
            o.append('\t\t\t\t<ocsElements>')
            o.append('\t\t\t\t\t<signals>')
            for node, pos in signal_of[i]:
                o.append(f'\t\t\t\t\t\t<signal id="sig_{node}" pos="{pos:.3f}" dir="up"/>')
            o.append('\t\t\t\t\t</signals>')
            o.append('\t\t\t\t</ocsElements>')
        o.append('\t\t\t</track>')
    o.append('\t\t</tracks>')
    o.append('\t</infrastructure>')
    o.append('</railml>\n')

    out_path = Path(out_path)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text("\n".join(o), encoding="utf-8")
    counts = {"tracks": len(tracks), "switches": len(seen_switch), "signals": n_signals, "ocps": len(ocps)}
    logger.info(f"railML 2.4: {counts['tracks']:,} tracks, {counts['switches']} switches, "
                f"{counts['signals']} signals, {counts['ocps']} OCPs -> {out_path}")
    return counts
