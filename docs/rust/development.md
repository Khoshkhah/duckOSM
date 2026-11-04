# Development

## Toolchain status (2026-10-21)

| Tool | Status |
|---|---|
| `rustc`, `cargo`, `rustup` | Installed on 2026-10-21 with `rustup` at user level (`~/.cargo`, `~/.rustup`, profile `default`, channel `stable`). Version 1.99.1. The installer added `~/.cargo/bin` to the `PATH` in the shell profile. |
| `g++`, `cmake`, `make` | Installed (system). The `duckdb` crate needs a C++ compiler. |
| `maturin` | Not installed. It is needed from M3 (Python wheel). |
| `wasm32-unknown-unknown` target | Installed (`rustup target add`). See section 7 of the design. |
| `wasm-bindgen` | `wasm-bindgen-cli` 0.2.129, installed with `cargo install` at user level (`~/.cargo/bin`). Its version must be the same as the `wasm-bindgen` crate in `Cargo.lock`. |
| `node` | Version 24, installed (system). It runs the WebAssembly build without a browser. |
| `nightly-2026-10-15` with `rust-src` | Installed on 2026-10-22 (`rustup toolchain install`). Only for the WebAssembly module with threads: it builds the standard library again with atomics. |

## Install Rust

Done on this machine. On a new machine:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile default
source "$HOME/.cargo/env"
rustup target add wasm32-unknown-unknown   # M4
cargo install wasm-bindgen-cli --version 0.2.129 --locked
rustup toolchain install nightly-2026-10-15 --profile minimal --component rust-src --target wasm32-unknown-unknown
```

## Build and test

```bash
cargo build --release          # the first build takes several minutes: it compiles DuckDB
cargo test --release           # hash vectors, geometry tests, and the Monaco golden test
cargo build --release --no-default-features -p duckosm-cli   # without DuckDB: GeoParquet only, 7.9 MB program, builds in under a minute
./target/release/duckosm-rust tests/data/monaco.osm.pbf -o out --name monaco --no-component-filter
```

The binary is `target/release/duckosm-rust`. `--timing` prints the time of each step.

## One shared build folder

On Kaveh's machine, all worktrees of duckosm-rust and of roadstyle use one build folder for each repository:
`<main checkout>/.shared-target`. Thus the dependencies (DuckDB is the largest) compile one time, not one time
for each worktree. The `cargo` wrapper `tools/cargo` does this. Install it before `~/.cargo/bin` in `PATH`:

```bash
install -m 755 tools/cargo ~/.local/bin/cargo
```

For `build`, `check`, `clippy`, `doc`, `fix`, `run`, `rustc`, `test` and `bench` in a checkout of one of the two
repositories, the wrapper does these steps:

1. If the workspace has no `target`, it makes `target` a link to `.shared-target`. Paths such as
   `target/release/duckosm-rust` thus stay correct. A workspace that has a `target/` folder of its own keeps
   that folder: the change starts when you delete the old folder.
2. Cargo finds a crate "fresh" from the file times, and it identifies a workspace crate by its relative path.
   Thus a worktree with older files can get the binary of the worktree that built last. To prevent this, the
   wrapper keeps the path of the last workspace in `.shared-target/.last-tree`. When another workspace builds,
   the wrapper touches all files of that workspace that Git tracks. The workspace crates then compile again, but
   the dependencies do not.
3. A lock (`.shared-target/.lock`) lets the cargo commands of only one workspace use the folder at a time. The
   commands of the same workspace can run together. When a command waits, the wrapper tells which workspace it
   waits for. A `cargo run` that does not stop (a server) thus stops the builds of the other worktrees: run the
   program from `target/` instead.

`build.sh` puts roadstyle's clone in `target/roadstyle-<commit>`, one folder for each `ROADSTYLE_COMMIT`, and it keeps
a complete clone. The clone is a different Git repository, so it builds in its own `rust/target`. The test
`tests/native.rs` writes to `target/tmp/<checkout>/`. CI has no wrapper and uses a plain `target/`.

To stop the shared folder, delete `~/.local/bin/cargo`, or set `CARGO_TARGET_DIR` or `--target-dir` to a
folder outside `.shared-target`. `cargo clean` deletes the shared folder of all worktrees.

## Check the result against duckOSM

`tests/data/golden_monaco.yaml` builds the golden database with duckOSM 0.6.0 (run it in an empty
folder, with `duckosm build --config`). Then compare every row, column and geometry byte:

```bash
python tools/compare_with_duckosm.py monaco_golden.duckdb out/monaco.duckdb [driving|walking|cycling]   # or the GeoParquet folder
```

`tests/data/golden_monaco_modes.yaml` is the config of the three-mode golden (four variants; the comments say which lines to change). `tools/make_golden_summary.py` writes `tests/data/monaco_modes_summary.tsv` from those databases. `tools/make_golden.py` writes `tests/data/monaco_driving_edges.tsv` from such a database.
`tools/compare_levels.py` compares `visualization.edge_levels` of a duckosm-rust file with roadstyle's result on
the same edges, in duckOSM's virtual environment ([levels.md](levels.md), "The equality check"):

```bash
/home/kaveh/projects/duckOSM/.venv/bin/python tools/compare_levels.py out/monaco.duckdb
```

`tools/edge_id_reference.py` is the `edge_id` hash in plain Python. It compares itself with the
installed DuckDB, and with every edge of a duckOSM database:

```bash
pip install duckdb
python tools/edge_id_reference.py                       # random checks
python tools/edge_id_reference.py path/to/area.duckdb   # every edge of a duckOSM database
```

## Benchmark

See [benchmark.md](benchmark.md). The runner is `bench/run_bench.py`.

## WebAssembly

Build the two modules and their JavaScript bindings (Git ignores both folders):

```bash
./crates/duckosm-wasm/build.sh
```

- `www/pkg/`: one thread, stable Rust.
- `www/pkg-threads/`: rayon on Web Workers (`wasm-bindgen-rayon`), nightly Rust. The browser gives
  WebAssembly threads only to a cross-origin isolated page, so the server must send the headers
  `Cross-Origin-Opener-Policy: same-origin` and `Cross-Origin-Embedder-Policy: require-corp`. Without them,
  the page uses `pkg/`. duckdb-wasm (from cdn.jsdelivr.net), Nominatim and Overpass work with these headers.
  `?threads=N` in the address sets the number of threads (the default: the cores, at most 8).
  GitHub Pages cannot send the two headers: there, `www/coi-serviceworker.js` (one `<script>` line in
  `index.html`) adds them in the browser and loads the page again. The first load has one thread.
- `www/map/pkg/`: roadstyle's Rust module for the roads map. `build.sh` fetches the commit `ROADSTYLE_COMMIT` (a full SHA)
  of roadstyle into `target/roadstyle-<commit>` and builds the crate `roadstyle-wasm` of its Rust engine (`rust/`). With
  `ROADSTYLE_REPO=/home/kaveh/projects/roadstyle`, it fetches from the local copy. `duckosm-build` and `duckosm-parquet`
  take the drawing levels from the crate `roadstyle-levels` of the same commit (a dependency by `rev` in both `Cargo.toml` files).
  Change the three places together ([levels.md](levels.md)).

Open the page (`index.html`, the plan is [browser.md](browser.md)). A browser does not load a module from
a `file://` address, so use a local server. `serve.py` sends the two headers; `python3 -m http.server` does
not (then the page has one thread):

