# The editor as a library: `duckosm-editor`

## Problem

The fixes editor (`web/fixes_editor/`) is JavaScript that a host page runs. `duckosm edit` has the files inside its program. Studio is a second host. It depends on
the duckOSM engine (`duckosm-core`, `duckosm-build`) as a library, at one git revision. The editor was not in that library, so the Studio build script fetched
duckOSM at a revision and copied the folder `web/fixes_editor` into its served folder. That is a second way to get the same version, and a copy that can go old.

## Decision

The editor files are a library crate, `rust/duckosm-editor`, in the same workspace as the engine.

- The crate has no dependencies. It has the four files as constants (`include_str!` of `web/fixes_editor/`), `FILES` and `file(name)`.
- `duckosm-edit` takes the files from the crate. Its own copy of the list is gone.
- A host (Studio) adds the crate to its Cargo file at the **same revision** as `duckosm-core`, and serves `FILES` from one folder `fixes_editor/`. It has no copy in git and no
  fetch step in its build script. One revision gives the engine and the editor that belong together.
- The files stay in `web/fixes_editor/`. Node tests (`fixes_units.mjs`), the README and the Python package (`pyproject.toml`, sdist) keep their paths.

## What a host does

```rust
for f in duckosm_editor::FILES {
    serve(&format!("duckosm/fixes_editor/{}", f.name), f.content_type, f.text);
}
```

Then in the page: `import { initFixesPanel } from "./duckosm/fixes_editor/fixes_panel.js"`. The host gives the options of `initFixesPanel`
(`web/fixes_editor/README.md`).

## Tests

- Rust, in the crate: the four files are there and not empty; only these names give a file (no path, no other file); the text equals the file on disk; every relative import of
  `fixes_panel.js` is a file of the crate.
- The existing page tests of `duckosm edit` use the files through the crate.

## Not in this change

- A release to crates.io. A git dependency checks out the whole repository, so `include_str!` can point outside the crate folder. A release needs the files inside the crate folder.
- Studio's change (its Cargo file and its build script). The Studio sessions do it, after this is on main.
