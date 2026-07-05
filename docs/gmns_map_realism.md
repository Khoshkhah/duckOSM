# Map realism — drive-side lane offset + smooth turn connectors

**Status:** **shipped** 2026-07-04 — §1 drive-side offset (`--drive-side`, default right) and §2
smooth Bézier turn connectors. Two contained geometry refinements that make the **lanes / meso /
micro** maps read like a real road, not a schematic. Both improve everything already built and are the
foundation the [micro network](gmns_micro.md) draws on.

These fix *geometry realism*; they don't invent data — lane **counts/widths** are still only as good
as OSM tags (see [gmns_micro.md](gmns_micro.md#geometry--width-what-s-real-vs-assumed); for measured
Swedish lanes the real source is **NVDB** via the `fetching-sweden-data` sibling).

---

## 1. Drive-side lane offset — separate the two directions

**Problem (measured):** today lanes are offset *symmetrically around the centerline*, so a two-way
road's forward and reverse lanes land on the **same centerline** — separation **0.00 m**. The street
reads as one line, not two lane-sets.

**Fix:** offset each direction's lane bundle to its **travel side** (right for right-hand traffic).
- Current: `off_m = half − (run + w/2)` → lane 1 of a 1-lane edge sits at 0 (centerline).
- New: `off_m = side · (run + w/2)`, where `side = −1` (right-hand traffic) or `+1` (left) and the
  whole bundle sits on one side. Lane 1 centers half a lane off the centerline; extra lanes stack
  outward.
- Because the reverse edge travels the opposite way, offsetting *both* to their travel-right puts
  them on **opposite physical sides** → the two directions separate, as on the ground.
- Config: `drive_side = 'right' | 'left'` (default **right** — Sweden and most of the world).

Applies in `_build_lane_curb` (the `lane.geom` offset), so it improves the lanes, meso and micro views
at once. One-way roads are unaffected visually (still a single bundle).

## 2. Smooth (curved) turn connectors

**Problem:** meso/movement/micro turn connectors are **straight lines** (`ST_MakeLine` of two points)
— they cut the corner and cross other lanes, which is what made the earlier junction views look like
floating triangles.

**Fix:** a **cubic Bézier** curve that respects the inbound and outbound **tangents**, so the turn
sweeps naturally from lane to lane:
- endpoints `P0` (inbound end), `P1` (outbound start); tangents `T0` (inbound bearing), `T1`
  (outbound bearing) — both already computed for the movement `type`/`mvmt_code`.
- control points `C0 = P0 + T0·d`, `C1 = P1 − T1·d`, with `d ≈ 0.4 × |P1−P0|`.
- sample `B(t)` at ~8 steps → a smooth `LINESTRING`.

Built in-engine (a `generate_series(0,8)` Bézier in SQL, or shapely). Replaces the straight connector
in the meso `movement` links, the `gmns` `movement.geometry`, and the micro turn cells — a single
helper reused in all three. Result: junction fans that curve like real turn lanes.

## 3. Micro inherits both

The [micro](gmns_micro.md) cells use the **drive-side-offset** lane geometry, and its turn cells use
the **Bézier** connectors — so the micro map is smooth and side-correct by construction.

---

## Sequence (all doc-first, small commits)

1. **Drive-side offset** — the foundation; every lane map benefits immediately. Add `drive_side`,
   rewrite the offset expression, re-verify two-way separation > 0.
2. **Smooth connectors** — one Bézier helper, wired into meso + `gmns.movement` (+ micro later).
3. **Micro** — on top of 1 + 2 (already specced in `gmns_micro.md`).

## Honest ceiling

These make the geometry *look* right. Fidelity of **how many** lanes and **how wide** still depends
on OSM tagging (thin on Södermalm). For a truly survey-accurate Stockholm lane map, enrich
`lane.lanes`/`lane.width` from **NVDB** (keyed on `edge_id`) — a documented follow-on, not part of
these two refinements.
