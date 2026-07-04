# Mesoscopic network — `duckosm gmns --meso`

**Status:** shipped 2026-07-04 (`to_meso` / `duckosm gmns --meso`). Driving by default; **cycling**
supported via `--meso-mode cycling`; walking excluded (no lanes).

Build a **lane-level mesoscopic network** from the GMNS extract, into the same standalone GMNS
DuckDB. It's the bridge between the macro (link-level) network and a micro (cell-level) one, and the
input mesoscopic DTA tools (DTALite) simulate on. duckOSM's angle vs [osm2gmns](gmns_export.md#comparison-with-osm2gmns):
**stable + reversible ids, turn-restriction-honoring connectors, and per-lane OSM semantics** — none
of which osm2gmns' geometric meso carries.

> Meso/micro are **not core GMNS** (the spec stops at node/link/lane/movement). They're the osm2gmns
> de-facto convention — so this table layout **mirrors osm2gmns' `mesonet`** columns for
> interoperability, plus our stable-id / semantic extras.

---

## Scope for v1: driving only

A meso network is a *vehicular lane* construct, so the modes are not equal:

| Mode | v1 | Why |
|------|----|-----|
| **driving** | ✅ build | lanes, turn bays, lane→movement assignment all matter; meso DTA is for cars |
| cycling | ✅ (opt-in flag) | `--meso-mode cycling`; cycleways are mostly single-lane so meso ≈ macro, but built on the same code path |
| walking | ✕ never | footways have no lanes; pedestrian sim uses *surfaces*, a different primitive (Tier-3 Vadere/JuPedSim) |

**Multimodal (stitched): deferred** — it inherits the cross-mode `edge_id` collision and would fold in
a meaningless walking-meso. Per-mode `meso_<mode>` schemas compose later if a real need appears.

---

## Output

A **`meso_driving`** schema in the same GMNS DuckDB (alongside `gmns_driving`), with two tables —
`meso_node`, `meso_link` — carrying native `geom` so it renders in place. Built by reading the
existing `gmns_<mode>` tables (`link`, `lane`, `movement`), so it needs the GMNS db, not the raw OSM.

```bash
duckosm gmns data/db/sodermalm.duckdb --meso            # build gmns_* AND meso_driving in one file
duckosm gmns data/db/sodermalm.duckdb --meso cycling    # also build meso_cycling (opt-in)
# or, on an already-built GMNS db:
python -c "from duckosm.gmns import to_meso; to_meso('sodermalm_gmns.duckdb', modes=['driving'])"
```

---

## The model — two kinds of meso link

Following the osm2gmns split (its log literally logs "normal meso links" then "movement meso links"):

1. **normal (section) meso link** — the body of a macro link. **v1: one per macro link** (no mid-link
   splitting for lane add/drop yet — see *Deferred*). Geometry = the macro link's line, trimmed a few
   metres at each end to leave room for the connectors. Carries the macro link's `lanes`, `free_speed`,
   `capacity`, `facility_type`, `allowed_uses`.
2. **movement (connector) meso link** — at each junction, **one per legal movement** taken straight
   from the `movement` table (which came from the turn-restriction-respecting `edge_graph`, so a turn
   OSM forbids has **no** connector — osm2gmns would draw it). Connects the downstream meso node of
   the inbound macro link to the upstream meso node of the outbound macro link. `lanes` = the number
   of inbound lanes feeding that turn, read from **`turn:lanes`** (our `lane.turn`), with
   `start_ib_lane`/`end_ib_lane`. Geometry = a short connector line across the junction.

**meso nodes** — two per macro link: an **upstream** node at its start and a **downstream** node at
its end (positioned at the trim points). Normal links run upstream→downstream; connectors run
downstream(inbound)→upstream(outbound).

```
   macro:      A ───────────────▶(J)───────────────▶ B
   meso:   Aup ══normal(A)══▶ Adown ──conn(A→B)──▶ Bup ══normal(B)══▶ Bdown
                                     ╲conn(A→C)      (only for turns legal in edge_graph)
```

