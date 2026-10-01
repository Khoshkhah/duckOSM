"""Turn-by-turn directions for a route: route-guidance's manoeuvre rules (the OSRM model).

duckOSM is the reference for routing (docs/design/viz_on_mapstyle.md); mapstyle's route planner
runs the same rules in the page, and a test keeps the two equal.

    from duckosm import route_points, directions
    r = route_points(con, (7.4155, 43.7285), (7.4400, 43.7480))
    [s["text"] for s in directions(con, r)]
    # ['Head northeast on Boulevard de Belgique', 'At the roundabout, take the 2nd exit onto …', …]

Each step: ``{"type", "modifier", "name", "mode", "at": (lon, lat), "length_m", "text"}``.
Types: depart, turn, fork, new name, roundabout (with ``exit``), mode (a change of mode), arrive.
"""
import math

_COMPASS = ["north", "northeast", "east", "southeast", "south", "southwest", "west", "northwest"]
_VERB = {"walking": "on foot", "cycling": "by bike", "driving": "by car"}
_R = 6371000.0


def _hav(a, b):
    dl, dn = math.radians(b[1] - a[1]), math.radians(b[0] - a[0])
    h = math.sin(dl / 2) ** 2 + math.cos(math.radians(a[1])) * math.cos(math.radians(b[1])) * math.sin(dn / 2) ** 2
    return 2 * _R * math.asin(math.sqrt(h))


def _brg(a, b):
    f1, f2, dn = math.radians(a[1]), math.radians(b[1]), math.radians(b[0] - a[0])
    y = math.sin(dn) * math.cos(f2)
    x = math.cos(f1) * math.sin(f2) - math.sin(f1) * math.cos(f2) * math.cos(dn)
    return (math.degrees(math.atan2(y, x)) + 360) % 360


def _head(c, from_start):
    """Heading over the first / last 18 m of a line."""
    acc = 0.0
    if from_start:
        for i in range(1, len(c)):
            acc += _hav(c[i - 1], c[i])
            if acc >= 18 or i == len(c) - 1:
                return _brg(c[0], c[i])
    else:
        for i in range(len(c) - 2, -1, -1):
            acc += _hav(c[i], c[i + 1])
            if acc >= 18 or i == 0:
                return _brg(c[i], c[-1])
    return 0.0


def _turn(b0, b1):                                     # + = right
    return ((b1 - b0 + 540) % 360) - 180


def _modifier(a):
    x, side = abs(a), ("right" if a > 0 else "left")
    return ("straight" if x < 20 else "uturn" if x >= 170 else f"slight {side}" if x < 45
            else side if x < 135 else f"sharp {side}")


def _is_turn(m):
    return m in ("left", "right", "uturn") or m.startswith("sharp")


def _nth(n):
    return f"{n}{'th' if 11 <= n % 100 <= 13 else {1: 'st', 2: 'nd', 3: 'rd'}.get(n % 10, 'th')}"


def _say(s):
    onto, on = (f" onto {s['name']}", f" on {s['name']}") if s["name"] else ("", "")
    t = s["type"]
    if t == "depart":
        return ("Walk " if s["mode"] == "walking" else "Head ") + s["modifier"] + on
    if t == "turn":
        return "Make a U-turn" + onto if s["modifier"] == "uturn" else f"Turn {s['modifier']}{onto}"
    if t == "fork":
        return f"Keep {s['modifier']} at the fork{onto}"
    if t == "new name":
        return "Continue" + onto
    if t == "roundabout":
        return f"At the roundabout, take the {_nth(s['exit'])} exit{onto}"
    if t == "mode":
        return f"Continue {_VERB.get(s['mode'], '')}{onto}"
    return "Arrive at your destination"


def _legs(route, mode):
    """[(mode, [edge_id, …], {edge_id: metres used})] from any duckOSM route result."""
    if route.get("legs"):
        return [(l["mode"], list(l["edges"]), {}) for l in route["legs"]]
    used = {p["edge_id"]: p["length_m"] for p in route.get("path") or [] if "length_m" in p}
    return [(mode, list(route["edges"]), used)]


