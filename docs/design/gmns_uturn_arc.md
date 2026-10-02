# GMNS: a U-turn is a half-circle as wide as its lanes

**Status:** approved by Kaveh after he saw the before / after pictures, built 2026-10-02 (`_build_lane_connectors`). Branch `gmns-values`. Monaco only.

**Result (Monaco):** of the 234 U-turn connectors 199 are now as wide as their lanes (before: none, median 60 %), 35 stay narrower (their lane ends are less than a lane width apart, or the lanes do not face opposite ways). No other connector and no lane changed (0 of 1,579 and 0 of 3,528); 1,813 connectors in all, as before. `duckosm gmns` driving + walking 46 s (44 s before).

## Problem

At the end of a two-way road (Boulevard Rainier III, Monaco: `4030367642102153402_1` in, `6871880265432049446_1` out, node 6363741444) the two lane
ends are 3.0 m apart, one lane width, and the connector between them is the U-turn. Today it is drawn as a 1.8 m wide band while the lanes are 3.0 m:
its outer arc (radius 2.4 m) lies inside the lanes' outer edges (which are 6.0 m apart, so the end wants an arc of radius 3.0 m), and its inner
side (radius 0.6 m) leaves a notch. The map shows a pinched end and a small hook (Kaveh, 2026-10-02, "the U-turn hook").

Why (`_build_lane_connectors`): every connector is a cubic Bézier whose handles are 0.4 × the chord along the two lane tangents. For a U-turn the
chord is the lane spacing (3.0 m) and the tangents are opposite, so the curve is tighter than a half-circle at its apex; `_fit_width` then
narrows the band to 1.8 × the tightest radius, never under 60 % of the lane. Monaco's 234 U-turn connectors are all narrowed (median 60 % of the
lane width, the floor). The open item of `gmns_lane_connectors.md`: "a half-circle curve for U-turns is not built".

## Proposal

For a movement of type `uturn` whose two lane tangents are opposite (dot product below -0.9) and whose chord (the distance between the two lane ends)
is at least 0.5 m:

- **Shape:** a half-circle through the two lane ends, bulging forward from the inbound lane (a cubic with handles 2/3 × the chord, which is within 3 % of a circle),
  not the 0.4 × chord handles.
- **Width:** the lanes' width, but never more than the chord (so the inner radius stays at or above 0). For a chord of one lane width the band is a
  full half-disc: its outer edge is the arc that joins the two lanes' outer edges. `_fit_width` is not applied.
- Every other movement, and a U-turn whose tangents are not opposite, is built as now.

Monaco: 234 U-turn connectors; 147 have a chord of at least their lane width (full width), 87 a smaller chord (width = chord, a neck).

## What lanestyle changes

Nothing. It draws the connector at its `width`, and the outline near a connector follows the surface the map draws (the casing rule of 2026-10-02).

## Checks (Monaco)

- Tests: a U-turn between two lane ends one lane width apart gets a half-circle of that width (every point of the centre line 1.5 m from the midpoint
  of the ends, within 0.1 m; width 3.0); a chord smaller than the lane width gets width = chord; a left or right turn is unchanged.
- Monaco: the widths and the shape of the 234; the Boulevard Rainier III spot and two other U-turns on the previews page, before and after.
- `duckosm gmns` build time and the full test suite unchanged.

## Not in this note

- A dead end where three connectors leave one point (Monaco `156780348#1f`): still open.
- Which lane may U-turn (OSM lane tags): unchanged.
