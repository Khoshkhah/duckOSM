# Pedestrian streets in driving: visible, not routable

**Status:** approved by Kaveh and implemented 2026-10-19 (the table change). The map part is
`mapstyle/docs/design/one_mode_map.md`. The segmentation fix is not done: it is measured first.
`delivery`: `access=delivery` stays rescued in `edges`; `motor_vehicle=delivery` goes to
`private_edges` with label `delivery`. A private pedestrian way has the label `private`.

**Result.** Norrmalm: `driving.edges` has the same 2,293 `edge_id`s (and walking and cycling the
same counts). `driving.private_edges` has 831 more edges: 783 `pedestrian`, 32 `delivery`, 16
`private`. 804 of the 819 pedestrian edges of the first test build have the same `edge_id` as in
walking. Monaco: `driving.edges` 2,765 (unchanged); `private_edges` 689.

## Problem

Cars cannot use a `highway=pedestrian` street, so the driving network leaves it out. A map of the
driving network then shows holes: the cross streets of Drottninggatan end at it, and look cut.
The bus-only streets (Kungsgatan) had the same problem. They are now in `driving.private_edges`
([bus-only roads](bus_only_edges.md)).

Norrmalm (OSM relation 5695409) has 140 `highway=pedestrian` ways. The driving network takes 15 of
them (as `living_street`). It does not take the other 125:

| The most specific car tag of the way | Ways |
|---|---|
| None | 109 |
| `delivery` | 5 |
| `private` | 5 |
| `yes` (from `vehicle` or `access`, not from `motorcar` or `motor_vehicle`) | 3 |
| `no` | 3 |

In walking, these 125 ways have 858 edges, 16.5 km of way.

## Rule

In driving, a `highway=pedestrian` way that is not rescued as `living_street` is built like any
other path. It is cut at the same points as in walking, and it has the same `edge_id`. Then it
moves from `edges` to `private_edges`, with the private roads and the bus-only roads. It is drawn
on maps. It is never routed or exported.

`access` in `private_edges` is the label. It tells why the way is not routable:

| The pedestrian way has | `access` |
|---|---|
| `motor_vehicle=delivery` or `motorcar=delivery` | `delivery` |
| `motor_vehicle=private`, `access=private` (the most specific car tag) | `private` |
| No car tag, `no`, `yes`, or any other value | `pedestrian` |

The most specific car tag decides, in this order: `motorcar`, `motor_vehicle`, `vehicle`,
`access` (the [access rule](access_private.md)).

A way with `highway=pedestrian` and a car tag of `yes`, `designated` or `permissive` is not
changed. It is rescued as `living_street` in `edges`. The rescue by `access=delivery`,
`destination` or `agricultural` also stays as it is.

Other path classes (`footway`, `path`, `steps`, `cycleway`) do not change. They stay out of driving.

## Where the code changes

| File | Change |
|---|---|
| `processors/road_filter.py` | Driving keeps every `highway=pedestrian` way. The `access` expression gives the label above to the ways that are not rescued. |
| `importer.py` (`_split_private`) | Move the ways with `highway = 'pedestrian'` to `private_edges`, with the ways that have `access` `private` or `bus`. |
| `bus.py` | A bus route over a pedestrian street is a new case. Today `access` other than `bus` means a private road. |
| `levels.py`, `route_map.py`, `viz.py` | Check that the new rows draw with a path style, not a road style. |

These readers do not change, because they read only `access = 'bus'` or only `edges`: `edge_graph`,
`routing.py`, `sumo.py`, `gmns*.py`, `lane_profile.py`. A test must prove it.

## What does not change

- `driving.edges`, `edge_graph`, `route()`, every export and the component filter: the same rows and
  the same `edge_id` as today. Norrmalm: 2,293 edges.
- `private_edges` is not filtered by the component filter. A pedestrian street with no road
  at one end stays in the table.
- Walking and cycling.

## What changes

- `private_edges` has more rows (Norrmalm: 831 more edges; Monaco: 475 more).
- `driving.private_edges.access` has two new values: `pedestrian` and `delivery`. The value
  `delivery` is only for pedestrian ways in this table. A road that is not a pedestrian street and
  has `motor_vehicle=delivery` stays in `edges`.
- Docs: `reference/database.md`, `concepts/networks.md`, `design/access_private.md`.

## Open questions

1. **`delivery`.** Today `access=delivery` on a pedestrian way rescues it into `edges`. With this
   proposal, `motor_vehicle=delivery` puts the same kind of way in `private_edges`. The two tags
   have one meaning. Choose one table for both:
   - A. Both in `private_edges`, label `delivery`. A cars-allowed street leaves the routable network.
   - B. Both in `edges`, as `living_street`. The routable network gets 5 more ways in Norrmalm.
   - Proposed here: neither changes the routable network, so keep today's rule for `access=delivery`
     and use the label for `motor_vehicle=delivery`. This is the least change, and it is not
     consistent. The decision is yours.
2. **Label for a private pedestrian way.** `private` (the access tag decides) or `pedestrian` (the
   road class decides). Proposed here: `private`.

## Maps

The maps are a mapstyle change, not a roadstyle change. mapstyle already draws `private_edges` with
a Private roads row and a Bus lanes row ([private and bus](../../../mapstyle/docs/design/private_and_bus.md)).
The changes are in the mapstyle design note
[one_mode_map.md](../../../mapstyle/docs/design/one_mode_map.md):

- A Pedestrian streets row, and a colour for it.
- A driving map with only `driving.edges` and `driving.private_edges` (`--only`). `duckosm viz`
  then makes the per-mode pages with it.

## Related bug (separate fix, done 2026-10-19)

Six rescued pedestrian ways had no edge in the Norrmalm driving network: Olof Palmes Gata
(1279023715, 1279023721), Tegnérgatan (695998359, 695998360) and Wallingatan (129712683,
129712690). Cause: `main.global_junctions` treated `highway=pedestrian` as a non-road. A rescued
way is a road in driving, but its middle node was not a road junction. The segmenter found no
segment end, and dropped both ways.

Fix: `GlobalJunctions` counts a rescued pedestrian way (`road_filter._ped_ok`) as a road. A road
junction is added only where rescued pedestrian ways meet each other or one ends alone. A road
cannot be at such a node, or the node was a junction already. So no road is cut in a new place.

Measured (driving, the `edge_id`s before and after the fix):

| Area | Ids that go | Ids that come | Classes that change |
|---|---|---|---|
| Norrmalm | 6 | 35 | `living_street` (rescued pedestrian ways) only |
| Södermalm | 5 | 13 | `living_street` only |

The ids that go are the one-direction edges that the bug left. Each is replaced by a pair. The
county database is not measured.

## Test

- Norrmalm: the 125 ways are in `private_edges` with the right label. `driving.edges` has the same
  `edge_id`s as before. A pedestrian edge has the same `edge_id` in driving `private_edges` and in
  walking.
- The Monaco sample: `driving.edges` is unchanged.
- `bus.py`, `sumo.py` and the GMNS export give the same output as before on Monaco.
