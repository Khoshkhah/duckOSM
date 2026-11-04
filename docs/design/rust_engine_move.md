# Move the Rust engine into duckOSM

Status: plan, 2026-11-04. It waits for the answer of Kaveh. Nothing is moved, renamed, pushed or released.

Decision of Kaveh (2026-11-04): the Rust engine of the repository `duckosm-rust` moves into this repository, as the Rust engine of roadstyle moved into roadstyle. Every part of duckOSM then uses the Rust engine by default. The repository `duckosm-rust` keeps the browser page only.

## 1. The end state

| Item | Today | After the move |
|---|---|---|
| Rust crates | `duckosm-rust/crates/duckosm-{core,parquet,duckdb,build,cli,py}` | `duckOSM/rust/duckosm-{core,parquet,duckdb,build,cli,py}` |
| Python module of the engine | `duckosm_rust` (a separate wheel) | `duckosm._rust` (inside the `duckosm` wheel) |
| PyPI package | `duckosm` 0.6.1 (pure Python) | `duckosm` (wheels for 5 systems, with the Rust module) |
| Python CLI | `duckosm` | `duckosm` (no change) |
| Standalone Rust program | `duckosm-rust` | `duckosm-build` (decision D4) |
| `options.engine` | `python` | `rust` |
| Page crate `duckosm-wasm` | `duckosm-rust/crates/duckosm-wasm` | no move; it takes the engine crates from duckOSM as a Git dependency |
| Repository `duckosm-rust` | engine and page | page, `docs/`, Pages workflow |

## 2. How roadstyle did it

Facts from the Git history of roadstyle and from the notes of Kaveh's sessions:

1. **2026-10-23.** The Rust crates came into the same repository, in the folder `rust/`, on the branch `rust-engine` ("Add the Rust engine roadstyle-rs in rust/ (step 1)"). Python stayed the reference. The PyO3 module is `roadstyle._rust` (crate `rust/roadstyle-py`, abi3). `pyproject.toml` uses the build backend `maturin` with `python-source = "src"`. Release 0.25.0 had the Rust engine as an experimental option, with wheels for 5 systems.
2. **2026-10-27, decision D0.** Rust is the default engine. The page, the style and the tiles lost their Python engine.
3. **2026-10-28.** The old `main` got the tag `python-engine-final` (commit `1c835679`).
4. **2026-10-29.** `main` got the commit `027bde66`, "main is the Rust engine from now on". It is a merge of the old `main` with the strategy `ours`, so it has the tree of `rust-engine`. No force push was necessary. `main` and `rust-engine` point to the same commit. Other branches merged `rust-engine` many times before.
5. **The lesson.** For six days there were two lines. Work landed on the wrong line (67 commits of the SimReady toolbox went to the Python `main`). Kaveh saw "nothing changed" because the switch to Rust was done on `rust-engine` only.

What this plan copies:

- The Rust code lives in the same repository as the Python code, in `rust/`.
- A tag marks the last commit of the old line, before the default changes.
- One module in the package (`duckosm._rust`), built with `maturin`, abi3, wheels for 5 systems.
- The Python code stays the reference until the next release after the default changes (design of duckosm-rust, section 12, item 7).

What this plan does differently:

- **No long branch.** The Rust code is new, so it goes into `main` at once, with the default still `python`. The new folder `rust/` changes no Python behavior. This avoids the two-line problem of roadstyle. The default changes later in one small pull request (step M4), after the tag.
- **No `merge -s ours`.** duckOSM keeps its Python files (CLI, exporters, features, bus, levels). `main` does not get a new tree, so the history does not need the trick.
- **An import step.** roadstyle ported its own Python code. Here the Rust code comes from another repository (361 commits). Decision D1 chooses the method.

## 3. The layout in duckOSM

```text
duckOSM/
  pyproject.toml            build backend: maturin (was setuptools)
  src/duckosm/              Python package (no change of the name)
    _rust.abi3.so           the module, built by maturin (git-ignored)
    rust_engine.py          imports duckosm._rust
  rust/
    Cargo.toml              the workspace (members: the 6 crates)
    Cargo.lock
    duckosm-core/ duckosm-parquet/ duckosm-duckdb/ duckosm-build/
    duckosm-cli/            binary duckosm-build
    duckosm-py/             PyO3 crate, lib name _rust
  tests/data/               Monaco and Tartu goldens of the Rust tests (new files)
  tools/                    compare_*.py, make_golden*.py, edge_id_reference.py
  bench/                    python_steps.py, run_bench.py
```