def directions(con, route, mode="driving"):
    """Turn-by-turn steps for ``route``: the result of :func:`duckosm.route`,
    :func:`duckosm.route_points` (``mode`` = its mode) or :func:`duckosm.route_multimodal` /
    :func:`duckosm.route_multimodal_points` (modes from its legs). ``[]`` for an empty route."""
    if not route:
        return []
    legs = _legs(route, mode)
    info, outs = {}, {}
    for m in {m for m, _, _ in legs}:
        ids = [k for lm, ks, _ in legs if lm == m for k in ks]
        if not ids:
            continue
        rows = con.execute(
            f"SELECT edge_id, name, highway, length_m, source, junction, ST_AsText(geometry) "
            f"FROM {m}.edges WHERE edge_id IN (SELECT unnest(?))", [ids]).fetchall()
        for k, name, hw, ln, src, jn, wkt in rows:
            body = wkt[wkt.index("(") + 1:wkt.rindex(")")]
            info[(m, k)] = {"name": name, "hw": hw, "len": ln, "src": src,
                            "rb": jn in ("roundabout", "circular"),
                            "c": [tuple(map(float, p.split()[:2])) for p in body.split(",")]}
        srcs = list({v["src"] for (mm, _), v in info.items() if mm == m})
        outs[m] = {}
        for src, k, wkt in con.execute(
                f"SELECT source, edge_id, ST_AsText(geometry) FROM {m}.edges "
                f"WHERE source IN (SELECT unnest(?))", [srcs]).fetchall():
            body = wkt[wkt.index("(") + 1:wkt.rindex(")")]
            outs[m].setdefault(src, []).append([tuple(map(float, p.split()[:2])) for p in body.split(",")])
    E = []
    for m, ks, used in legs:
        for k in ks:
            e = info[(m, k)]
            E.append({**e, "mode": m, "len": used.get(k, e["len"]), "out": outs[m].get(e["src"], [])})
    if not E:
        return []

    def same(a, b):
        return a["name"] == b["name"] if (a["name"] or b["name"]) else a["hw"] == b["hw"]

    def step(kind, mod, e):
        return {"type": kind, "modifier": mod, "name": e["name"], "mode": e["mode"], "at": e["c"][0],
                "length_m": 0.0}

    steps, cur, rb, out_b = [], None, None, 0.0
    for i, e in enumerate(E):
        e_in, e_out = _head(e["c"], True), _head(e["c"], False)
        if e["rb"]:                                    # a roundabout: one "take the Nth exit"
            if not rb:
                if cur:
                    steps.append(cur)
                cur, rb = None, {**step("roundabout", "", e), "name": None, "exit": 0}
            rb["exit"] += 1
            rb["length_m"] += e["len"]
            out_b = e_out
            continue
        if rb:
            rb["name"], cur, rb = e["name"], rb, None
            cur["length_m"] += e["len"]
            out_b = e_out
            continue
        if not cur:
            cur = step("depart", _COMPASS[int(((e_in + 22.5) % 360) // 45)], e)
        else:
            prev = E[i - 1]
            mod = _modifier(_turn(out_b, e_in))
            sharp = mod == "uturn" or mod.startswith("sharp")
            junction, named = len(e["out"]) >= 3, (not same(prev, e)) and bool(e["name"])
            fork = None                                # 2+ ways forward: keep left / right
            if junction and not _is_turn(mod):
                mine = _turn(out_b, e_in)
                sibs = [_turn(out_b, b) for b in (_head(c, True) for c in e["out"]) if abs(_turn(e_in, b)) > 8]
                if (12 <= abs(mine) < 50 and not any(abs(t) < 12 for t in sibs)
                        and any(abs(t) < 50 and (t < 0) != (mine < 0) for t in sibs)):
                    fork = "left" if mine < 0 else "right"
            nxt = (step("mode", mod, e) if e["mode"] != prev["mode"]
                   else step("turn", mod, e) if sharp or (junction and _is_turn(mod))
                   else step("fork", fork, e) if fork else step("new name", mod, e) if named else None)
            if nxt:
                steps.append(cur)
                cur = nxt
        cur["length_m"] += e["len"]
        out_b = e_out
    steps.append(rb if rb else cur)
    steps.append({"type": "arrive", "modifier": "", "name": None, "mode": E[-1]["mode"],
                  "at": E[-1]["c"][-1], "length_m": 0.0})
    for s in steps:
        s["text"] = _say(s)
    return steps
