# The walls of the buildings

duckOSM can give each wall of each building a type: `front`, `back`, `side`, `party` or `court`. The build writes the walls to the table
[`main.building_faces`](../reference/database.md#building_faces). The fixes editor shows them in colors, and you can change the type of a
wall there ([Edit OSM data](../guides/edit-osm-data.md)).

This page tells how the build finds the type of a wall. The method is a rule of thumb: it uses only the shape of the buildings, the streets
and a few OSM tags. Check its result by eye where the result is important to you.

## Turn it on

The walls need the Rust engine (the default), a PBF source and a boundary. The [config templates](../reference/configuration.md) turn them
on:

```yaml
options:
  building_faces: true
```

The sample `config/sample_monaco.yaml` has the option too. A build that cannot make the walls (`--engine python`, a clip from a parent
database, no boundary) writes a warning and makes no walls. A config file with no `building_faces` key makes no walls.

The step is fast: on Monaco it makes 10,909 walls of 1,262 buildings in about 0.2 s.

## The five types

| Type | Color in the editor | The wall |
|---|---|---|
| `front` | green | faces a street that it can see, or has an entrance. A corner building can have more than one front |
| `back` | red | points away from the front and faces no near street |
| `side` | grey | every other wall of the outer ring |
| `party` | purple | is shared with another building: the two buildings have the same two nodes |
| `court` | cyan | is a wall of a courtyard: an inner ring of a building that is a multipolygon relation |

A building with no street near it, no entrance and no street within 150 m has no front and no back: all its walls are `side` or `party`.

## What the build uses

- **The buildings.** A way with a `building` tag (not `building=no`) and a closed ring of three nodes or more. A multipolygon relation with
  a `building` tag: its member ways are joined into rings by their end nodes, and a chain that does not close is dropped. The outer rings
  get the same types as a way. The inner rings get `court` walls.
- **The streets.** The edges of the driving and walking networks of the build, but not `footway`, `path`, `steps`, `corridor`,
  `bridleway`, `cycleway` and `track`. A footpath along a street would else be the nearest "street" of a wall that faces the real street.
- **The entrances.** The nodes with an `entrance` tag, but not `entrance=no`, `service`, `emergency` or `exit`: these are not a way in
  for people.
- **The address.** The `addr:street` tag of the building.

## The method

### 1. The walls

Each segment of a ring is a wall. The key of a wall is the building and its two nodes, `(building, node, node)`. This key does not change
when the ring gets a node in another place. `face_index` (the place of the wall in the ring) can change. A wall shorter than 0.1 m has no
row.

The build uses one local metric scale for the whole area (the middle of the buildings). It turns each outer ring counter-clockwise, so the
normal of each wall points out of the building. An inner ring turns the other way, so the normal of a courtyard wall points into the
courtyard.

### 2. The street of each wall

For each wall, the candidates are the street segments within 60 m of the middle of the wall that the wall points at. `align` is the
cosine of the angle between the normal of the wall and the line to the nearest point of the street; a candidate must have `align` more
than 0. Each candidate gets a score:

```text
score = align / sqrt(distance + 1) × (1 + ln(length + 1)) × class weight × address
```

- `distance` is in meters from the middle of the wall. `length` is the length of the wall in meters: a long wall is more of a facade.
- **The class weight.** `motorway`, `trunk` and `primary` 1.3; `secondary` 1.25; `tertiary` 1.15; `service` 0.7; every other class 1.0.
  A `_link` has the weight of its class.
- **The address.** 1.8 when the name of the street is the `addr:street` of the building, else 1.

The street of the wall is the candidate with the best score that the wall can **see**. A wall sees a street when the line from one of
three points of the wall (at 50, 20 and 80 percent of its length) to the nearest point of the street crosses no wall of any building. Three
points let a wall see past a small building in front of its middle. When two streets have the same score, the lower `edge_id` wins.

### 3. The party walls

A wall is `party` when two rings have the same two nodes. A party wall is never a front, a back or a side.

### 4. The main front wall

The build looks for the main front wall in this order:

1. **A street.** The wall with the best street score among the walls that are not party walls and have `align` of 0.5 or more (the street
   is within 60 degrees of the normal). The reason is `address` when its street is the street of the address, else `street`.
2. **An entrance.** With no such wall: the first wall in the ring with an entrance node at one of its ends. An entrance in the middle of a
   straight wall splits it in two walls, so both get the entrance. The reason is `entrance`.
3. **The nearest street.** With no entrance either (a building behind other buildings): the wall that faces the nearest street within
   150 m with `align` of 0.5 or more, with no test of the line of sight. The reason is `inferred`.
4. **None.** The building has no front. Its walls are `side` with the reason `none`.

### 5. The runs

A person sees a curved facade or a facade with small steps as one wall. So the build joins walls into **runs**: consecutive walls of a
ring whose normals differ by at most 25 degrees, and that are not party walls. The first and the last wall of a ring can be in one run. A
run gets one type, and the street and the distance of its longest wall.

### 6. The type of each run

The rules run in this order. A run keeps the first type that it gets, except in rule 5.

1. **Fixed types.** A party run is `party` (reason `shared`). The run of the main front wall is `front`. A run with an entrance is `front`
   (reason `entrance`).
2. **More front runs (a corner).** A run is also a front when all of these are true:
    - it faces its street with `align` of 0.65 or more (about 50 degrees);
    - its street is at most 20 m away, and at most 8 m farther than the street of the main front;
    - its street has a class weight of at least 0.6 times the weight of the main front's street: a service lane does not give a front to
      a building that has a primary road on the other side;
    - its street is another street than the main front's street (another name; with no name, another way), or the run is on the same
      facade (its normal is within 25 degrees of the main front's normal).

    Of these runs, the two with the best street scores are `front` (reason `corner`). The others are `side`. A building on a corner of three
    streets can have three fronts.
3. **Back runs.** A run is `back` (reason `opposite`) when its normal points away from the main front (the cosine of the angle between the
   two normals is -0.4 or less, about 114 degrees or more), and it faces no near street: its `align` is 0.1 or less, or its street is
   more than 25 m away.
4. **Side runs.** Every other run is `side` (reason `other`; `none` for a building with no front).
5. **Short runs.** A run shorter than 3 m in all (a chamfer, a step) gets the type and the reason of its longer neighbour run. A fixed run
   (party, main front, entrance) does not change, and a party neighbour gives no type.

### 7. Your rules

Last, the build applies the wall rules of the fixes file (`extra.building_faces`, [Edit OSM data](../guides/edit-osm-data.md#extrabuilding_faces)).
A rule finds its wall by the building and the two nodes, in either order.

| Column | With a rule |
|---|---|
| `face_type` | the type of the rule |
| `computed_type` | the type of the method, steps 1 to 6 |
| `edited` | true when the rule gives another type than the method |
| `door` | the kind of door of the rule (`main`, `service`, …); NULL for no door |

A rule that gives the computed type, or only a door, does not make `edited` true.

## Why a wall has its type

The column `type_reason` tells which rule gave the type. The editor shows a note for `entrance`, `address`, `inferred` and `none`.

| `type_reason` | Type | The rule |
|---|---|---|
| `street` | `front` | the main front wall: the best street that the wall can see |
| `address` | `front` | the main front wall, and its street is the street of `addr:street` |
| `entrance` | `front` | an entrance node is on the wall |
| `corner` | `front` | a second or third front: another street at a corner, or the same facade |
| `inferred` | `front` | no street can be seen and no entrance: the nearest street within 150 m. Check it |
| `opposite` | `back` | it points away from the front and faces no near street |
| `shared` | `party` | another building has the same two nodes |
| `other` | `side` | a wall of a building that has a front |
| `none` | `side` | the building has no front |
| `inner` | `court` | a wall of a courtyard |

## The numbers

The numbers of the method are named constants at the top of `rust/duckosm-core/src/building_faces.rs`.

| Constant | Value | Use |
|---|---|---|
| `MAX_STREET_M` | 60 m | a street farther from the middle of a wall is not its street |
| `FRONT_ALIGN` | 0.5 (60°) | the main front wall faces its street at least this much |
| `CORNER_ALIGN` | 0.65 (about 50°) | a corner front faces its street at least this much |
| `CORNER_DIST_M`, `CORNER_EXTRA_M` | 20 m, 8 m | a corner front is near its street, and not much farther than the main front |
| `CORNER_CLASS` | 0.6 | a corner front's street has at least this part of the class weight of the main front's street |
| `FACADE_DOT`, `RUN_DOT` | 0.906 (25°) | the same facade; the walls of one run |
| `MAX_EXTRA_FRONTS` | 2 | the most corner fronts of one building |
| `MIN_RUN_M` | 3 m | a shorter run takes the type of its neighbour |
| `BACK_DOT`, `BACK_ALIGN`, `BACK_FAR_M` | -0.4, 0.1, 25 m | a back wall points away from the front and faces no near street |
| `INFER_STREET_M` | 150 m | the farthest street for an `inferred` front |
| `ADDRESS_BOOST` | 1.8 | the street of the address scores this much more |
| `MIN_FACE_M` | 0.1 m | a shorter wall has no row |

## The result on Monaco

The Monaco sample (`config/sample_monaco.yaml`), 10,909 walls of 1,262 buildings (1,231 ways and 31 relations):

| Type | Walls | Share | By reason |
|---|---|---|---|
| `side` | 4,632 | 42 % | `other` 4,602, `none` 30 |
| `front` | 3,134 | 29 % | `street` 1,478, `corner` 1,136, `address` 320, `entrance` 116, `inferred` 84 |
| `back` | 1,614 | 15 % | `opposite` 1,614 |
| `party` | 1,130 | 10 % | `shared` 1,130 |
| `court` | 399 | 4 % | `inner` 399 |

5 buildings have no front, and 509 buildings have no back wall. A building in a row between two streets has two fronts and no back,
which is correct.

## The same rows each time

The rows come in the order of the ways and of each ring. A tie between two streets goes to the lower `edge_id`. So two builds of the same
data give the same rows.

## Limits

- No reference data tells which wall is the real front. The numbers above show a pattern, not the truth.
- The method does not know the blocks of the city. A building that sees no street can get a street across another block (`inferred`).
- At most two corner fronts: a building with four streets around it has at most three fronts from the streets. A wall with an entrance
  is a front too.
- An entrance tag on a node of the wall gives a front, also when the entrance is at the back of the building.

## Proposal: party walls of different heights

Date: 2026-11-10. Status: proposal, not built. Nothing above this heading changes until Kaveh agrees.

### The fault

Step 3 marks a wall `party` when two rings have the same two nodes. It does not use heights. The 3D walls of
[duckOSM Studio](https://github.com/Khoshkhah/duckosm-studio) draw each wall from the ground to the roof in one color. So a tower that
stands on a podium shows its whole facade in the `party` color, also above the roof of the podium.

Data of Burnaby (`pbf/burnaby.complete_ways.osm.pbf`, 6,066 `party` walls), heights from the `height` and `building:levels` tags at 3 m for each level:

| Case | Count | Example |
|---|---|---|
| A tower beside a lower building: tower 40 m or more, with a party wall | 338 wall pairs. In 239 the other building is less than half as high. In 75 it has no height tag | Station Square (Willingdon Avenue): towers of 41 levels (123 m) and a podium of 3 levels (9 m, relation 16733309) share 120 m of wall |
| Two rings with the same footprint | 8 pairs | Way 962138234 (`Station Square Tower 4`) and relation 16733312 have the same 12 nodes. Each wall of both is `party` with the other |
| A part of a building (`building:part`) | 2 of 41,548 | Not a problem here: a way with `building:part` and no `building` tag is no ring |

Of all the party wall pairs, 484 have a height difference of more than 15 m, 60 have 3 to 15 m, and 1,146 have less than 3 m. In 4,372 pairs at least one
building has no height tag.

### The change

1. **Same footprint, no party wall.** Two rings whose walls are all the same two nodes are one footprint that is mapped twice. The build gives
   neither ring a party wall. It also leaves each ring out of the line-of-sight test of the other, so that the walls keep the type that they
   would have with one ring.
2. **A new column `open_type`.** It tells the type of the wall if no building stood beside it. For a `party` wall, the build runs steps 2 to 6
   a second time for the buildings that have party walls. In this run, the walls of the neighbour buildings are not shared and are not in the
   line-of-sight test. For every other wall, `open_type` is `computed_type`.
3. **No height in the table.** The height of a building depends on the look: `m_per_level` and the heights of the kinds are values that a
   person changes in the design panel. The engine has no look. So the page, not the build, finds the shared part.

The page finds the neighbour of a wall in `building_faces` itself: it is the other row with the same two nodes. The shared height is the lower
of the heights of the two buildings, by the rule of the tiles. [roadstyle draws the two parts](https://github.com/Khoshkhah/roadstyle/blob/claude/party-wall-heights/docs/design/party_wall_split.md).

### The effect on the table

| Column | Change |
|---|---|
| `open_type` (new, VARCHAR) | the type with no neighbour; equal to `computed_type` for a wall that is not `party` |
| `face_type`, `computed_type`, `type_reason` | no change, except for the twin rings of item 1 |

A new column changes the result of the build. The tests that compare the columns of `building_faces` (Monaco, Tartu) need the new column, and
Kaveh must agree to that before the code changes.

### The limits

- The type above the shared part comes from a 2D method. A tower wall above a podium can face a street that the podium hides in the
  line-of-sight test. Item 2 removes the neighbour from the test, but not a third building.
- A wall with several neighbours takes the lowest one.
- A building with no height tag has the height of its kind, as in the tiles. The shared part is then a guess.