- **Why `rust/<crate>`.** It is the layout of roadstyle (`rust/roadstyle-*`) and SimReady already uses `../roadstyle/rust/...`. The depth is the same as `crates/<crate>`, so the paths `CARGO_MANIFEST_DIR/../../tests/data` in the Rust tests find `duckOSM/tests/data` with no change. The folder `tests/data` of duckOSM has one entry (`gmns_spec`), so the golden files have no name clash.
- **Module and package.** The module is `duckosm._rust`. There is no new PyPI package. The name `duckosm` exists on PyPI (0.6.1, Kaveh), and the package `duckosm-rust` was never published (the last version is the GitHub release v0.3.0). The code `import duckosm_rust` has one user, `rust_engine.py`, so no alias module is necessary.
- **The version check goes away.** `rust_engine.MIN_VERSION` compared two packages. Now the module and the package are one wheel. A test checks `duckosm._rust.__version__ == duckosm.__version__`.
- **No DuckDB in the wheel.** `duckosm-py` has the default feature `duckdb`, which compiles DuckDB 1.5.6 into the module. The Python side imports its own `duckdb` package. Two copies of DuckDB in one process are a risk, and the build takes minutes longer. The wheel builds with `--no-default-features` (the importer asks for `formats=["geoparquet"]` only). The standalone program keeps the bundled DuckDB 1.5.6.
- **Developer build.** `pip install -e .` runs maturin and builds the module in the debug profile. A change of a Rust file needs a new build. Add `rust/duckosm-py/build.py`, as `rust/roadstyle-py/build.py` in roadstyle, for a quick release build into `src/duckosm/`.
- **The program name (D4).** The pip script `duckosm` and a program `duckosm` from a CLI archive would hide each other on `PATH`. roadstyle has this clash. The proposal is `duckosm-build`, with the arguments of today's `duckosm-rust` (`duckosm-build area.osm.pbf -o out`).

## 4. Rust as the default

### The switch

- `src/duckosm/config.py:51`: `engine: str = "rust"`. The CLI option `--engine python` stays, to run the reference.
- The error text for a missing module goes away: the module is always in the wheel. A source install needs a Rust toolchain (the README says so).
- A clip build (`source.type: duckdb`) has no PBF, so the engine has no effect there. This does not change.

### Steps that stay in Python for now

This move changes one thing: the network step and its tables (section "The test" below). The other steps keep their Python code. Rust code exists for some of them (`bus.rs`, `levels.rs` in `duckosm-build`, the layers of the page). The notes `docs/bus.md`, `docs/levels.md` and `docs/features.md` from duckosm-rust have the results of their equality checks. Each step switches later, alone, with its own comparison on Monaco and Tartu.

| Step | Why it stays | Rust state |
|---|---|---|
| osmium cut of the PBF (`pbf/` cache) | The build cuts with the osmium strategy `smart` when features are on. The Rust cut (`--boundary`) equals `complete_ways` and `smart` needs a check. | `cutfile.rs` and the boundary cut exist |
| `raw.*` (`ST_READOSM`) | Features, bus routes and the fixes read it. | The Rust build can write `raw` tables (`Params.raw`) |
| Features (`features.*`) | A step that has a large output and its own golden test. | Layers exist for the page |
| Bus routes (`bus.py`) | As above. | `bus.rs` |
| Levels (`visualization.edge_levels`) | Python calls roadstyle. | `levels.rs` calls `roadstyle-levels` |
| Multimodal graph, ComponentFilter, H3, indexes, validation | They run after the tables are loaded. | none |
| RoadFilter (`ways`, `way_nodes`) | The tables stay in the file. | none |

The Rust engine replaces GraphBuilder, GraphSimplifier, PathConnector, DismountMarker, SpeedProcessor, CostCalculator, FunctionalType and the private split, as `docs/design/rust_engine.md` says. It also replaces RestrictionProcessor and EdgeGraphBuilder.

### Decision: all parts go to Rust, one by one

Kaveh wants every part on the Rust engine by default. The plan does this in stages, after the network step (M4). Each stage is one pull request with its own golden test on Monaco and Tartu. The default of a stage changes only when the test passes. The Python code stays as the reference for one release.

