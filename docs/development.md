# Development

## Set up

```bash
git clone https://github.com/Khoshkhah/duckOSM.git && cd duckOSM
python -m venv .venv && source .venv/bin/activate
pip install -e ".[dev,routing]"
```

## How the code is laid out

| Where | What |
|---|---|
| `src/duckosm/importer.py` | `DuckOSM(config).run()`: the build. `run()` lists the steps in order ([How a build works](concepts/build.md)); read it first |
| `src/duckosm/processors/` | one class per build step, each running SQL on the shared connection. The logic is SQL, not Python loops |
| `src/duckosm/config.py`, `templates/config.yaml` | the config fields and the commented template `init-config` writes ([Configuration](reference/configuration.md)) |
| `src/duckosm/sumo.py`, `matsim*.py`, `gmns*.py`, `gis.py`, `opendrive.py`, … | exporters: each reads a finished database |
| `src/duckosm/cli.py` | every `duckosm` command ([Command line](reference/cli.md)) |
| `docs/design/` | design notes: one per non-trivial change, written and agreed before the code |

## Rules that are easy to break

- **`edge_id`**: `(hash(osm_id, source, target) >> 1)::BIGINT` ([Stable edge ids](concepts/edge-ids.md)).
  Other projects join on it. Don't change the formula, the order of its inputs, or the `duckdb<2` pin.
- **Heavy dependencies are imported inside functions** (geopandas, networkx, rasterio, roadstyle,
  sumolib), so the core install stays small: duckdb, shapely, h3, click, rich, pyyaml, pandas,
  timezonefinder (`[project.dependencies]` in `pyproject.toml`).
- **Paths are relative to the current folder**: a pip install has no repo, so only files inside
  `src/duckosm/` ship.
- **A change to a table** updates [Database schema](reference/database.md); a new command or
  option updates [Command line](reference/cli.md) (a test fails if a command is missing).

## Tests

```bash
pytest tests/ -q                          # everything
pytest tests/test_gmns.py -q              # one file
pytest tests/test_gmns.py -k lane -q      # tests whose name matches
pytest tests/ -q -rs                      # also list why tests were skipped
```

`tests/conftest.py` installs DuckDB's `spatial` extension once, so a fresh machine needs nothing
else. Some tests skip themselves when an optional tool or local data is missing:

| Skipped without | Install / provide |
|---|---|
| `netconvert` | `pip install "duckosm[sumo]"` |
| `ogr2ogr` | GDAL |
| `osmium` | osmium-tool |
| `rasterio` (elevation tests) | `pip install "duckosm[elevation]"` |
| `roadstyle` (`tests/test_route_map.py`) | `pip install "duckosm[viz]"` |
| `geopandas` | `pip install geopandas` (the `viz` extra brings it) |
| the maintainer's local data (`pbf/`, `data/db/`, `data/maps/`, `data/boundaries/`, a mapstyle checkout) | not needed: the Monaco sample (`tests/test_sample_config.py`) covers a full build |

A skip is not a pass: run `-rs` to see what didn't run.

CI (`.github/workflows/ci.yml`) installs GDAL and osmium, runs the tests on Python 3.10 and 3.12
with `pip install -e ".[dev,routing,sumo]"`, and builds the package with `twine check --strict`, on
every push to `main` and every pull request. The route-map tests (`tests/test_route_map.py`) need
the `viz` extra, which CI doesn't install, so they are skipped there: run them locally with
`pip install -e ".[dev,routing,viz]"`. The `dev` extra also installs `black` and `ruff`; CI doesn't
run them.

A route map has no unit test for what happens in the browser. Check one by hand with playwright
(`pip install playwright && playwright install chromium`):

```bash
python scripts/route_map_stress.py reports/monaco_route_map.html   # random trips, on and off screen
```

## Checking a build

Every build can check its own invariants (single dominant component, no stranded named edge) and
fail on a breach: `enabled: true` in the config's `validation:` section (on in the config
template, off by default in code).

For the geometry of a finished db:

```bash
python scripts/validate_geometry.py --db monaco.duckdb
```

| Check | Expected |
|---|---|
| self-loops (`source = target`) | 0: loops are split at a virtual node |
| edge geometry starts / ends at its `source` / `target` node | 0 mismatches |
| zero-length edges | 0 |

## Docs

```bash
pip install "mkdocs>=1.6,<2" "mkdocs-material>=9.5,<10" "mkdocs-llmstxt>=0.5,<0.6"
mkdocs serve                  # live preview at http://127.0.0.1:8000
mkdocs build --strict         # a broken link fails the build: run it before you push
```

`mkdocs build` prints a long warning banner from the Material theme about MkDocs 2.0; it is harmless.
`.github/workflows/docs.yml` builds the site with `--strict` and deploys it to GitHub Pages on every
push to `main` that touches `docs/`, `mkdocs.yml` or the workflow file (it can also be run by hand).
Pull requests don't build the docs. The build also writes `llms.txt` (an index of the site for AI
agents) and `llms-full.txt` (every page as Markdown, in one file). Pages under `docs/design/` are design notes: they stay in the
repo but are not published.

The three maps embedded in the docs are built from the Monaco sample, run from the repo root (it
writes `monaco.duckdb` and `monaco_gmns.duckdb` there; both are git-ignored). Draw them with the
released roadstyle from PyPI (the `viz` extra), not a local roadstyle checkout, so they show what
users get:

```bash
duckosm build -c config/sample_monaco.yaml && duckosm multimodal monaco.duckdb
duckosm viz monaco.duckdb -m driving && cp reports/monaco_driving_network.html docs/maps/monaco_driving.html
duckosm route-map monaco.duckdb -m driving -o docs/maps/monaco_route_map.html
duckosm gmns monaco.duckdb -m driving && duckosm gmns-map monaco_gmns.duckdb --style lane -o docs/maps/monaco_gmns_lanes.html
```

The Monaco numbers quoted in the docs come from the same build: rebuild them after a change to what
a build keeps.
