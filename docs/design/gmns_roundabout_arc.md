# A roundabout's lanes follow its circle

**Status:** implemented 2026-10-04 (Kaveh: "fix the duckOSM part", after the roundabout of lane `8630696718605506421_1` in Monaco looked angular).

## The problem

OSM draws a roundabout as a polyline with a vertex every few metres. Monaco's ring at lon 7.4139, lat 43.7273 has a radius of 10.1 m and segments of 2.5 m: 14 degrees per segment. All the vertices lie on the circle
(0.01 m), but the lanes are offset from the polyline, so they inherit its corners. Drawn at lane width the ring is visibly a polygon, with a corner every 2.5 m on the centre line.

## The rule

`_run_lane_wkts` already offsets a **closed run** (a roundabout, possibly several ways and edges) as one ring. Before the offset, the ring's vertices are fitted with a circle (least squares, in the local metre frame). If the fit is good, the ring's line is replaced by the points of the circle: the
original vertices are kept, and between two vertices points are inserted on the circle every 3 degrees at most. The lanes are offset from that line, then cut back into the pieces at the joints as before.

A fit is good when the ring has a radius of at least 3 m, no vertex is farther than 0.25 m or 4 % of the radius from the circle (whichever is larger), and the ring goes once round it in steps of at most 60 degrees (a ring of fewer than six vertices could be a polygon on purpose, a square for instance). An oval, a ring with a bulge, or a ring that is not closed keeps its polyline: nothing is guessed.

## What does not change

- The edges and their `edge_id`s (a content hash of the geometry), `link.geometry`, lengths and costs: only the **lane** lines of a roundabout (`gmns_*.lane`) change, by at most the distance of the ring's chords from its circle (0.08 m for Monaco's ring).
- Open runs, other bends, connectors and every other mode.

## Not done

A curve that is not a roundabout (a slip road, a bend) keeps its sparse vertices. The same fit could be used for it with an arc instead of a circle; it needs its own decision, because most such bends are not circular.