```bash
python3 crates/duckosm-wasm/www/serve.py 8000
```

`serve.py` also has the relay of the page ([relay.md](relay.md)): open `http://localhost:8000/?relay=relay`, and
**Get** in step 3 downloads a Geofabrik extract into the browser.

Then go to `http://localhost:8000`. The page is one app ([app.md](app.md)): choose an area on the map (a
search, a drawing or a file), select a region PBF file that covers it (the page names the smallest Geofabrik
extracts), the modes and an optional OSM fixes file, and click **Build**. The data can also come from the
Overpass API, or from a PBF file that is already cut. The build runs in a Web Worker (`build_worker.js`), which
reads the region file piece by piece. The page loads the tables into duckdb-wasm and shows the roads on the
same map. You can run SQL on the tables, and download the Parquet files and `load.sql`, which makes a
`.duckdb` file from them (`duckdb area.duckdb < load.sql`). The build side of the page is `app/duck.js`. The
page loads MapLibre, terra-draw, polyclip-ts and duckdb-wasm from cdn.jsdelivr.net, and the map from
OpenFreeMap.

Run the same `.wasm` file in Node. The script writes all files and prints the time and the WebAssembly memory:

```bash
node crates/duckosm-wasm/www/run_node.mjs tests/data/monaco.osm.pbf out-wasm driving,walking,cycling [fixes.yaml] [tags|solve|none]
```

