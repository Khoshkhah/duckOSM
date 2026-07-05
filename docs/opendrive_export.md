# OpenDRIVE (.xodr) export → `duckosm opendrive` (design)

**Status:** **Phase 1 + 2 shipped** 2026-07-04 (`to_opendrive` / `duckosm opendrive [--junctions]`) —
Phase 1: roads + lanes + geometry; Phase 2 (`--junctions`, from a GMNS db): routable junctions +
turn connecting roads. Tier 2 on the
[export roadmap](../../product/simulation-export-targets.md) — the premium **AV / driving-sim +
commercial-micro** target: one `.xodr` opens **CARLA, esmini** (AV) *and* **PTV Vissim, Aimsun**
(commercial micro). It's the export that finally cashes in the lane-level geometry from
[micro](gmns_micro.md) + [drive-side offset & smooth connectors](gmns_map_realism.md). Verified on
Södermalm: 2,876 roads, well-formed, per-road `length` = Σ planView segments, metric SWEREF99 TM
reference lines.

## What OpenDRIVE is (and why it's the hard one)

ASAM OpenDRIVE describes a road as a **reference line** (`planView` geometry) plus **lanes defined as
signed width offsets** from it, with **junctions** that carry *connecting roads* between the incoming
and outgoing roads. Unlike GMNS/MATSim (node-link graphs), it is a **continuous-geometry, lane-level**
model — the reason it reaches micro/AV simulators the graph formats can't.

```xml
<OpenDRIVE>
  <header revMajor="1" revMinor="7" name="sodermalm">
    <geoReference><![CDATA[+proj=tmerc +lat_0=0 +lon_0=15 +k=0.9996 +x_0=500000 +ellps=GRS80 +units=m]]></geoReference>
  </header>
  <road name="" length="82.4" id="{edge_id}" junction="-1">
    <planView>
      <geometry s="0" x="674032.1" y="6580123.4" hdg="1.5533" length="41.2"><line/></geometry>
      <geometry s="41.2" x="674033.0" y="6580164.6" hdg="1.4102" length="41.2"><line/></geometry>
    </planView>
    <lanes>
      <laneSection s="0">
        <center><lane id="0" type="none"/></center>
        <right>
          <lane id="-1" type="driving" level="false"><width sOffset="0" a="3.25" b="0" c="0" d="0"/></lane>
        </right>
      </laneSection>
    </lanes>
  </road>
</OpenDRIVE>
```

## Mapping: duckOSM → OpenDRIVE

Read from the **core routing db** (`driving.edges`/`nodes`), reprojected to a metric CRS (default
`EPSG:3006`, same as MATSim; the proj4 string goes in `<geoReference>`). duckOSM already **segments
edges at junctions only**, so *one edge = one road between junctions* — a clean fit.

| OpenDRIVE | ← duckOSM |
|---|---|
| `road/@id` | `edge_id` (preserved) |
| `road/@length` | `length_m` |
| `planView` `<geometry><line/>` | one record per polyline segment: `s` (cumulative), reprojected `x,y`, `hdg=atan2(Δy,Δx)`, `length` |
| `right` lanes `-1…-N`, `type="driving"` | `edges.lanes` (per-direction); `<width a>` = lane width (3.25 m default) |
| `junction` | `-1` in v1 (see phasing) |

**Directed roads (v1).** One `<road>` per **directed** edge (oneway, lanes on the `right` side only),
so `road id = edge_id` is 1:1 and stable — a two-way street is two roads. Simpler and id-faithful; the
bidirectional idiom (one road, `left`+`right` lanes) is a possible later refinement.

## Phasing — the honest scope

**Junctions are the hard part and I'm proposing to defer them.** A correct `<junction>` needs
*connecting roads* (separate road elements with their own geometry + lane links) between every
incoming/outgoing pair, and physical-junction detection is not just node degree (a two-way mid-block
point already looks like degree-4: 2 in + 2 out). So:

- **Phase 1 (this doc — v1):** header + `<geoReference>`, and every edge as a **geometrically real,
  lane-level `<road>`** (reprojected reference line + right driving lanes with width), `junction="-1"`,
  **no links**. Result: a **valid, loadable `.xodr`** — esmini/CARLA render the true lane-level road
  network. Geometry & lanes are real; roads don't yet connect *through* intersections.
- **Phase 2 (follow-on):** predecessor/successor `<link>`s at true degree-2 continuations, and
  `<junction>` elements whose **connecting roads reuse the [meso turn-connector Béziers](gmns_meso.md)**
  (we already generate them) — full routable intersections.

This gets a real, inspectable OpenDRIVE deliverable out now, with the genuinely-hard routable-junction
work isolated as a deliberate next step rather than half-built.

## Phase 2 — routable junctions (shipped 2026-07-04, `--junctions`)

