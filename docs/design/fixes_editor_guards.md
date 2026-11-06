# The fixes editor: review, and guards for the forms

**Status:** approved by Kaveh 2026-11-06 ("All five parts").

## Problem

Kaveh asked for a review of the edit page (`duckosm edit`, the panel "Edit OSM data" of `web/fixes_editor/`). The page must be easier
to use. A dropdown list or a field must accept only the values that agree with the object and with the choices before it.

The review used the page on a Monaco build (the six tabs, with an object picked in each) and the code of `fixes_panel.js` and
`presets.json`. The panel has a guard system already (`presets.json`, `_guards`: a choice closes a field or makes its list shorter).
The findings below are the gaps.

## Findings

### A. Edits are lost with no warning

| # | Where | What occurs |
|---|---|---|
| A1 | Road, Building, Node | You change fields, then you click another object on the map. The fields that you did not add are lost. No message tells you. (`select()` clears the draft. Only **Unselect** tells you after the loss.) |
| A2 | Turn | You choose rules, then you pick another junction or another road. The rules that you did not add are lost. |
| A3 | New road | You click 10 points, then you push **Esc** or **Cancel**. The points are lost. |
| A4 | All | You close or load the page again while a tab has edits that are not added. The edits are lost. |

### B. Labels and fields that are not clear

| # | Where | What occurs |
|---|---|---|
| B1 | Road | **Lanes, all** is the rule `lanes`: the lanes in each direction. A two-way road with 2 lanes in OSM shows `1`. In the New road tab, **Lanes** is the OSM tag: the lanes of all directions. The same word has two meanings. |
| B2 | Road | The road class (`highway`) is a free text in **Other tags**. You can type `seconday`. The guards of the Road tab read the road class, but you cannot choose it from a list. |
| B3 | Node | The head says "Node node 21912089". |
| B4 | Road | The three check boxes of **Modes** have no space between the box and the word. |

### C. The check of a value occurs too late

| # | What occurs |
|---|---|
| C1 | The panel checks a value only when you push **Add to changes**. The error shows for some seconds in the line under the tabs, not at the field. |
| C2 | The panel has no check for: lanes 0 or less; lanes forward or lanes back that is different from the lanes in each direction; the format of max speed (`50`, `30 mph`, `walk`, `none`, `SE:urban`); the format of turn lanes (`left;through|right`) and its number of lanes; the format of height (`12`, `12 m`, `40 ft`); levels less than 0; a color that is not `#rrggbb` or a color name; layer out of -5 to 5; a key in **Other tags** with a space in it. |

### D. Guards between the fields of one object

| # | What occurs |
|---|---|
| D1 | A guard closes a field, but the old value of the field stays, and **Add to changes** keeps it. Example: a crossing becomes a traffic signal, and `crossing=marked` stays. |
| D2 | Bridge and layer do not agree: a bridge with layer 0 or -1, a tunnel with layer 1. |
| D3 | A road that is in no mode (all three boxes) stays in the data but is in no network. **Delete** is the correct tool for this. |
| D4 | Node: the check at **Add to changes** applies only to a traffic signal and a crossing, not to stop, give way and bus stop. (The list closes these options, but a value from the data is not checked.) |

### E. Guards between objects

| # | Where | What occurs |
|---|---|---|
| E1 | Turn | You can give two **Only …** rules at one junction (only left to road A, only right to road B). They contradict. You can also give **No …** to every road that leaves, and the junction becomes a dead end. |
| E2 | Road | You delete a road that turn rules or new roads use. The rules then do nothing, and no message tells you. |
| E3 | Building | You delete a building that has wall rules. The wall rules then do nothing. |
| E4 | New road | A new road that joins no junction is an island. The build keeps only the largest part of the network, so the build removes the road. Two points on the same junction one after the other make an edge of length 0. |
| E5 | Road, Building, Node | You type an ID that is not in the area. **Add to changes** adds a rule for it with no question. If the ID is a building and you are in the Road tab, the panel does not tell you to use the Building tab. |

## The proposal

### 1. Ask before a loss (A1 to A4)

- A1, A2, A3: before the panel drops edits that are not added, it asks with the browser's dialog: "2 fields of road 503462470 are not added. Drop them?" **Cancel** keeps the object and the edits.
- A4: the page asks before it closes (`beforeunload`) while a tab has edits that are not added.