---

## Stable, reversible ids (the differentiator)

No hashing — composite strings, like our existing `lane_id`/`mvmt_id`, so they're **reversible** and
collision-free, and every table also keeps the macro id as an explicit column:

| meso object | `*_id` | reverses to |
|-------------|--------|-------------|
| normal meso link | `"M{edge_id}"` | the macro `edge_id` |
| movement meso link | `"X{from_edge}-{to_edge}"` | the two macro links (= a `movement`) |
| upstream meso node | `"{edge_id}u"` | macro link + end |
| downstream meso node | `"{edge_id}d"` | macro link + end |

(osm2gmns numbers these sequentially → not stable, not reversible.)

---

## Column reference (mirrors osm2gmns `mesonet` + our extras)

**`meso_link`** — `link_id`, `from_node_id`, `to_node_id`, `dir_flag`(1), `length`, `lanes`,
`capacity`, `free_speed`, `facility_type`, `allowed_uses`, `geometry`(WKT) + `geom`(native), and the
traceability/semantic columns: **`macro_link_id`** (= `edge_id`), **`meso_type`** (`normal`|`movement`),
`movement_id` (the `mvmt_id` for connectors), `mvmt_txt_id` (e.g. `NBL`/`EBT`), `start_ib_lane`,
`end_ib_lane`, `ctrl_type` (`signal` at signalized junctions).

**`meso_node`** — `node_id`, `x_coord`, `y_coord`, `macro_node_id`, `macro_link_id`, `geom`.

---

## What we do that osm2gmns' meso doesn't

1. **Stable + reversible ids** (above) — meso stays joinable to macro and survives rebuilds.
2. **Turn-restriction-correct connectors** — from `edge_graph`, not geometric guessing.
3. **Real lane→movement assignment** — `turn:lanes` tells us which inbound lane feeds which turn, so
   the connector `lanes`/`start_ib_lane`/`end_ib_lane` are *tagged*, not inferred.
4. **Use-aware sections (optional)** — because our `lane` rows carry `allowed_uses`, a normal meso
   link can be split into use-homogeneous sub-links (a bus-lane meso link vs a general one). *v1:
   carry the majority use; the split is a fast follow.*

---

## Deferred (not in v1)

- **Mid-link splitting** where lane count changes along a macro link (osm2gmns' homogeneous-capacity
  sections). Our edges are already split at junctions; intra-link lane changes are rarer — add later.
- **Use-homogeneous section splitting** (bus/bike sub-links) — needs the per-lane use grouping.
- **Micro (cell) network** — explicitly out of scope; that's osm2gmns' mature niche (see the
  [comparison](gmns_export.md#comparison-with-osm2gmns)). If micro is needed, feed this meso to a
  microsim tool.
- **cycling / multimodal meso** — behind a flag / deferred as above.

---

## Implementation

`to_meso(gmns_db, modes=["driving"])` in `src/duckosm/gmns.py`, plus a `--meso` / `--meso-mode` flag on
`duckosm gmns`. Per mode it opens the GMNS db (writable), and from `gmns_<mode>.link` / `lane` /
`movement`:

1. `meso_node` — 2 rows per link (upstream/downstream), at points a few metres inside each end
   (`ST_LineInterpolatePoint`).
2. normal `meso_link` — 1 per link, geometry trimmed to the meso nodes (`ST_LineSubstring`).
3. movement `meso_link` — 1 per `movement` row, connector geometry from the inbound downstream node to
   the outbound upstream node, `lanes`/`start_ib_lane`/`end_ib_lane` from the inbound link's `lane`
   rows whose `turn` matches the movement `type`.

All in-engine DuckDB SQL + a little geometry; no new OSM parse. Verified the same way as the extractor
(referential integrity, id reversibility, connector-count == legal-movement count). See the
[GMNS export doc](gmns_export.md) for the tables it builds on.
