# Bus-only roads and bus lanes in driving: visible, not routable

**Status:** implemented 2026-10-01, as approved by Kaveh the same day: both cases, in
`private_edges` with `access = 'bus'`, `psv` treated as `bus` (Kaveh: "we should have it in driving
mode the same as private roads"). Monaco: driving `edges` unchanged (2,765, same ids);
`private_edges` 196 -> 214 (18 bus edges: 8 bus-lane directions, 10 on bus-only service roads).

## Today

Driving is the car network, and a road or direction only buses may use is left out of it:

- **Bus lanes against a one-way street** (`oneway=yes` + `oneway:bus=no` or `oneway:psv=no`):
  driving makes the one-way edge only, with no reverse twin. Example: Boulevard des Moulins, OSM way
  166009794, edge 7754084833459313314; its reverse (1964280132851416298) is a bus lane shared with
  bikes (`cycleway:left=share_busway`), so it exists in cycling (`cycle_type = bus_cycle_lane`) and
  walking, but not in driving. Monaco: 5 such ways (secondary, tertiary).
- **Bus-only roads** (the driving access value is `psv`, or `no` / `private` with `bus` or `psv` =
  `yes` / `designated`, or `highway=busway`): forbidden for cars, so not in driving at all
  ([access rule](access_private.md)). Monaco: 5 such ways (service roads).

So a map of the driving network shows a one-way arrow where buses run both ways, and bus-only roads
are missing.

## Rule

In driving, a road or direction that **only buses** may use is built like any other (cut at the
same points, the same `edge_id` formula), then moved out of `edges` with the private roads: drawn
on the maps, never routed or exported. Exactly what is done for private roads today.

- **A bus lane against a one-way street** gets its reverse edge built (as if the way were two-way,
  for that direction only), with `access = 'bus'`. Its `edge_id` is the usual
  `hash(osm_id, source, target)`, so it is the same id as that direction in cycling and walking.
- **A bus-only road** is no longer forbidden in driving: its edges get `access = 'bus'`.
- Both are moved, with the private roads, before the graph of legal turns is made. `edges`,
  `edge_graph`, `route()`, every export and the component filter see no change: the routable
  driving network keeps the same edges and ids (Monaco: 2,765).

Other modes don't change: cycling and walking already have these roads where they may use them.

## Where they go (decision 2)

Recommended: **in `<mode>.private_edges`**, with `access` saying why (`private` or `bus`). One table
for "on the map, not for routing": every consumer that already leaves private roads out leaves
these out too, with no code change. The cost is the name: the table holds bus lanes too, so the docs
describe it as "roads you can see but not use: private roads and bus-only roads".

Alternative: a new table `<mode>.bus_edges`. A clearer name, but `viz`, `route-map`, `info`,
`extract`, the clip and the docs each need to learn a second side table.

## Maps

`duckosm viz` and `route-map` already draw `private_edges`. They draw bus edges the same way, in
their own colour (a muted blue), with their own row in the Roads box, **"Bus lanes"**, next to
Private roads; the popup says `access: bus`. The route planner never snaps to them, as for private
roads.

## Checks

- Tests: a one-way way with `oneway:bus=no` gets one routable edge and one `access='bus'` reverse in
  `private_edges`, with the reverse's id equal to the cycling twin's; a `psv` road is in
  `private_edges` with `access='bus'`; neither appears in `edge_graph` or an export.
- Monaco: driving `edges` unchanged (2,765, same ids); `private_edges` grows by the bus edges;
  Boulevard des Moulins drawn with its bus lane on the viz map.
- Docs: What each network contains (access table), the database reference, Draw a map (the new row).

## Decisions for Kaveh

1. **Scope.** Recommended: both bus lanes against one-way streets and bus-only roads. Alternative:
   only the bus lanes.
2. **Table.** Recommended: `private_edges` with `access = 'bus'`. Alternative: a new `bus_edges`.
3. **Taxis.** OSM's `psv` means buses *and* taxis. Recommended: treat `psv` like `bus` (the lane is
   still not for private cars), both as `access = 'bus'`; the OSM tags stay in `raw.ways`.
