# GMNS extras — capacity, richer movements, cycling meso, combined mode-tagged

Fills GMNS spec columns the base export leaves empty (capacity, movement details), adds a cycling
meso network, and a combined mode-tagged network `gmns_all` (`duckosm gmns --combined`).

Confirmed on Södermalm driving: `link.capacity` 0/2,876, `movement.mvmt_code`/`geometry`/
`start_ib_lane`/`end_ib_lane` all 0/4,503 — these are populated columns in the spec that we leave
blank today. Cycling meso already builds (4,164 sections + 7,339 connectors); it just isn't exposed
as a first-class, tested option.

---

## 1. `link.capacity` — class defaults ✅ Phase 1 (auto, in `to_gmns`)

GMNS `capacity` = *saturation capacity (pce/hr/lane)*. Fill it from the `facility_type` (OSM highway
class), the same "fill a sensible default" move osm2gmns makes. `_link` folds into its parent.

| facility_type | capacity (pce/hr/lane) | | facility_type | capacity |
|---|---|---|---|---|
| motorway | 2000 | | unclassified | 1000 |
| trunk | 1800 | | residential | 800 |
| primary | 1600 | | living_street | 300 |
| secondary | 1400 | | service | 300 |
| tertiary | 1200 | | *other* | 800 |

Per-lane value (GMNS convention); a lookup like `free_speed`/`lanes` already use. Documented as a
default (a downstream model can override).

## 2. `movement` enrichment ✅ Phase 1 (auto, in `to_gmns`)

Three NULL columns, all derivable from data we already have:

- **`mvmt_code`** → the standard **direction+turn code** (`NBL`, `EBT`, `WBR`, `SBU`…). Inbound
  *cardinal* from the inbound link's bearing at the junction — compass-binned `NB/EB/SB/WB`
  (N=[315,45), E=[45,135), S=[135,225), W=[225,315)) — plus the turn letter from the existing `type`
  (`left→L`, `thru→T`, `right→R`, `uturn→U`). We already compute the inbound bearing for `type`.
- **`geometry`** → a short connector `LINESTRING` for the turn path: `ST_MakeLine(
  ST_LineInterpolatePoint(ib.geometry, 0.85), ST_LineInterpolatePoint(ob.geometry, 0.15))` — a 2-point
  line curving through the junction (for QA / rendering; osm2gmns' movements carry geometry too).
- **`start_ib_lane` / `end_ib_lane`** → the inbound lanes feeding the turn, from the `turn:lanes`-derived
  `lane.turn` (min/max `lane_num` whose turn category matches `type`) — the **same tagged lane→turn
  logic the meso connectors already use**, just written onto the macro `movement` too.

## 3. Cycling meso — expose + test ✅ Phase 1 (already works)

`to_meso(modes=['cycling'])` and `duckosm gmns --meso --meso-mode cycling` already build a
`meso_cycling` schema. This item just makes it a **documented, tested** option (update
[gmns_meso.md](gmns_meso.md) from "driving-only v1" to "driving + cycling; walking still excluded —
no lanes", and add a cycling-meso test). No new code.

## 4. Combined mode-tagged GMNS — ✅ Phase 2 (`--combined`)

The single-network variant the multimodal note flagged. Today each mode is its own `gmns_<mode>`
schema (because `edge_id` collides across modes). A combined network unifies them:

- **`gmns_all` schema** — one `link` table = the union of every mode's `link`, **grouped by
  `link_id` (= `edge_id`)**, so the shared physical edge (1,285 driving↔walking on Södermalm) becomes
  **one row** with `allowed_uses` = the union of the modes that contain it (`auto,bike,walk`). Because
  we group by `edge_id`, `link_id` is unique again — the exact problem that forced per-mode schemas is
  solved by the *merge*, not by renumbering. `node` = union by `node_id`. `lane`/`movement` stay
  per-mode-referenced or are omitted in v1.
- Opt-in: `duckosm gmns … --combined` (writes `gmns_all` alongside the per-mode schemas).
- This is what a **mode-tagged** MATSim/GMNS assignment wants (links carry which modes use them), and
  it's a duckOSM-native win — osm2gmns builds one network *type* per run and can't merge them (no
  stable shared id).

---

## Sequencing

- **Phase 1** (this doc, cheap): `capacity` + `movement` enrichment become part of `to_gmns` (always
  populate — strictly fills spec columns); cycling meso documented + tested. Small, high-confidence.
- **Phase 2**: combined mode-tagged `gmns_all` (opt-in). Moderate.

<a id="then-micro"></a>
## Then: micro (cell-based) network — *separate doc + implementation, next*

After this round-out, the next item is a **microscopic (cell-based) network** — each meso lane
decomposed into cells with lane-change connectors, the level osm2gmns' `micronet` provides and the
one duckOSM doesn't yet. That gets its **own design doc** (`docs/gmns_micro.md`) and implementation,
built on the `meso_<mode>` tables this and the meso work produced.

## Implementation notes

Phase 1 slots into `src/duckosm/gmns.py`: `_build_link` gains a `facility_type → capacity` CASE;
`_build_movement` gains the `mvmt_code` (bearing→compass + turn), the `geometry` connector, and the
`turn:lanes` lane-range join (lifted from `_build_meso_links`). Phase 2 adds a `_build_combined`
step + a `--combined` flag. All in-engine DuckDB SQL; verified like the rest (non-null coverage,
`mvmt_code` sanity, referential integrity). Then folded into [gmns_export.md](gmns_export.md).
