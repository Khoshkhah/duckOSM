"""Compare the lanes duckOSM gives SUMO with the lanes SUMO's own OSM import makes (docs/design/gmns_lane_profile.md, decision 2).

``lane_diff(db, osm_file, work_dir)`` builds both SUMO networks of the same area: from the duckOSM database (``to_sumo``: the lanes
of ``driving.lane_profile``) and from the OSM file itself (``netconvert --osm-files``, its own tag reading). It returns one row per
OSM way and direction whose lanes differ, with the lane counts (car, bus, bike) of both, the profile's sources for that way and the
cause, so every difference has a name. Roads SUMO's import closes to cars and buses (its type map makes ``highway=service`` a
delivery road) are left out: there it has no lanes to compare.
"""
import collections
import os
import subprocess

CAUSES = ("override", "contraflow bus lane", "no lanes tag", "inherited", "turn:lanes count", "bike lane", "bus lane", "other")


def _kind(lane):
    a = set((lane.get("allow") or "").split())
    if a == {"bicycle"}:
        return "bike"
    if a and "passenger" not in a and "bus" in a:
        return "bus"
    if a and "passenger" not in a:
        return "other"
    return "car"


def _lanes(net, key):
    """``{(way, 'f' / 'r'): (car, bus, bike)}`` of a ``.net.xml``: per way and direction, the lanes of its longest kind of piece."""
    import xml.etree.ElementTree as ET
    by = collections.defaultdict(collections.Counter)
    for e in ET.parse(net).getroot().iter("edge"):
        if e.get("function") == "internal":
            continue
        k = key(e.get("id"))
        if k is None:
            continue
        ls = list(e.iter("lane"))
        ks = [_kind(x) for x in ls]
        by[k][(ks.count("car"), ks.count("bus"), ks.count("bike"))] += float(ls[0].get("length") or 0)
    return {k: c.most_common(1)[0][0] for k, c in by.items()}


def lane_diff(db, osm_file, work_dir, netconvert_bin=None):
    """The differences (a list of dicts: ``way``, ``dir``, ``duckosm``, ``sumo_osm``, ``sources``, ``cause``) and the number of road
    way-directions compared, as ``(rows, compared)``. ``osm_file``: the area's ``.osm`` (XML; a ``.pbf`` is converted with osmium)."""
    import duckdb

    from duckosm.sumo import _find_netconvert, to_sumo
    os.makedirs(work_dir, exist_ok=True)
    binary = _find_netconvert(netconvert_bin)
    if str(osm_file).endswith(".pbf"):
        xml = os.path.join(work_dir, "area.osm")
        subprocess.run(["osmium", "cat", str(osm_file), "-o", xml, "--overwrite"], check=True, capture_output=True)
        osm_file = xml
    own = os.path.join(work_dir, "osm.net.xml")
    res = subprocess.run([binary, "--osm-files", str(osm_file), "-o", own, "--geometry.remove", "false", "--ramps.guess", "false",
                          "--junctions.join", "false", "--osm.turn-lanes", "true", "--osm.lane-access", "true", "--osm.bike-access", "true",
                          "--proj.utm", "true", "--no-warnings", "true"], capture_output=True, text=True)
    if res.returncode != 0:
        raise RuntimeError(f"netconvert --osm-files failed (exit {res.returncode}):\n{res.stderr[-2000:]}")
    con = duckdb.connect(str(db), read_only=True)
    try:
        ours = to_sumo(con, work_dir, net_name="duckosm", netconvert_bin=binary)["net"]
        ref = dict(con.execute("SELECT edge_id::VARCHAR, edge_ref FROM driving.edges").fetchall())
        tags = {str(k): v for k, v in con.execute("SELECT osm_id, tags FROM raw.ways").fetchall()}
        src = collections.defaultdict(set)
        for r, s in con.execute("SELECT e.edge_ref, p.source FROM driving.lane_profile p JOIN driving.edges e USING (edge_id)").fetchall():
            src[(r.split("#")[0], r[-1])].add(s)
    finally:
        con.close()
    osm_key = lambda i: (i.lstrip("-").split("#")[0], "r" if i.startswith("-") else "f") if i.lstrip("-").split("#")[0].isdigit() else None  # noqa: E731
    duck_key = lambda i: (ref[i].split("#")[0], ref[i][-1]) if ref.get(i) and "#" in ref[i] else None  # noqa: E731
    D, O = _lanes(ours, duck_key), _lanes(own, osm_key)
    keys = [k for k in sorted(set(D) & set(O)) if O[k][0] + O[k][1] > 0]
    rows = []
    for k in keys:
        if D[k] == O[k]:
            continue
        t, s = tags.get(k[0], {}), src[k]
        if "override" in s:
            cause = "override"
        elif t.get("oneway:bus") == "no" or t.get("oneway:psv") == "no":
            cause = "contraflow bus lane"          # counted in lanes (OSM); SUMO's import adds it on top
        elif "inherited" in s:
            cause = "inherited"
        elif "turn:lanes" in s:
            cause = "turn:lanes count"             # more turn:lanes entries than lanes: we take the entries, SUMO the lanes tag
        elif "default" in s and not any(x in t for x in ("lanes", "lanes:forward", "lanes:backward")):
            cause = "no lanes tag"                 # our default 1 lane, SUMO's type default
        elif D[k][2] != O[k][2] and D[k][:2] == O[k][:2]:
            cause = "bike lane"
        elif D[k][1] != O[k][1]:
            cause = "bus lane"
        else:
            cause = "other"
        rows.append({"way": k[0], "dir": k[1], "duckosm": D[k], "sumo_osm": O[k], "sources": sorted(s), "cause": cause})
    return rows, len(keys)
