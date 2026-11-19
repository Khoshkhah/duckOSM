# Routing details

The details that the manual page [Route](../guides/route.md) does not show.

## Point routing

| Rule | Result |
|---|---|
| Roads that a point can join | every road of the mode within `radius_m`, both directions of a two-way street. The trip that is fastest wins |
| The walk to the road | a straight line at `access_kmh`, for every mode. It does not cross another road: for cars, it joins the first road in its way |
| The first and the last edge | only the part that the trip uses: a start in the middle of a 300 m street costs 150 m |
| No road within the radius | `ValueError` ("no road within 50 m of the start") |

The result also has `start` and `end` (`point`, `road_point`, `edge_id`, `fraction`, `access_m`, `access_s`), and
`path[0]["from_fraction"]` (where on the first edge the trip starts, 0 to 1). Design:
[point routing](point_routing.md).

Example on Monaco, Fontvieille to Larvotto: `route` by car gives 287 s and 3.7 km.

## Directions

Each step has `text`, `length_m` (the distance to the next step), `type` (`depart`, `turn`, `fork`, `new name`,
`roundabout`, `mode`, `arrive`), `modifier` (`left`, `slight right`, `uturn`, ...), `name`, `mode`, `at` (the
`(lon, lat)` of the step) and, for a roundabout, `exit`. The rules are those of the OSRM model. The route map uses the
same rules in the page.

## Route map

A marker joins the roads within the off-road distance by a walk, as `route_points` does. The Roads box hides road classes,
bridges, tunnels, private roads or bus lanes. One difference with the command is possible: when two routes have the same
time and length, the page and the command can select different roads. The database gives the roads near a point in the
order of its spatial index, and the page in the order of the table.

## Large areas

The graph of one mode is in memory, with 12 bytes for each turn. A trip across modes reads only the modes of `--modes`.

Measured on a Mac mini (Apple M6, 12 cores, 16 GB):

| Area | Step | Time | Peak memory |
|---|---|---|---|
| France, driving (25.9 million edges, 62.5 million turns) | build by tiles (`duckosm tiles`) | 204 s | 1.6 GB |
| | load the driving graph | 11 to 16 s | 6.4 GB (load and route) |
| | `route` Paris to Marseille (773 km, 6.5 h, 1,427 edges) | 3.7 s | |
| | `route-points` Paris to Marseille (reads all weights and turns for each call) | 33 s | 5.4 GB |
| Île-de-France, three modes (1.8, 4.1 and 4.0 million edges) | build (`duckosm`, GeoParquet) | 55 s | 3.0 GB |
| | `route-points` by bike across Paris (Porte Maillot to Nation, 9.3 km) | 3.5 s | 1.3 GB |
| | `route-multimodal-points` Versailles to Bastille (walk, drive, walk; 24.3 km, 35 min), `--modes walking,driving` | 4.5 s load, 4.4 s trip | 2.6 GB |
| | the same trip with all modes in memory | 8.0 s load, 4.9 s trip | 4.4 GB |

The France rows have no walking and cycling network: on that computer, the disk did not have the space for the three modes
of France with 30 GB kept free.
