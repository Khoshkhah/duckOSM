# MATSim lanes.xml + signals

`duckosm matsim-lanes` (`to_matsim_lanes`). The detailed-intersection companion to
[`matsim`](matsim_export.md): turn **lanes** (`lanes.xml`) and traffic **signals**
(`signalSystems` / `signalGroups` / `signalControl`), so a MATSim run can model lane-level turn
restrictions and signalised junctions. Every file is validated against the official MATSim **XSD**
schemas (v2.0; vendored under `tests/fixtures/matsim_xsd/`).

**Source = a GMNS db**, not the core routing db — because the [GMNS `movement`](gmns_export.md) table
already is the lane→turn→downstream model (`ib_link_id` → `ob_link_id`, with `start_ib_lane`/
`end_ib_lane` from `turn:lanes`, turn restrictions & U-turns already honoured), and `node.ctrl_type` /
`signal_controller` mark the signalised nodes. `link_id` **is** `edge_id`, so these files line up with a
`network.xml` exported from the same build. Build order: core db → `duckosm gmns` → `matsim` (network)
+ `matsim-lanes` (lanes/signals).

## lanes.xml — `laneDefinitions_v2.0`

One `<lanesToLinkAssignment linkIdRef="{ib_link}">` per inbound link that has ≥1 legal movement. The
lanes encode **which downstream links each lane may feed** — i.e. turn connectivity + turn:lanes:

```xml
<lanesToLinkAssignment linkIdRef="{ib_edge_id}">
  <lane id="{ib_edge_id}_1">
    <leadsToLink refId="{ob_edge_id_left}"/>
    <representedLanes numberOfRepresentedLanes="1"/>
    <capacity representedCapacity="1600"/>
    <startsAt meterFromLinkEnd="45.0"/>
  </lane>
  ...
</lanesToLinkAssignment>
```

Per inbound link at a node, from its movements:
- **legal `ob_links`** = the distinct `ob_link_id` (restrictions already applied in the movement build).
- **if `turn:lanes` tagged** (`start_ib_lane`/`end_ib_lane` present): one `<lane>` per lane index,
  `leadsToLink` = the `ob_link`s of the movements assigned to that lane — true lane-level turns.
- **else** (the common urban case): `link.lanes` lanes, each `leadsToLink` **all** legal `ob_links`
  (any lane → any legal turn) — so no legal movement is forbidden, restrictions still honoured.
- `capacity` = the link's per-lane `_CAPACITY`; `startsAt` = a short default turn-pocket length
  (`min(link_length, 45 m)`).

**Value:** even without `turn:lanes`, this injects the **legal turn set** (turn restrictions) into
MATSim's lane model; with `turn:lanes` it's true per-lane turn assignment.

## signals — `signalSystems` / `signalGroups` / `signalControl` (v2.0, three files)

For each **signalised node** (`ctrl_type='signal'`, 11 on Södermalm) with movements:

- **signalSystems.xml** — one `<signalSystem id="{node_id}">`; one `<signal id="{ib_link}">` per
  controlled inbound link, `linkIdRef="{ib_link}"` (+ its `laneIds` when lanes.xml defines them).
- **signalGroups.xml** — one `<signalGroup>` per approach (inbound link), the unit that turns
  green/red together.
- **signalControl.xml** — a **default fixed-time plan** per system:
  `DefaultPlanbasedSignalSystemController`, `cycleTime=90 s`, approaches split into **two phases by
  orientation** (N–S vs E–W, from the movement `mvmt_code`), ~42 s green + a few seconds all-red each;
  if orientation is unclear, fall back to equal round-robin per approach.

> **Fidelity — read this.** *Which* intersections are signalised is **real** (OSM `traffic_signals`).
> The **timing is a synthetic default** — OSM carries no signal plans — so `signalControl.xml` is a
> calibrate-me placeholder (a plausible 2-phase 90 s cycle), not measured operation. lanes.xml turn
> **connectivity** is real; per-lane turn *assignment* is real only where `turn:lanes` is tagged.

## CLI / API

```bash
duckosm matsim         monaco.duckdb                 # network.xml (existing, from core db)
duckosm gmns           monaco.duckdb -o gmns.duckdb  # movements + signals (prereq)
duckosm matsim-lanes   gmns.duckdb                          # -> lanes.xml + signalSystems/Groups/Control.xml
duckosm matsim-lanes   gmns.duckdb --no-signals             # lanes.xml only
```

```python
from duckosm import to_matsim_lanes
to_matsim_lanes("gmns.duckdb", out_dir=".", mode="driving", signals=True)   # -> {lanes, systems, groups, control}
```

- `to_matsim_lanes(gmns_db, out_dir, mode="driving", signals=True, cycle_s=90)` → writes `lanes.xml`
  and (unless `signals=False`) `signalSystems.xml` / `signalGroups.xml` / `signalControl.xml`; returns
  counts. Plain XML (these are config-side, not the big gzipped network).

## Scope

**v1:** lanes.xml (full) + the three signals files with a default fixed-time plan. **Out of scope:**
actuated/adaptive control, coordinated green waves, real timings (need a survey/vendor source), and
the MATSim `config.xml` flags that switch signals on (the user's scenario, not the network).

## Tests

- `lanes.xml`: an assignment per inbound link with movements; `leadsToLink` covers exactly the legal
  `ob_links`; per-lane split when `turn:lanes` present; **validates against vendored
  `laneDefinitions_v2.0.dtd`**.
- signals: a `signalSystem` per signalised node; a group per approach; a default control plan with the
  expected `cycleTime`; **validates against the vendored signal DTDs**.
- A no-`turn:lanes` fixture (all-lanes-all-turns) and a `turn:lanes` fixture (per-lane) both covered.
