# GMNS lanes: one-way carriageways of one road placed as one road

**Status:** approved by Kaveh and built 2026-09-30 (branch `paired-carriageways`), checked on Monaco
only (Kaveh: the other areas aren't needed). Kaveh chose to fix this in duckOSM, not in
lanestyle, so every user of `lane.geom` gets it (lanestyle, lane routing, meso / micro, `gmns-map`).

## Problem

A two-way road with a lane count per direction is often mapped in OSM as **two one-way ways**, one
per direction, drawn close together. `_build_lane_curb` (`gmns.py`) places a one-way edge's lanes
**centred on its own way** (`off_m = half − (run + w/2)`,
[gmns_map_realism.md §1](gmns_map_realism.md)). It doesn't look at the other direction's way, so
when the two ways are closer than their lanes need, the two directions' lanes overlap.

- **Tunnel Dorsale** (Monaco, ways 120114108 and 166643410, each `oneway=yes`, `lanes=2`): the two
  ways run 4.6–5.2 m apart, while 2 + 2 lanes need 13 m. Lane 1 of each direction ends up 1.13 m from
  the other; the two lanes overlap by 384 m².
- **Boulevard du Larvotto** (edges 5847493959192600157 / 4298199569386326816): lane 1 of the two
  directions 0.31 m apart, 161 m² overlap.
- Lane pairs of opposite directions on different links overlapping by more than 20 m²: Monaco 52,
  Södermalm 34, Tartu 177 (lanestyle measurement, 2026-09-30).

The data is fine; the placement rule is what's wrong.

## Proposal: place a one-way edge from the middle between it and its partner

For each one-way edge A, its **partner** is the one-way edge(s) B that:

- run the **opposite way** (headings more than 150° apart where they're alongside);
- lie on A's **inner side**: left of A for `drive_side='right'`, right of A for `'left'`;
- are **closer than their lanes need**: the gap `d` between the two centre lines is less than
  `half_A + half_B` (half of each edge's total lane width);
- run alongside A for **at least half of A's length**. A road split at different junctions in its
  two directions has several partner edges; their union counts.

Then A's lanes are placed from the line midway between the two ways, not from A's own centre line:

```
two-way (as today):       off_m = side · (run + w/2)
paired one-way (new):     off_m = side · (run + w/2 − d/2)
plain one-way (as today): off_m = half − (run + w/2)
```

`side` is −1 for right-hand traffic and +1 for left. So with `d = 0` a paired edge is placed exactly
like a two-way road. With `d = half_A + half_B` (just touching) it is placed exactly like today's
centred one-way edge, so the rule is continuous: a pair that is far enough apart doesn't change.

- **`d` per edge:** the median distance from A to its partner(s), over the part of A that runs
  alongside them, sampled every 2 m in the local metre frame `_offset_wkt` already uses.
- **Detection:** a shapely STRtree over the driving mode's one-way edges, in one pass before the
  lane loop. The lane loop only looks up `d` by `edge_id`.
- **Where:** `_build_lane_curb`, so `lane.geom` changes; links, nodes, ids and lane counts don't.
- **Option:** `--pair-carriageways / --no-pair-carriageways` on `duckosm gmns` (default on), so the
  old placement can be compared.

### Ceilings

- **`d` is one number per edge.** Where the two ways converge or split (dual carriageway ends,
  Boulevard du Larvotto's south-west end), the gap changes along the edge and the lanes fit only
  approximately. A gap varying along the edge needs a variable offset; that's a later step if the
  maps show it matters.
- **Two-way ↔ one-way transitions** (Avenue de Monte-Carlo, node 1204288376: a centred one-way lane
  meets an offset two-way piece) are a different case and not part of this change.

## Result on Monaco

Opposite-direction lane pairs on the same level overlapping by more than 20 m²: **32 → 14**, and
1,896 m² → 444 m² in total. The lane 1s of Tunnel Dorsale are now 2.84 m apart (were 1.13 m); those of
Boulevard du Larvotto 2.94 m and 3.01 m (were 0.31 m). The pairs left are partial ones, alongside each
other for less than half their length where carriageways merge or split (the single-`d` ceiling).

## lanestyle side (separate, after this)

GMNS has no field for "this one-way link is paired with that one". So lanestyle finds paired edges
itself, from geometry: lane 1's left edges of two links lying on top of each other (within 0.3 m,
opposite directions). It then draws that boundary as the white centre line, once, instead of two
grey edge lines.

## Checks

- Tests (`tests/test_gmns.py`):
  - two antiparallel one-way edges 4 m apart with 2 lanes each: no overlap, lane 1s 3.25 m apart,
    mirrored about the midline;
  - the same edges 20 m apart: unchanged (centred);
  - a two-way road and a lone one-way road: unchanged;
  - `drive_side='left'` mirrors.
- Rebuild the Monaco / Södermalm / Tartu GMNS dbs and re-count overlapping opposite-direction lanes
  (now 52 / 34 / 177). Then screenshots of Tunnel Dorsale and Boulevard du Larvotto on lanestyle's
  previews page (port 8090).
- Docs: `docs/exports/gmns.md` (lane geometry) and `gmns_map_realism.md` §1 get a line pointing here.