| Stage | Part | Why this order |
|---|---|---|
| R1 | Bus routes | Small. `bus.rs` is done and was checked in duckosm-rust. |
| R2 | Levels | `levels.rs` is done and was checked. It uses `roadstyle-levels`, so Python no longer calls roadstyle for the build. |
| R3 | Features (`features.*`) | Large output. It needs `raw.*` or the Rust raw tables. |
| R4 | `raw.*` and the cut (osmium) | Last. The Rust cut equals osmium on Monaco and Tartu for `complete_ways`. The strategy `smart` needs a check. Then osmium and `ST_READOSM` are not necessary. |

### The test that proves equal output

`tests/test_engine_rust.py` builds `config/sample_monaco.yaml` with both engines and compares every row of 20 tables (6 tables for each of the three modes, plus 2 turn tables of driving). After the move it changes in three ways:

1. `pytest.importorskip("duckosm_rust")` goes away. A skip is not a pass (`AGENTS.md`). A new test fails when `duckosm._rust` does not import.
2. The test sets `engine="python"` and `engine="rust"` by name, because the default is now `rust`.
3. Tartu is a manual check with `tools/compare_with_duckosm.py` against `data/db/tartu.duckdb` (built by duckOSM 0.6.x). Estonia is a manual check too (decision D8).

The Rust tests (`cargo test`) keep the golden files from duckosm-rust: the Monaco TSV summaries, `golden_monaco_modes.yaml`, the lane profile and the graph summary.

### The known difference

The note in `tests/test_engine_rust.py` says that connector rows (`osm_id < 0`) can have another direction label in the two engines. The commit `123910c` ("PathConnector: a fixed order of the dangling ends, and min(highway)") says that duckosm-rust uses the same order, "so the two programs are equal on every edge row". The mask `LABELS` is possibly not necessary any more.

- **Step M1 check.** Remove the mask on a copy of the test and run it on Monaco. If it passes, the difference is gone, and the mask is deleted. If it fails, the mask stays and the changelog of the default switch lists the difference.
- **The edge rows of connectors are the only rows that changed after 0.6.0.** The commit `123910c` changed 68 walking and 32 cycling connector rows (`edge_ref`, `is_reverse`, `highway`). No other row changed.

### What must not change

- **`edge_id`.** The formula is `(hash(osm_id, source, target) >> 1)::BIGINT`. Rust has its own implementation (`duckosm-core`, `hash.rs`). The test in `duckosm-duckdb` compares it with DuckDB's `hash()`. The file `tools/edge_id_reference.py` is the plain Python version. Nothing in the move changes these files. The move commits must not touch a line of `hash.rs`. A check in M1 compares the copy with `S` byte by byte.
- **Byte-equal output.** The tables of duckOSM 0.6.0 (Monaco, Tartu) equal the Rust output row by row, except the connector rows above. The golden test is the guard. A golden file must never change to make a test pass.
- **DuckDB 1.5.6** for the Rust crate (`duckdb = "=1.10506.1"`, `parquet`/`arrow` `=60.0.0`). The Python pin `duckdb>=1.0.0,<2` stays. Files keep `STORAGE_VERSION 'v1.5.0'`, so that DuckDB 1.5.3 opens them. The upgrade to DuckDB 2 is a separate task (`docs/design/duckdb2_upgrade.md`).

## 5. What duckosm-rust keeps

- `crates/duckosm-wasm/` (the page crate and `www/`), `docs/` (app, dashboard, browser, osm-editing, relay), `tests/data` (the Monaco files for the page tests), `.github/workflows/pages.yml` and a `ci.yml` with the page jobs only.
- The page crate builds the same core crates to WebAssembly. It needs them as a dependency from duckOSM:

  ```toml
  duckosm-core  = { git = "https://github.com/Khoshkhah/duckOSM", rev = "<sha>", default-features = false }
  duckosm-build = { git = "https://github.com/Khoshkhah/duckOSM", rev = "<sha>", default-features = false, features = ["snap"] }
  ```

  This is the way the repository takes roadstyle today (`ROADSTYLE_COMMIT` in `build.sh`, `rev` in `Cargo.toml`). Both repositories are public, so CI needs no token.
