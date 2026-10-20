# The drawing order is always in the database

**Status:** proposal (2026-10-20). Not approved. No code is written.

## Goal

One duckOSM database per area holds everything a map needs. A project that draws a map reads
`visualization.edge_levels` and finds it **current**: made for the roads of this file, with
your edits. No one runs a second command, and a rebuild does not lose anything.

## Today

- `duckosm build` does not make `visualization.edge_levels`. You run `duckosm levels DB`.
- Your edits (`level_edits`, `level_heads`, `level_caps`) are tables in the file, but the master
  is the folder `DB.levels/`. See [the drawing order](../guides/drawing-order.md).
- A rebuild deletes the file (`importer._connect`). The tables go with it. Only the folder keeps
  your edits.
- A map of a file with no `edge_levels` has the order computed by roadstyle, without your edits.

## Rule

1. **The build makes `visualization.edge_levels` as its last step** (after the other tables, before
   the report and the maps, so the maps use it). It is the same work as `duckosm levels`.
2. **The tables are the master.** The step takes your edits from `visualization.level_edits`,
   `level_heads` and `level_caps`. It writes them to the work folder `DB.levels/` (which is only a
   work area, rewritten each time), solves, and writes the result back. The editor
   (`roadstyle-levels edit DB.levels`) already writes the tables at each Apply.
3. **A rebuild keeps the edits.** Before the old file is deleted, the build reads its
   `level_edits`, `level_heads`, `level_caps` and `level_unmatched` (below), and writes them into
   the new file. Then rule 1 solves again with them.
4. **A road that is gone is ignored, with a warning.** An edit row names edges by `edge_id`
   (`edits`: `a`, `b`; `heads`, `caps`: `road`). A row whose road is not in the new file is not
   used. It goes to `visualization.level_unmatched` (`kind`, the row as text) and the build says
   how many. The next build tries these rows again. An `edge_id` is a content hash, so a road that
   comes back with the same shape has the same id.
5. **A switch:** `options.levels` (default `true`). For a big area, `false` skips the step: the file has
   no `edge_levels`, and `duckosm levels DB` makes it later. A file with the tables and
   without the step stays valid.

## Why the order is stable

The result is solved from the roads of the file and the edits in the file. A rebuild solves
again, so `edge_levels_meta` always matches the file (`n_edges` and the hash of the ids).
mapstyle's error "solved for other roads" cannot happen for a file that the build made.

## Cost

The step adds seconds to a build: Norrmalm 8 s, Sundbyberg under 1 s, Monaco about 14 s. The
solver time grows with the area. Stockholm county is not measured: use `options.levels: false`
there until it is.

## Changes

- `config.py`: `Options.levels: bool = True`.
- `importer.py`: read the edits from the old file before `_connect` deletes it; a last step
  `levels` that writes them into the new file and calls `levels.make_and_solve`.
- `levels.py`: `make_and_solve` takes the edits from the tables first (the database wins over
  `DB.levels/*.csv`), drops the rows of roads that are gone into `level_unmatched`.
- Docs: `guides/drawing-order.md`, `reference/configuration.md`, `reference/database.md`.

## Needs from roadstyle (no change to roadstyle in this note)

- The tables `level_*` and `restore_area_inputs` are in the roadstyle branch `geolibre`
  (commit `b326169`), not released (PyPI has 0.22.1 without them). The step needs that release. Until then
  the build skips the step with a warning, and `duckosm viz` works as today.
- Later, in roadstyle (Kaveh's decision): `roadstyle-levels edit --db FILE` reads and writes the
  tables with no folder. Not needed for this note.

## Questions for you

1. **Default of `options.levels`:** `true` (proposed) or `false`?
2. **A clip build** (`source.type: duckdb`): it could take the parent's `level_*` rows for the
   roads it has. Not in this note. Say if you want it.

## Test

- Monaco: after a build, `edge_levels` has one row for every `edge_id` of the three modes, and
  `edge_levels_meta.n_edges` is that number.
- Add a `level_heads` row, rebuild: the row is in the new file and the result is the same.
- A row that names a road that does not exist: the build does not fail. It warns, and the row is
  in `level_unmatched`.
- `options.levels: false`: no `edge_levels`, and the rest of the build is the same.
- Without a roadstyle that has `level_*`: a warning, and the build ends well.
