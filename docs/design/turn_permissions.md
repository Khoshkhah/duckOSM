# Turn restrictions with exceptions, conditions and vehicle classes

**Status:** implemented 2026-10-09, as agreed with Kaveh the same day ("go ahead"). Build: `processors/restrictions.py`,
`processors/edge_graph.py`; exports: `sumo.py`, `gmns.py`; copied by `extract.py` and the clipper.

## The problem

An OSM turn restriction is more than `restriction=no_left_turn`. It can say who is exempt, when it applies, and which vehicles it
binds:

| tag | meaning | example |
|---|---|---|
| `restriction=*` | the turn is banned (or mandated: `only_*`) for every vehicle, always | `no_left_turn` |
| `except=*` | ... except these vehicles (`;`-separated) | `except=psv`: buses and taxis may still turn |
| `restriction:conditional=* @ (...)` | banned only when the condition holds | `no_left_turn @ (Mo-Su 07:00-19:00)` |
| `restriction:<vehicle>=*` | banned only for that vehicle class | `restriction:hgv=no_left_turn` |

duckOSM reads only `restriction`. So:

- `except` is ignored: the turn is removed for the exempt vehicles too. Granville St x W Broadway, Vancouver (relation 6965100,
  `no_left_turn`, `except=psv`): the bus left turn from northbound Granville onto West Broadway is gone.
- `restriction:conditional` without a plain `restriction` is skipped: the ban is never applied. Relation 6956554 at the same
  junction (no left turn 07:00-19:00 except buses) does nothing.
- `restriction:<vehicle>` alone is skipped the same way.

How often: Vancouver 2,538 restrictions, 141 with `except` (111 bicycle, 38 psv, 9 bus, 3 bus;bicycle), 174 conditional, 1 hgv;
Södermalm 41, 6 with `except`; Monaco 43, none.

## Rule

1. **`<mode>.turn_restrictions` keeps the whole rule.** New columns: `except_vehicles` (the `except` list, or NULL),
   `applies_to` (the vehicle of `restriction:<vehicle>`, or NULL: all), `condition` (the text inside `@ (...)`, or NULL). One row
   per rule: a relation with `restriction`, `restriction:conditional` and `restriction:hgv` gives three rows.
2. **`<mode>.edge_graph` does not change meaning:** the turns open to all traffic, as today. Only an unconditional rule for all
   vehicles changes it (as today); routing, map matching and every downstream project see the same graph as before.
3. **New `<mode>.turn_permission`** (`from_edge`, `to_edge`, `allowed`, `vehicles`, `except_vehicles`, `condition`,
   `restriction_id`): what `edge_graph` does not say.
   - an `except` on an unconditional rule: the turn is not in `edge_graph`, but `allowed = true` for `vehicles` (the bus left
     turn above);
   - a conditional rule: the turn stays in `edge_graph` (open outside the condition), `allowed = false` for `vehicles` (NULL: all)
     but `except_vehicles`, while `condition` holds;
   - a rule for one vehicle class: the turn stays in `edge_graph`, `allowed = false` for that class.
   A `turn_restrictions` table from before these columns is read as: every rule binds all vehicles, always.
   `only_*` rules work the same way on every other turn out of the `from` edge.
4. **SUMO** (`to_sumo`): an `allowed` turn is written as a connection with SUMO's `allow` (OSM `psv` -> `bus coach taxi`, `bus` ->
   `bus coach`, `bicycle` -> `bicycle`, `taxi` -> `taxi`, `hgv` -> `truck trailer`, `emergency` -> `emergency`); a banned class is a
   `disallow` on the connection. netconvert takes a permission only on a lane-to-lane connection, and once one move of an edge is
   given by lane it works out no other move of that edge; so a last pass reads the lanes netconvert chose for every move of an edge
   with a permission and writes them all back by lane, the permission on its own. SUMO has no time-dependent turns: a conditional ban is written in force (the most restrictive
   network); its time stays in `turn_permission`. When junctions are joined, a path through a swallowed edge takes the
   permissions of all its steps (the vehicles allowed on every step, banned on any).
5. **GMNS** (`gmns.py`): an `allowed` turn becomes a `movement` with `allowed_uses` = its uses (`bus` for psv / bus, `bike`). A
   vehicle-class or conditional ban is not in GMNS yet (it needs the time-of-day tables).
6. **Extract and clip** copy `turn_permission` and `turn_path_restrictions` with the edges they name, like `turn_restrictions`.
7. **Restrictions with a via way** ban (or mandate) a path: from way, along the via way(s), onto the to way. Single turns in
   `edge_graph` cannot say that without banning legal moves too (the via way is used by other paths), so `edge_graph` keeps them
   out and `<mode>.turn_path_restrictions` holds one row per edge path (`from_edge`, `via_edges`, `to_edge`) with the same
   `except_vehicles`, `applies_to`, `condition`. A rule may name several `to` ways (a `no_u_turn` naming both carriageways): a path onto
   any of them counts. The SUMO export applies a path rule where the via edges vanish into a joined
   junction, the path being one connection there (`out["n_path_restrictions"]`, logged as "n of m applied"); elsewhere SUMO cannot
   say it. GMNS cannot either (a movement is one turn at one node).

## Checks

- A test per case: an `except` turn is in `turn_permission` and not in `edge_graph`; a conditional or vehicle-only rule leaves
  `edge_graph` as it is and adds a `turn_permission` row; the SUMO network has the bus connection with `allow`.
- Granville x Broadway after a Vancouver rebuild: the left turn from Granville onto West Broadway exists for buses only.
- Every area's build report lists the restrictions by kind and how many matched edges.

## Results (2026-10-09)

Vancouver: `edge_graph` identical before and after (225,239 pairs, none different); `turn_restrictions` 2,104 plain, 125 with
`except`, 165 conditional, 1 for one class; `turn_permission` 142 turns open only to some vehicles, 168 closed only to some or at some
times; 45 of 53 via-way restrictions matched edge paths, the other 8 name a way outside the city extract (`edge_graph` still
identical). Granville St x W Broadway: the left turn
from northbound Granville onto West Broadway is open to buses and taxis only (relation 6965100); from southbound Granville onto
West Broadway east, buses only (relation 6956554, via the link way, 07:00-19:00, written in force in SUMO).

## Not now

Routing that honours via-way restrictions (it needs paths, not pairs: a node split per restricted path), and via-way restrictions
in GMNS.


Routing by vehicle class (a bus router reading `turn_permission`), evaluating `condition` at a given time, and GMNS time-of-day
tables.