- **The work loop.** A change of the engine goes to duckOSM first. Then the page moves its `rev`. For a local test, the page uses `[patch."https://github.com/Khoshkhah/duckOSM"]` with a path to a duckOSM checkout. `docs/development.md` of the page describes this.
- **No dependency cycle.** `duckosm-build` takes `roadstyle-levels` from roadstyle (Git `rev`). roadstyle takes nothing from duckOSM or duckosm-rust at build time. The page takes duckOSM and roadstyle.
- **The name of the page repository** is a separate decision (`docs/proposals/name.md` in duckosm-rust). The move does not rename it.

## 6. The order of the steps

Each step is one pull request or one small set of pull requests. Each step ends with a check. The next step starts only when the check passes.

| Step | Where | What | Check |
|---|---|---|---|
| **M0** Prepare | both | Merge or park the open page branches (section 7). Record the source commit `S` of `duckosm-rust/main`. Build `S` once and keep the test output. Take the baseline: `cargo test` counts, `tests/test_wheel.py`, Monaco and Tartu goldens. | The baseline is written in this file (section 9). |
| **M1** Import the crates | duckOSM, branch from `main`, PR | Copy the 6 engine crates, `Cargo.lock`, `tests/data`, `tools/`, `bench/` and the docs of the engine (`design.md`, `benchmark.md`, `tables.md`, `edge_graph.md`, `lane_profile.md`, `levels.md`, `features.md`, `bus.md`, `boundary_tool.md`) from `S` into `rust/`. The commit message names `S`. A new CI job `rust` runs `cargo test`. No Python file changes. | (a) `git diff --no-index` of each crate against `S` is empty. (b) `cargo test` gives the same counts as the baseline. (c) The Python suite is as before. (d) The mask test above. |
| **M2** The binding | duckOSM, PR | `pyproject.toml` to maturin (`module-name = "duckosm._rust"`, `python-source = "src"`, `manifest-path = "rust/duckosm-py/Cargo.toml"`, `--no-default-features`). `rust_engine.py` imports `duckosm._rust`. Drop `MIN_VERSION`. `.gitignore` the `.so`. Rename the PyO3 module in `duckosm-py` to `_rust`. | `engine=rust` test passes on Python 3.10 and 3.12. `maturin build` gives one abi3 wheel. The wheel has no DuckDB (`nm -D` shows no `duckdb_` symbols). `import duckosm._rust` in a clean venv. |
| **M3** Release workflow and CLI | duckOSM, PR | `release.yml`: `wheels` (5 systems, as roadstyle), `sdist`, `cli` (5 archives of `duckosm-build`), the test of the wheel in a clean venv on the 5 systems, `publish` (as today). `ci.yml`: Rust toolchain, `package` job runs `maturin build`. | A run by hand (`workflow_dispatch`, no tag) builds all files and passes the wheel test. **No `v*` tag is pushed.** |
| **M4** The default | duckOSM, PR | Tag the commit before the PR: `python-default-final` (D7). Then `engine: str = "rust"`, `test_engine_rust.py` as in section 4, docs (`docs/design/rust_engine.md`, `reference/configuration.md`, `templates/config.yaml`, the `skills/duckosm` skill, `AGENTS.md`, `README`, `CHANGELOG`). | The full Python suite passes with the new default. The Monaco and Tartu goldens equal 0.6.x. The files of Monaco built by both engines have the same set of `edge_id` values as `monaco.duckdb` (built by 0.6.x). Estonia: only after the OK of Kaveh. |
| **M5** The page repository | duckosm-rust, PR | Replace the path dependencies with the Git dependency on the commit of M4. Delete the 6 engine crates (their history stays in Git). Update `build.sh`, `ci.yml` (page jobs), `README.md`, `docs/design.md` (a pointer to duckOSM), `CHANGELOG.md`, `tools/cargo`. | `build.sh` builds both WebAssembly modules. `cargo test -p duckosm-wasm` passes. The Node run writes files that equal the native files (`cmp`). The Pages workflow builds. The page opens at a local port (Monaco). |
| **M6** Downstream | other repositories | Section 8. | Each repository builds and passes its own tests. |
| **M7** Release | duckOSM | Version 0.7.0, `CHANGELOG`, then the tag `v0.7.0`. The tag publishes to PyPI. | **Only Kaveh decides.** |