### 2. Check each value while you type (C1, C2, D4)

- Each field gets an optional `check` in `presets.json`: `min` and `max` for a whole number, or `format` (`maxspeed`, `turn_lanes`,
  `height`, `color`). The rules stay data, as the guards are.
- The panel checks the form at each input. A field with an error gets a red line and the reason under it. **Add to changes** is off, and
  the bar says why ("Max speed: type a number, a number with mph, walk or none.").
- The checks that use two fields are in the code. Lanes forward or lanes back that is different from the lanes in each direction is a
  warning: the rule `lanes` sets the two directions, and `lanes_forward` and `lanes_backward` then replace it. The number of lanes in turn
  lanes that is different from the lanes of that direction is a warning too. the node kind and the junction (`nodeRule`, for all kinds).
- A value from the data that fails a check is not blocked if you do not change it. The panel shows it as a warning only.
- The checks are an exported function with no DOM. `fixes_units.mjs` tests it in Node.

### 3. Guards that clear a value (D1, D2, D3)

- When you change a choice and a guard then closes a field that has a value, the panel clears that value and says so under the field
  ("Cleared: a traffic signal with a crossing has crossing=traffic_signals."). **Reset** brings it back.
- When the data has the conflict already (you did not cause it), the panel shows a warning and does not change the value.
- D2 and D3 are warnings, not blocks: OSM can have a bridge with layer 0.

### 4. Clear labels (B1 to B4)