Checks:

```bash
cargo test --release -p duckosm-wasm     # the browser build equals duckosm_build::build, byte for byte (Monaco, 3 modes, a fixes file)
python crates/duckosm-wasm/www/e2e.py tests/data/monaco.osm.pbf out-page --fixes fixes.yaml   # the page in headless Chromium (Playwright)
python crates/duckosm-wasm/www/e2e.py tests/data/monaco_sample.osm.pbf out-area --boundary tests/data/monaco.geojson   # the cut to a boundary (add --one-thread for pkg/)
duckosm-rust tests/data/monaco_sample.osm.pbf --boundary tests/data/monaco.geojson -m driving,walking,cycling -f geoparquet -o out-cli   # the same, native
```

The test writes its files to `target/tmp/<checkout>/duckosm-wasm-native/` (`<checkout>`: the folder name of the
checkout, for example `duckosm-rust`). Compare them with `cmp` to the files of
`run_node.mjs` or `e2e.py` (built with the same fixes text as the test). `e2e.py` also prints the times of
the page as JSON. The CLI writes zstd and the page writes snap, so compare `out-area` and `out-cli` by rows,
not with `cmp`.

Check the level editor of the page (card 8) in headless Chromium: open, one edit, Apply, the tables and the `.duckdb`
download, "← Steps". It writes `editor.png` and `editor-dark.png` and prints the time to open the editor:

```bash
python crates/duckosm-wasm/www/editor_e2e.py out-editor
```

Check the level editor as a person uses it (real clicks, no test hook): two roads, a change, "← Steps" with the change waiting, Apply and solve.
On a computer (light and dark) and on a phone (390 × 844, touch):

```bash
python crates/duckosm-wasm/www/editor_path_e2e.py out-editor-path
```

Check the panel (the cards that follow the steps, the help windows and their links, collapse all, the main button) and the action buttons (disabled until
their inputs are ready, with the reason under them). On a computer (light and dark) and on a phone:

```bash
python crates/duckosm-wasm/www/panel_e2e.py out-panel
python crates/duckosm-wasm/www/buttons_e2e.py out-buttons
```

Check the H3 tab of card 1 (hexagons picked with clicks; the area, the cap and a build of Monaco from the cells; a phone too):

```bash
python crates/duckosm-wasm/www/h3_e2e.py out-h3
```

Check the generation of the tiles (the dashboard's red message and the hidden vector layers when the tiles are of another road set,
and the normal path):

```bash
python crates/duckosm-wasm/www/generation_e2e.py
```

Check the tile format (Monaco): MVT is the default, the style and the tile address have the format, MLT gives
the same picture, and the page keeps the choice:

```bash
python crates/duckosm-wasm/www/tile_format_e2e.py out-tile-format
```

Check the page on a phone (390 × 664, touch): roadstyle's sheet with its three heights, the map's padding, and the level editor with
its bar ([app.md](app.md), "On a phone"). It writes `phone-*.png`:

```bash
python crates/duckosm-wasm/www/phone_e2e.py out-phone
```

Check the form of roadstyle's dashboard (the bar and its tools, the panel, one look for the cards in both themes, the boxes, the settings
and the Google key, the windows; and the phone form at 390 × 844). The test needs no build of an area. It writes `dashboard-*.png`
([app.md](app.md), "roadstyle's dashboard look"):

```bash
python crates/duckosm-wasm/www/dashboard_e2e.py out-dashboard
```

### The deployed page: a version on each file