The order is M0, M1, M2, M3, M4, M5, M6, M7. M1 to M4 touch only duckOSM, so they cannot collide with page work. A pull request that changes `rust/` and Python files together is split in two.

## 7. Open branches in duckosm-rust

State at `origin/main` = `53b3196` (2026-11-03):

| Branch | Commits ahead | Engine crates | Page files |
|---|---|---|---|
| `claude/fixes-tabs` | 2 | none | 2 (`serve.py`, the map worker) |
| `claude/design-step` | 8 | none | 16 (`www/`), `CHANGELOG.md`, `docs/` |
| `claude/great-tesla-151906` | 4 | none | none (`README.md`, `CHANGELOG.md`, `docs/`: manual, CLI guide, name proposals) |
| `claude/all-test` | 4 | 16 files | 9 (old test branch, many merge bases) |

- Neither page branch changes an engine file, a `Cargo.toml` or a workflow. M1 to M4 are in another repository. They cannot collide.
- M5 is the only step that edits `duckosm-rust`. It changes `Cargo.toml`, `crates/duckosm-wasm/Cargo.toml`, `build.sh`, the workflows, `README.md`, `CHANGELOG.md` and the docs. `claude/design-step` and `claude/great-tesla-151906` also edit `CHANGELOG.md`, `README.md` and `docs/`. **M5 starts after `claude/fixes-tabs`, `claude/design-step` and `claude/great-tesla-151906` are merged.** If Kaveh wants M5 earlier, the two branches rebase on it. The rebase of page files is clean, and the conflicts are only in `CHANGELOG.md` and `docs/`.
- **Freeze.** From the merge of M1 to the merge of M5, no engine change goes into duckosm-rust. Today no open branch has one. A page task that needs an engine change in this window goes to duckOSM (`rust/`) first, and M5 takes it. Keep the window short.
- `claude/all-test` has engine files. It needs a decision from Kaveh: delete it, or merge it before M1.
- The page sessions need a message at M1 (the freeze starts) and at M5 (the new dependency and the work loop of section 5).

## 8. Downstream users

| User | What changes | When |
|---|---|---|
| **simready** | `rust/duckosm-sim/Cargo.toml` and `rust/duckosm-sim-cli/Cargo.toml` have path dependencies `../../../duckosm-rust/crates/duckosm-{core,build}`. They become `../../../duckOSM/rust/duckosm-{core,build}`. `rust/Cargo.toml` and the docs (`docs/7-plan/crates.md`, `docs/1-system/architecture.md`) name the new path. `page/build.sh` copies the page from `duckosm-rust/crates/duckosm-wasm/www`: this path does not change. | M6, after M4 |
| **roadstyle** | No code change. `tests/data` has its own copies of the golden levels (`level_edges.parquet`, `roads.parquet`), so the "duckosm-rust's own Parquet files" in `rust/roadstyle-rs/Cargo.toml` are a comment. The script `rust/roadstyle-assets/sync_old_paths.py` writes the old paths for the page of duckosm-rust. The page stays there, so the script stays until the page takes the new paths (task T6 in roadstyle). The Python extra `levels` (`roadstyle>=0.24.2`) in duckOSM stays. | Comments only, with T6 |
| **sensor-matching, SonoFlow, fetching-sweden-data** | They join on `edge_id` and do not use `duckosm_rust` (no match in their trees). They read `.duckdb` files that duckOSM Python writes. The tables and the `edge_id` values do not change. | A check at M4: their join keys are equal on the Monaco file |
| **duckOSM docs and skill** | `docs/design/rust_engine.md`, `skills/duckosm/SKILL.md`, `AGENTS.md`, `mkdocs.yml` (the engine pages), `docs/install.md` | M4 |
| **Release workflow** | `release.yml` of duckOSM gets the matrix of `ci.yml` of duckosm-rust (wheels, CLI archives, sdist). The release of the files to GitHub (`gh release create`) is dropped or kept as a second job. The old releases v0.1.0 to v0.3.0 of duckosm-rust stay. | M3 |
| **Build helper** | `tools/cargo` (the shared build folder) lists the repositories in `REPOS`. Add `$HOME/projects/duckOSM`. The target link is `rust/target`. | M1 |
| **Notes of Kaveh's sessions** (memory) | `shared-build-folder`, `duckosm-golden-databases`, `build-test-pitfalls`: update the paths | M4 |