- Road: **Lanes in each direction**, **Lanes forward**, **Lanes back**. New road: **Lanes (all directions)**.
- Road: a **Road class** list in Basics (the classes of duckOSM's road filter), with the guards. A class that is not in the list stays as
  it is.
- B3 and B4: fix the head and the check boxes. The box **Modes** becomes **Networks**, with the text "Keep this road out of:". The
  check boxes use roadstyle's option rows (`rs-opt`), which are in roadstyle `origin/main` (the page now pins it).

### 5. Guards between objects (E1 to E5)

- E1: when a road gets **Only …**, the lists of the other roads at that junction close, with the reason "Only left turn: no other turn is
  possible." If every road gets **No …**, the bar shows a warning.
- E2, E3: the Delete section lists the rules that use the object ("2 turn rules use this road"), with a button that deletes them too.
- E4: **Add to changes** is off until one point joins a junction. The panel does not add a point on the same junction as the point
  before it.
- E5: an ID that is not found: the panel looks in the other kinds and offers the correct tab ("This is building 23715051. Open it in
  the Building tab."). An ID that is not found in any kind needs a second push of **Add to changes**.

## What changes

| File | Change |
|---|---|
| `web/fixes_editor/fixes_panel.js` | the checks, the dialogs, the clear of a closed field, the turn guard, the links between objects |
| `web/fixes_editor/presets.json` | `check` on the fields, the Road class field, the new labels |
| `web/fixes_editor/fixes_panel.css` | the red line and the error text, the check boxes of Modes |
| `web/fixes_editor/fixes_units.mjs` | the tests of the checks |
| `docs/guides/edit-osm-data.md` | the checks and the warnings |

The fixes file does not change. `fixes.js` does not change.

## Not in this proposal

- A check of the tags against the OSM wiki or a full tag schema (as iD's presets). The list in `presets.json` is enough.
- New doors or new nodes. The fixes file cannot add a node.

## What was done

All five parts, in `fixes_panel.js`, `presets.json`, `fixes_panel.css`, `fixes_units.mjs` and the guide `docs/guides/edit-osm-data.md`.
Changes from the proposal:

- **The box Modes is gone** (Kaveh: the editor is for OSM data, and `exclude_modes` is not OSM data). The Road tab has an **Access**
  section with the OSM tags `access`, `motor_vehicle` (Cars), `foot` (Walking) and `bicycle` (Bikes). Both engines remove a way from a mode
  when its access tag for that mode is `no` (`FORBIDDEN` in `road_filter.py`, `filter.rs`). A rule `exclude_modes` of an older file stays in
  the file, and the form shows it with the button **Remove this rule**.
- **The turn list keeps the choices.** Before this change, the list of turns was drawn again from the changes when you went to another tab
  and back, and the rules that you chose were lost. Now the list keeps them while it shows the same road and junction.
- **A guard reason shows once**, under the first field that it closes. The other fields have it as a tooltip.
- The pure checks (`checkValue`, `checkKey`, `crossChecks`, `clearClosed`, `guardsFor`, `nodeRule`) are exports of `fixes_panel.js` with no
  DOM, and `fixes_units.mjs` tests them.


## The fixes file in the build, and the walls (2026-11-06)

Kaveh asked for three checks after the guards: the latest design, the fixes file in `duckosm build`, and the walls in the pipeline.

- **The fixes file in the build.** The editor made one change of each kind through its forms (`duckosm edit` on the Monaco sample).
  Its file is `tests/data/editor_changes/osm_fixes.yaml`. `tests/test_editor_changes.py` builds Monaco with it in the two engines and
  checks the result of each change: 21 changes in both engines, and the wall rule in the Rust engine. The guide has the section "What each
  change writes", with the records and the results.
- **The walls.** `main.building_faces` is part of `duckosm build`, behind `options.building_faces` (default off, Rust engine). The test
  server of the review had no walls because the Monaco sample config does not set the option. Now a build with wall rules and the option off
  writes a warning, and the editor's Walls section names the option.
- **The design.** The review of every tab found four faults, now corrected: the guard reason did not show in the Node tab (the code read it
  two times), a button that cannot act looked the same as one that can, the walls check box had no space (the host page's `#fix-panel label`
  reset and roadstyle's `#sb label` rule won over the option row), and the New road tab said Kind for the road class. One more fault came
  from the test: a choice in the New road form while the Name field had the focus drew the form two times, one in the other.

## No shape on the map, and roadstyle's latest parts (2026-11-06)

- **No shape on the map in `duckosm edit`.** `ST_AsGeoJSON` gives a JSON value. duckdb-wasm (Studio) gives it as a text, and the server of
  `duckosm edit` (rows as JSON) gives it as an object. The panel parsed only a text and dropped the shape with no message. So the walls had no
  colors, a pick was not blue, the changes were not amber, and a pick from the Changes list did not zoom. Now `geoOf` reads both. A pick
  from the Changes list opens the tab of the change and zooms to it; a wall change also selects its wall.
- **roadstyle `origin/main`.** `rust/duckosm-edit/Cargo.toml` pinned roadstyle `0689bc2`, 55 commits before `origin/main` (`ac08e95`). The
  page now pins `origin/main`: every control of the panel has a hover, and the option rows (`rs-opt`) are roadstyle's. The option-row
  fallback of `fixes_panel.css` is gone, and the host page's `#fix-panel label` reset skips the option rows.

### Parts to ask roadstyle for

Kaveh's rule (2026-11-06): when the editor needs a part that roadstyle's dashboard does not have, ask roadstyle to add it first. These
are the parts that `fixes_panel.css` still draws itself:

| Part | Where the editor uses it | In roadstyle |
|---|---|---|
| A select box with a border in the editor frame | every list of the forms | the border is only in a card (`.sb-sec[data-rs-card]`) |
| A grey disabled button in the frame | Add to changes, Build again, Undo | only the toolbar and the map buttons |
| A section box in the frame | Pick, Selected, each group of fields | `rs-sec-box` has its look only in a card |
| A field error and a field warning (a line under the field, a red edge) | the checks of the forms | no |
| A row that is a pick on the map: hover, selected, with a select box in it | walls, doors, turn roads, the Changes list | list rows (`rs-qrow`) have no selected state and no control |
| A count badge and a "not saved" dot on a tab | the tabs | planned |
| A confirm dialog | "Drop them?" before a loss (now the browser's `confirm`) | planned |
| A status line | "Saved in osm_fixes.yaml", the cleared fields | planned |
| A busy button | Build again while it builds | planned |

Parts that roadstyle has, and that the editor can use in place of its own CSS: a callout (`rs-callout`) for the box "This road is in your
changes" and the box "Not added to the changes yet", a dot (`rs-dot`) for the legends, and rows of fields (`rs-kv`) for "All tags in the
data" and the changes of an item.
