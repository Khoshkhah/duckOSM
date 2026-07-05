# OpenDRIVE (.xodr) export → `duckosm opendrive` (design)

**Status:** **Phase 1 shipped** 2026-07-04 (`to_opendrive` / `duckosm opendrive`) — roads + lanes +
geometry; routable junctions are the Phase-2 follow-on. Tier 2 on the
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