Each repository is changed by a pull request of its own, after Kaveh says yes. This plan does not change another repository.

## 9. The baseline (to fill in at M0)

| Check | Value at `S` |
|---|---|
| `cargo test --release -p duckosm-core -p duckosm-duckdb -p duckosm-parquet -p duckosm-build` | tests passed: |
| Monaco sample, CLI counts | driving 3092 edges / 1904 nodes / 724 private; walking 12238 / 4818 / 166; cycling 10670 / 4867 / 141 |
| `tests/test_wheel.py` | |
| `tests/test_engine_rust.py` (Python 3.10, 3.12) | |
| Wheel size | |

The Monaco counts are the lines that `ci.yml` of duckosm-rust checks today.

## 10. Risks

| Risk | What helps |
|---|---|
| **The wheel is not pure Python any more.** A user with no wheel for the system needs a Rust toolchain. | 5 systems, abi3 (Python 3.9 and newer). The README says it. The sdist builds with `maturin`. |
| **PyPI.** `duckosm` is on PyPI. A bad wheel there is hard to undo. The DuckDB trademark question of duckosm-rust is open. | The wheels build on the release branch and by hand first. The tag is Kaveh's decision (M7). |
| **Two DuckDB copies in one process.** | The wheel has no DuckDB (section 3). |
| **`edge_id` changes by mistake.** | The files with the hash are not touched. The DuckDB hash test, the goldens and the `edge_id` set check at M4 fail at once. |
| **A silent skip.** `importorskip` hides a missing module. | The skip is removed. A test fails when the module is missing. |
| **CI time.** A cold `cargo test` with the bundled DuckDB takes minutes. | Cache with `Swatinem/rust-cache`. The 5-system wheels run on a tag or by hand only. |
| **Memory and disk of the machine.** | `CARGO_BUILD_JOBS=4`, one heavy build at a time, the shared build folder of duckOSM (`rust/target` linked to `.shared-target`). Wheel builds on the CI runners, not here. |
| **The page loses the quick work loop.** | `[patch]` with a path (section 5). |
| **Other sessions edit `pyproject.toml`, `ci.yml`, `release.yml` or `importer.py` in duckOSM.** | M2 and M3 are small. The worktrees `gmns-pair-level` (GMNS, Python only) do not touch these files today. Check `git diff origin/main` before M2 and M3. |
| **Estonia is a large area.** | Not run without the OK of Kaveh. Monaco and Tartu only. |

## 11. Decisions for Kaveh

| # | Question | Proposal |
|---|---|---|
| D1 | How does the code come in? **(a)** Copy the files and name the source commit in the message. **(b)** Merge the history of duckosm-rust (`--allow-unrelated-histories`), which gives `git log` of every Rust file but puts the history of the page (`www/` is 20 MB today) into duckOSM. | (a). The repository `duckosm-rust` stays public with all history, tags and releases. |
| D2 | Folder `rust/duckosm-*` (as roadstyle) | Yes |
| D3 | Module `duckosm._rust` in the `duckosm` wheel. No new PyPI package. The wheel `duckosm-rust` ends at v0.3.0. | Yes |
| D4 | Name of the standalone program | `duckosm-build` |
| D5 | The wheel has no bundled DuckDB. The program keeps DuckDB 1.5.6. | Yes |
| D6 | Version 0.7.0 for the release with the new default | Yes |
| D7 | Tag `python-default-final` on the last commit with the default `python` (the Python engine code stays, so `python-engine-final` of roadstyle would be a wrong name) | Yes |
| D8 | Estonia as the third golden, once, before the release (Python side 26 minutes, 10 GB; Rust 19 s) | Ask again at M4 |
| D9 | The branch `claude/all-test` | Delete after you look at it |
| D10 | When does M5 start: after `claude/fixes-tabs`, `claude/design-step` and `claude/great-tesla-151906` merge? | Yes |

## 12. Not in this plan

- The new name of the page repository (`docs/proposals/name.md` in duckosm-rust).
- The next parts of the design (features, bus, levels, GMNS) in Rust.
- The upgrade to DuckDB 2.
- A change of any other repository without a pull request that Kaveh sees.
- Any `git push`, rename, deletion or release. Each one waits for the word of Kaveh.
