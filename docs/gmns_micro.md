# Microscopic (cell-based) network — `duckosm gmns --micro`

`duckosm gmns --micro` (`to_micro`); driving only, `cell_length_m` default 7 m. The level below
[meso](gmns_meso.md): each lane is chopped into **cells** with **lane-change** connectors, the
representation microsimulation (DTALite micro, cell-transmission) consumes. Built on the per-lane
geometry (drive-side offset) in the `lane` table and the smooth Bézier turn connectors.

> Like meso, micro is **not core GMNS** — it's the osm2gmns `micronet` convention. Column layout
> **mirrors osm2gmns' micronet** for interoperability, plus our stable-id extras.

---

## Scope for v1: driving only

Same reasoning as meso — cells + lane changes are a vehicular concept. **Driving** by default;
**cycling** via `--micro-mode cycling` (opt-in); **walking** excluded (no lanes). Built from the
`gmns_<mode>` (lane, movement) + `meso_<mode>` tables.

---

## The model — three kinds of micro link

osm2gmns' micronet decomposes each meso lane into cells; ours does the same from the **per-lane
offset geometry** already in `gmns_<mode>.lane` (which meso doesn't carry — meso keeps a lane
*count*, so micro reads the `lane` table directly):

1. **cell (normal) micro link** — each lane split into fixed-length cells (`cell_length_m`, default
   **7 m**; the last cell absorbs the remainder). One `micro_link` per cell, geometry = the cell
   slice of the lane's offset line (`ST_LineSubstring`), `lane_no` = the lane number, `cell_type =
   'normal'`.
2. **lane-change micro link** — between **adjacent lanes** of the same macro link: cell *k* of lane
   *i* → cell *k+1* of lane *i±1*, a short diagonal (`cell_type = 'lane_change'`) so a vehicle can
   change lanes. Only where both lanes have a cell *k* (+1).
3. **movement (turn) micro link** — at each junction, the **last cell of the inbound lane** → the
   **first cell of the outbound lane**, one per legal movement (from the `movement` table +
   `turn:lanes` lane assignment, reusing the meso connector logic), `cell_type = 'movement'`,
   `mvmt_txt_id` carried.

**micro nodes** sit at every cell boundary (`{lane_id}@{k}`) plus the lane endpoints.

```
lane 1:  o──cell──o──cell──o──cell──o        (normal cells)
             ╲lane_change╱      ╲movement→ outbound lane
lane 2:  o──cell──o──cell──o──cell──o
```

---

## Stable, reversible ids (the differentiator)

Composite strings, like our lane/meso ids — reversible, collision-free, and every row keeps the
`macro_link_id` / `lane_no` columns:

| micro object | `*_id` | reverses to |
|--------------|--------|-------------|
| cell micro link | `"C{lane_id}#{k}"` | the `lane_id` (= `edge_id_laneNo`) + cell index |
| lane-change link | `"H{lane_id_a}-{lane_id_b}#{k}"` | the two lanes + cell |
| movement micro link | `"X{ib_lane_id}-{ob_lane_id}"` | inbound/outbound lanes |
| micro node | `"{lane_id}@{k}"` | lane + cell boundary |

(osm2gmns numbers all of these sequentially — not stable, not reversible.)

## Column reference (mirrors osm2gmns `micronet`)

**`micro_link`** — `link_id`, `from_node_id`, `to_node_id`, `dir_flag`(1), `length`, `lanes`(1),
`width`, `free_speed`, `facility_type`, `allowed_uses`, `geometry`(WKT) + `geom`(native),
**`macro_link_id`** (= `edge_id`), **`meso_link_id`** (the `M…` section), **`lane_no`**,
**`cell_type`** (`normal`|`lane_change`|`movement`), `mvmt_txt_id` (movement cells), `ctrl_type`.

### Geometry & width — what's real vs. assumed

- **Geometry: real, per-lane.** Each cell's `geometry` is a slice of the lane's **offset centerline**
  (from `gmns_<mode>.lane.geom`) — genuine geometry down to the lane. It is a *centerline*, not a
  width-ribbon/polygon (GMNS/osm2gmns micro cells are centerlines too).
- **Width: only where OSM tags it.** `width` inherits the lane's `width` (from OSM `width:lanes`). On
  Södermalm that is **0 / 3,193 lanes** — essentially untagged — so it's **NULL** there. The lane
  *offset spacing* in the geometry falls back to a **default 3.25 m** (`_DEFAULT_LANE_W`, the same
  value the `lane` offset uses). So the cell geometry always exists, but its lane spacing is *assumed*
  unless the area's OSM actually tags `width:lanes`. Proposed: write `width =
  COALESCE(lane.width, 3.25)` so the column is always populated and consistent with the drawn spacing
  (documented as a default; a downstream model can override).

**`micro_node`** — `node_id`, `x_coord`, `y_coord`, `macro_node_id`, `macro_link_id`, `lane_no`,
`geom`.

## Scale

Expect ~10× the lane count (osm2gmns produced ~35 k micro links for Södermalm driving from ~3.5 k
macro links). Fine for DuckDB; `--micro` is opt-in and logs the count. `cell_length_m` trades detail
for size (bigger cells → fewer rows).

## What we do that osm2gmns' micro doesn't

- **Stable + reversible ids** (above) — micro cells stay joinable to the lane / macro `edge_id`.
- **Tagged lanes** — cells inherit the lane's `allowed_uses` (bus/bike from `psv:lanes`/
  `bicycle:lanes`), so a bus-lane cell is *typed*, not just geometric; turn cells come from
  `turn:lanes`, not geometric inference.
- **One queryable DuckDB** — `micro_<mode>` alongside `gmns_<mode>` / `meso_<mode>`, renderable in the
  [viewer](gmns_viewer.md) (a third layer).

## Not in v1

- Sub-cell signal/detector placement, cell-level VDF, and time-of-day — out of scope (no source).
- Curved turn-cell geometry (v1 movement cells are straight, like the meso connectors).

## Usage (proposed)

```bash
duckosm gmns data/db/sodermalm.duckdb --meso --micro            # build meso + micro (driving)
duckosm gmns data/db/sodermalm.duckdb --micro --micro-mode cycling
```

`--micro` implies `--meso` (micro reuses the meso connectors / needs the section ids).

## Implementation sketch

New `to_micro(gmns_db, modes=["driving"], cell_length_m=7.0)` in `src/duckosm/gmns.py` + `--micro` /
`--micro-mode` on `duckosm gmns`. Per mode, from `gmns_<mode>.lane` + `.movement` (+ `meso_<mode>`):
(1) `generate_series` cell boundaries per lane → `micro_node` + normal `micro_link` cells via
`ST_LineSubstring`; (2) lane-change links joining adjacent `lane_no` at matching cells; (3) movement
links from each inbound lane's last cell to the outbound lane's first cell (turn:lanes assignment,
lifted from `_build_meso_links`). All in-engine DuckDB SQL; verified like meso (cell coverage,
referential integrity, id reversibility). Then folded into a `docs/` note + the viewer as a layer.
