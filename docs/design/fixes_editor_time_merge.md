# Fixes editor: the time of each change, and Add a file

Status: done (2026-11-06). Kaveh asked for two things in the fixes editor (`duckosm edit`, `web/fixes_editor/`):

1. Each change has a date and a time.
2. You can add the changes of another fixes file to the changes that you have.

## 1. The time of each change

### In the file

Each record that the editor writes gets the key `changed`: the local date and time, with the offset from UTC, to the second.

```yaml
overrides:
  - osm_id: 503462470
    lanes: 2
    changed: "2026-11-06T14:32:05+01:00"
extra:
  tag_edits:
    - type: way
      id: 503462470
      set: {maxspeed: "30"}
      changed: "2026-11-06T14:32:05+01:00"
```

- The five lists get the key: `overrides`, `new_ways`, `turn_restrictions`, `extra.tag_edits` and `extra.building_faces`.
- The build does not read `changed`. The Rust reader (`rust/duckosm-build/src/config.rs`) refuses an unknown key, so `changed` goes into
  its five lists of keys. The Python reader ignores keys that it does not use, so it needs no change.
- The value is a quoted text. js-yaml quotes it, so no reader makes a date object of it.
- A record with no `changed` (an old file, or a file that a person wrote) gets the time of now when the editor reads the file
  (`F.stampMissing`, Kaveh's decision on 2026-11-06). The page then saves the file. A record of an added file gets the same treatment.

### When the time changes

The editor sets `changed` in one place: `commit()`, after each change. It compares each record with the records before the change. A record
that is new or different (without `changed` in the comparison) gets the time of now. A record that did not change keeps its time.

- An edit of a road changes its `overrides` rule and its tag edit: both get the same time.
- Undo and Redo bring back the records with their old times.
- A join of two records of one object ("2 rules", then Edit) is a change: the joined record gets the time of now.

### In the page

- The Changes list shows the time of each item under its name, for example `6 Nov 2026, 14:32`. An item with more than one record shows the
  latest time.
- The box "This road is in your changes" (Road, Building, Node tabs) shows the time too.
- The list keeps the order of the file. A sort by time is not part of this change.

## 2. Add a file

### In the page

The Changes tab gets the button **Add a file** in the row of Undo, Redo and Download. It opens the file dialog of the browser (`.yaml`, `.yml`).

1. The editor reads the file with the same reader as the fixes file (`F.parse`). If the file is not valid YAML, or has a key that a fixes
   file cannot have, the editor shows the error and changes nothing.
2. The editor adds the records of the file to the changes (the merge below).
3. A status line tells the result, for example: `Added 12 changes from other.yaml. 3 objects were in both files: the newer change stays.
   1 new road got a new ID.`
4. The changes are saved in the fixes file at once, as each change is. **Undo** takes back all of the added file in one step.

### The merge

The editor adds each record of the other file to the end of its list, with these rules:

- **The same object in both files** (the same way, node or relation; the same wall; the same turn; the same new road): the records with
  the later `changed` stay, and the others go. The `overrides` rule and the tag edit of a way are one object.
- **The same object, and no time tells which is newer** (the same time; or no time on one side, which the editor does not make): the
  two records stay, the added one last. The build applies them in file order, so the added record wins key by key. The Changes list shows "2 rules" on the item,
  as it does now for such a file.
- **A new road with the same ID as a new road here, and other nodes:** this is another road. It gets the next free ID (from 9,000,000,000,000
  up), and the turn rules and the tag edits of the added file that use the old ID get the new ID too.
- **A new road with the same ID and the same nodes:** this is the same road. The first rule applies (the newer record stays).
- The added records keep their `changed` times. The merge sets the time of now only on a record with no time.
- An object that a road of the other file deletes (`remove`) is a change like the others: the newer record stays.

### Out of scope

- `duckosm build` with two `--fixes` files. Add it later if you need to build from two files with no editor.
- A choice, object by object, of which record stays.

## Tests

- `web/fixes_editor/fixes_units.mjs`: the time of a new, a changed and an unchanged record; Undo; the merge rules above (the newer record, no
  time, a new road with the same ID and other nodes, a file that is not valid).
- `tests/test_editor_changes.py`: a fixes file with `changed` in each list builds in the two engines.
- `rust/duckosm-build/src/config.rs`: a test that the five lists accept `changed`.
- `tests/test_edit_page.py`: Add a file in the page, with the status line and Undo.

## Docs

- `docs/guides/edit-osm-data.md`: the time in the Changes list and the key `changed`; the Add a file button and the merge rules.
- `docs/reference/` (the fixes file keys): the key `changed`.
