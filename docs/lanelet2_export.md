# Lanelet2 export → `duckosm lanelet2` (design)

`duckosm lanelet2` (`to_lanelet2`). A lane-level HD-map export in **Lanelet2**, the main open
format for autonomous-driving lane maps (Autoware, the `lanelet2` C++/Python library). Lanelet2 is
OSM XML, so the result also renders in ordinary OSM tools.

> **What this is (and isn't).** This produces an **HD-map *skeleton* in the AD-standard format** — the
> correct lane structure, topology and semantic tags — **not a survey-grade HD map**. The geometry is
> OSM road centerlines offset by assumed lane widths (meter-level, not the cm-accuracy a real HD map
> gets from LiDAR/mobile-mapping), and width/markings are defaulted. It is a legitimate **base layer /
> prior** for simulation, routing/behaviour research, and HD-map prototyping — the map you *refine* with
> better data, not the finished safety-grade map. The highest-value step *toward* HD that stays within
> reach is **Phase-2 regulatory elements** (traffic lights tied to lanelets, stop lines from crossings),
> because that's real semantic data we already hold — not geometry OSM can't give.

## Why Lanelet2 fits duckOSM especially well

- **It's OSM XML** (`.osm`): nodes / ways / relations with Lanelet2 tags — the same primitive world
  duckOSM already lives in, and it **renders directly** in any OSM/deck.gl pipeline (the boundary ways draw
  with zero conversion).
- **It uses lat/lon** — the loader projects to a local metric frame itself, so unlike MATSim/OpenDRIVE
  we **don't reproject**; our native EPSG:4326 lane geometry goes straight in.
- We already produce the exact source: `gmns_<mode>.lane` (3,193 lanes on Södermalm) has a **drive-side
  offset centerline** (`geom`) + `width` + `allowed_uses` + `turn` per lane.

## What a Lanelet2 map is

Three primitives (all OSM elements):
- **Point** = OSM `<node>` (a boundary vertex; lat/lon).
- **Linestring** = OSM `<way>` tagged `type=line_thin|road_border|virtual|…` — a **lane boundary**.
- **Lanelet** = OSM `<relation>` `type=lanelet` with a **`left`** and a **`right`** way member (the two
  boundaries); travel direction is the boundary point order. Tags: `subtype`, `location`, `one_way`,
  `speed_limit`, …

A lanelet is defined by its **two boundaries**, not a centerline (the library derives the centerline).

## Mapping: GMNS lane → Lanelet2

The one real transform: our lane is a *centerline + width*; a lanelet needs *left + right boundaries*.
Offset the centerline by ±½·width (reusing `gmns._offset_wkt`, already used for the drive-side offset):

| Lanelet2 | ← duckOSM |
|---|---|
| left boundary `<way>` | `_offset_wkt(lane.geom, +width/2)` → nodes + a `type=line_thin` way |
| right boundary `<way>` | `_offset_wkt(lane.geom, −width/2)` → nodes + a `type=line_thin` way |
| lanelet `<relation>` | `left`+`right` members; `type=lanelet` |
| `subtype` | `allowed_uses`: auto→`road`, bus→`bus_lane`, bike→`bicycle_lane`, walk→`walkway` |
| `one_way` | `yes` — our lanes are per-directed-edge, so each lanelet is inherently one-way |
| `speed_limit` | `link.free_speed` (km/h), via `lane.link_id` → `link` |
| `location` | `urban` (default) |
| `duckosm:edge_id` | `lane.link_id` (= `edge_id`) — traceability tag back to the source edge |

```xml
<osm version="0.6" generator="duckosm">
  <node id="1" lat="59.31996" lon="18.06012"/> …
  <way id="10001"><nd ref="1"/><nd ref="2"/>…<tag k="type" v="line_thin"/></way>   <!-- left -->
  <way id="10002"><nd ref="7"/><nd ref="8"/>…<tag k="type" v="line_thin"/></way>   <!-- right -->
  <relation id="20001">
    <member type="way" ref="10001" role="left"/>
    <member type="way" ref="10002" role="right"/>
    <tag k="type" v="lanelet"/><tag k="subtype" v="road"/><tag k="location" v="urban"/>
    <tag k="one_way" v="yes"/><tag k="speed_limit" v="50"/><tag k="duckosm:edge_id" v="…"/>
  </relation>
</osm>
```

**Connectivity via node snapping.** To make lanelets connect longitudinally (successor/predecessor),
boundary points at the same location must be the **same node** — so we **dedup nodes by snapped
coordinate** (~1 cm grid). Where consecutive lanes meet, their boundary endpoints coincide → the
`lanelet2` router infers the connection. (No explicit successor relations needed.)

## Scope

**v1:** lanelets (left/right boundaries from ±½·width) + subtype/one_way/speed_limit tags + node-snap
longitudinal connectivity, from a GMNS db. A valid, loadable, renderable Lanelet2 `.osm`.

**Out of scope → Phase 2 (noted, not dropped):**
- **Shared boundaries** between adjacent same-link lanes (lane *N*'s left = lane *N+1*'s right) — gives
  explicit lane-change adjacency; v1 emits independent boundaries (snapping still connects longitudinally).
- **Regulatory elements** — traffic lights / right-of-way relations (we have the signal + turn data to
  feed them).
- Areas (parking/pedestrian), road-border vs lane-marking boundary typing.

## CLI / API

```bash
duckosm lanelet2 monaco_gmns.duckdb                 # -> monaco_gmns.lanelet2.osm
```
`to_lanelet2(gmns_db, out_path, mode="driving", snap_m=0.01)` → `{lanelets, ways, nodes}`. Reads
`gmns_<mode>.lane` + `link`.

## Fidelity & validation

- Boundaries are the centerline offset by ±½·width, and **width is the 3.25 m default** where OSM lacks
  `width:lanes` → geometrically consistent, uniform, not survey-accurate.
- v1 has longitudinal (node-snap) connectivity but **no explicit lane-change adjacency** (Phase 2) and
  no regulatory elements.
- **Validation:** Lanelet2 has no XSD (it's OSM XML); there's no `lanelet2` lib here, so validation is
  **structural** (well-formed OSM XML; every lanelet relation has exactly one `left` + one `right` way;
  every way's `nd` refs resolve to real nodes; required tags present) with an **Autoware / lanelet2
  load** as the manual acceptance check.

## Tests (structural)

Build a tiny GMNS db (2 lanes on a link) → export: a `type=lanelet` relation per lane with `left`+`right`
way members; boundary ways reference existing nodes; `one_way=yes`, `subtype` by use, `speed_limit`
present; `duckosm:edge_id` tag round-trips; node-snapping dedups a shared endpoint; well-formed XML.