GitHub Pages sends `cache-control: max-age=600`. After a deploy, a browser can thus get the new `index.html` with an
old `app/duck.js` from its cache, and the page fails (for example "duck.roadstyle is not a function"). The workflow
`pages.yml` therefore runs `www/version_refs.py` on the deployed copy: it adds `?v=<the commit's short SHA>` to each
relative reference of a `.js`, `.wasm`, `.css`, `.json` or `.html` file in the `.js` and `.html` files (the imports,
`new URL(...)` of the workers and the wasm files, `fetch(...)`, and `src=` / `href=`). A new deploy thus gives new
addresses, and the browser cannot use an old file. Addresses on other sites (MapLibre, duckdb-wasm) do not change. The
script then makes sure that no relative reference of these types is left without a version, else it stops the job.
The service worker gets a version too: a new deploy installs it again, and the page loads one time more.

Test it on a copy, never on `www/` itself (the script changes the files in place):

```bash
mkdir -p /tmp/www-v && cp -r crates/duckosm-wasm/www/. /tmp/www-v/
python3 crates/duckosm-wasm/www/version_refs.py /tmp/www-v test123        # prints the number of references with a version
python crates/duckosm-wasm/www/e2e.py tests/data/monaco_sample.osm.pbf out-area --boundary tests/data/monaco.geojson --www /tmp/www-v
python crates/duckosm-wasm/www/editor_e2e.py out-editor /tmp/www-v
```

With a folder that has versions, both tests also record each request of the page and its workers. A request for a
`.js`, `.wasm`, `.json` or `.css` file of the site with no `?v=` stops the test. Add `--one-thread` to `e2e.py` to
serve the copy with no COOP and COEP headers, as GitHub Pages does: the service worker then adds them, and the build
has more than one thread.

Check the area steps of the page in headless Chromium (a file, two rectangles drawn with the mouse, a
Nominatim search, and the Build button; `--no-search` skips the search):

```bash
python crates/duckosm-wasm/www/boundary_e2e.py
```

The boundary tool uses `www/geofabrik.json`, a small copy of Geofabrik's list of extracts (Geofabrik sends no
CORS header). Make it again when Geofabrik changes its extracts:

```bash
curl -o index-v1.json https://download.geofabrik.de/index-v1.json
python tools/make_geofabrik_index.py index-v1.json crates/duckosm-wasm/www/geofabrik.json   # needs shapely
```

## Check the base-map layers

The `raw.*` cut and the base-map layers (`features.*`) of [features.md](features.md). duckOSM's `monaco.duckdb`
keeps its `raw.*` and is built with its OSM fixes, so `raw.*` and all 11 layers must be equal:

```bash
cargo test --release -p duckosm-core --test raw     # the raw cut equals osmium's smart cut (tests/data/monaco_smart.osm.pbf)
./target/release/duckosm-rust tests/data/monaco_sample.osm.pbf --boundary tests/data/monaco.geojson -m driving,walking,cycling \
    --fixes ../duckOSM/osm_overrides/osm_overrides.yaml --raw -f geoparquet -o out-raw
python tools/compare_features.py out-raw --duckosm ../duckOSM/monaco.duckdb
python crates/duckosm-wasm/www/e2e.py tests/data/monaco_sample.osm.pbf out-page --boundary tests/data/monaco.geojson \
    --fixes ../duckOSM/osm_overrides/osm_overrides.yaml
python tools/compare_features.py out-raw --page out-page     # the page's features_*.parquet against the native result
```

Another area against osmium (`osmium extract -s smart -p boundary.geojson region.osm.pbf -o cut.osm.pbf`):

```bash
RAW_CHECK="region.osm.pbf boundary.geojson cut.osm.pbf" cargo test --release -p duckosm-core --test raw -- --ignored
```

`crates/duckosm-build/sql/features.sql` is made from duckOSM's code. Make it again when duckOSM's layers change, then
build the WebAssembly modules again (the module holds the text):

```bash
../duckOSM/.venv/bin/python tools/make_features_sql.py
```

## Check the OSM fixes

`monaco.duckdb` of duckOSM is built with its OSM fixes file, so all driving rows must be equal:

```bash
./target/release/duckosm-rust tests/data/monaco.osm.pbf -o out-fixes --fixes ../duckOSM/osm_overrides/osm_overrides.yaml
python tools/compare_with_duckosm.py ../duckOSM/monaco.duckdb out-fixes driving
```

## Check another area (example: Tartu)

Build the golden with duckOSM, then run duckosm-rust with the matching flags and compare. The yaml is
`tests/data/golden_monaco_modes.yaml` with `pbf_path` changed and the switches set (see the comments in it).