Turns Phase-1's disconnected roads into a **routable** network: roads link through junctions, and each
legal turn becomes a **connecting road** carrying the smooth turn geometry. Verified on Södermalm:
2,876 main + **3,924 connecting roads, 765 junctions** (the refined rule — junction iff a turn choice
or ≥3 approaches — correctly leaves two-way mid-blocks as direct road links, not junctions).

**Source = a GMNS db** (`--junctions`), because its `movement` table already carries, per legal turn,
`ib_link_id` → `ob_link_id`, the node, `turn:lanes`, *and* the **Bézier turn geometry** (`geometry`) —
i.e. the connecting-road path. `link_id` = `edge_id` = the Phase-1 `road/@id`, so it all lines up.
Roads come from `gmns_<mode>.link` (same edge geometry/lanes as Phase 1). Verified on Södermalm:
**969 junction nodes, 4,332 connecting roads, 171 simple-through nodes.**

**Junction detection.** A node is a **junction** if any inbound link has ≥2 outbound choices *or* ≥2
inbound links meet; otherwise it's a **simple through** (1-in/1-out → a direct road link).

**Connecting roads.** One `<road junction="{node}">` per movement at a junction node: reference line =
the reprojected movement Bézier, one `driving` lane, and
`<link><predecessor elementType="road" elementId="{ib}" contactPoint="end"/>`
`<successor elementType="road" elementId="{ob}" contactPoint="start"/></link>`.

**`<junction>` element** per junction node, one `<connection>` per movement:
```xml
<junction id="{node}">
  <connection id="0" incomingRoad="{ib}" connectingRoad="{conn_id}" contactPoint="start">
    <laneLink from="-1" to="-1"/>
  </connection>
</junction>
```

**Road end-links** (Phase-1 roads gain `<link>`): at each end of a directed road, by the node's type —
junction end → `<successor|predecessor elementType="junction" elementId="{node}"/>`; simple-through →
`elementType="road"` to the single next/prev road with its `contactPoint`; dead-end → nothing.

**Lane links.** The connecting road has one lane (`-1`); `<laneLink>` maps the inbound lane(s) that
feed the turn — the `turn:lanes` lanes where tagged (`start_ib_lane`/`end_ib_lane`), else `-1→-1`
(a documented simplification; a lane legitimately leads to several connecting roads).

**CLI / API.** `to_opendrive(gmns_db, out, mode, crs, junctions=True)` / `duckosm opendrive gmns.duckdb
--junctions` (requires a GMNS db). Without `--junctions` it stays Phase-1 (core db, no junctions).

**Fidelity & the honest risk.** Turn geometry is the smooth Bézier (plausible, not surveyed); lane
links are simplified (`-1→-1`) without `turn:lanes`. The connecting-road Bézier meets the incoming/
outgoing roads *near* — not exactly at — their ends (it's sampled at 0.94/0.06 along them), so there's
a small geometric gap a strict simulator may flag; snapping connector endpoints to the road ends is a
refinement. **Bigger caveat:** with no esmini/CARLA or ASAM XSD available here, I can emit
**spec-correct** junctions with structural tests, but can't *prove* simulator acceptance — that needs
a load in esmini/CARLA on your side. Flagging before, not after.

**Tests (structural).** A `<junction>` per detected junction node; a connecting road per movement with
pred/succ links + `junction=` set; Phase-1 roads gain junction/road end-links; `<laneLink>` present;
well-formed; connecting-road count = movements at junction nodes.

## CLI / API

```bash
duckosm opendrive sodermalm_pbf.duckdb                     # -> sodermalm_pbf.xodr (driving, EPSG:3006)
duckosm opendrive tartu_pbf.duckdb --crs EPSG:32635        # UTM 35N
```

```python
from duckosm import to_opendrive
to_opendrive("sodermalm_pbf.duckdb", "network.xodr", mode="driving", crs="EPSG:3006")
```

`to_opendrive(source, out_path, mode="driving", crs="EPSG:3006")` → `{"roads": n}`.

## Fidelity

Reference-line **geometry, lengths, lane counts, and widths** are real (widths default 3.25 m where
OSM lacks `width:lanes`, same caveat as everywhere). **v1 has no junction connectivity** — roads carry
correct geometry but don't link through intersections until Phase 2. Elevation/superelevation, road
markings, and signals are out of scope.

## Validation & tests

- **XSD note:** the ASAM OpenDRIVE 1.7 schema on code.asam.net turned out to sit behind **GitLab
  auth** (raw requests redirect to a login page), so it isn't freely vendorable like the MATSim XSDs.
  As the plan allowed, validation is therefore **structural** (well-formed + the OpenDRIVE invariants
  below), with an **esmini/CARLA load** as the manual acceptance check. If a redistributable copy of
  the XSD is later obtained, add an `lxml` schema test.
- Tests (`tests/test_opendrive.py`): a road per edge; `planView` segment count = polyline segments;
  per-road `length` = Σ segment lengths; N right `driving` lanes with width and decreasing negative
  ids; center lane id 0; `junction="-1"`; metric coords; exact SWEREF99 TM `geoReference`.