```bash
# duckOSM (in an empty folder; the lane profile step is switched off because it is slow and not part of the network step)
python bench/python_steps.py g.yaml --no-lane-profile
# duckosm-rust, same settings as g.yaml (here: no boundary, merge on, global junctions on, sidewalk rule on, connector on)
duckosm-rust area.osm.pbf -o out --format geoparquet --modes driving,walking,cycling --no-component-filter
python tools/compare_with_duckosm.py golden.duckdb out walking
```

Switches: `--no-functional-types`, `--no-speeds`, `--no-costs`, `--no-merge-segments`, `--no-global-junctions`, `--no-sidewalk-inference`, `--no-connect-paths`,
`--no-component-filter`. With a boundary in the yaml (and `build_graph`, `process_speeds` and
`calculate_costs` on, because duckOSM runs the component filter only then), give duckosm-rust the cut file
that duckOSM wrote in `pbf/` and leave the component filter on.

Known difference: a connector between two facing dangling ends can have the other direction label
(`edge_ref`, `is_reverse`, sometimes `highway`). See section 6 of the design.

## Build and test the Python wheel

```bash
python3 -m venv .venv && .venv/bin/pip install maturin
.venv/bin/maturin build --release -m crates/duckosm-py/Cargo.toml -o wheels          # with DuckDB (the first build takes minutes)
.venv/bin/maturin build --release -m crates/duckosm-py/Cargo.toml --no-default-features -o wheels_small   # without DuckDB: 2 MB
python3 -m venv /tmp/t && /tmp/t/bin/pip install wheels/duckosm_rust-*.whl duckdb && /tmp/t/bin/python tests/test_wheel.py
```

## Wheels for other systems

`.github/workflows/ci.yml` builds the wheel on 5 targets (Linux x86_64 and aarch64 with manylinux 2_28, macOS
x86_64 and arm64, Windows x64), plus the sdist, runs `tests/test_wheel.py` on each wheel, runs the Rust tests of
`duckosm-core`, `duckosm-duckdb`, `duckosm-build` and `duckosm-wasm`, and builds the WebAssembly modules. Then it
runs the module in Node on Monaco (`run_node.mjs`) and compares each file with `cmp` to the native file of the
`duckosm-wasm` test. The metadata files hold the build time, so it does not compare them. It runs on each push to `main` of
[Khoshkhah/duckosm-rust](https://github.com/Khoshkhah/duckosm-rust) (private) and on each pull request. The
results are on the Actions page of the repository. It does not publish.

What was checked on this machine (Linux x86_64, 2026-10-21):

```bash
pip install maturin ziglang
maturin build --release --zig --compatibility manylinux2014 -m crates/duckosm-py/Cargo.toml -o wheels   # 13.6 MB, glibc 2.17 or newer
maturin sdist -m crates/duckosm-py/Cargo.toml -o sdist                                                  # 46 KB, contains all crates
```

The `manylinux2014` wheel with DuckDB needs only glibc 2.17 (it has no libstdc++ dependency) and passed
`tests/test_wheel.py` in a new virtual environment. The wheel from a plain `maturin build` needs glibc 2.35.
Not checked: macOS, Windows, aarch64, and a real old Linux distribution.

## Keys for a local test of the page

Street View (Linked) and Google 3D need a Google Maps JavaScript API key, and the Mapillary photos need a Mapillary client token. The page does not have them in its code. Two ways to give them in a test:

- **The settings of the map bar** (the gear button): paste each key. The browser keeps it for this site only (`localStorage`).
- **`crates/duckosm-wasm/www/local_keys.json`**, a file that `.gitignore` leaves out of git: `{"google": "<key>", "mapillary": "<token>"}`. A page on `localhost` or `127.0.0.1` reads it at the start and gives the keys to roadstyle's dashboard as the page's keys. A key that the person saved in the settings is over it. The deployed page never reads the file.

## The pictures of the dashboard guide

`docs/dashboard.md` has pictures in `docs/img/dashboard/`. Make them again after a change of the page:

```bash
python crates/duckosm-wasm/www/docs_shots.py
```

The script builds Monaco in headless Chromium and needs the page built first. The Search picture needs the network.
