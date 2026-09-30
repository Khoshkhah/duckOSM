# Development

## Set up

```bash
git clone https://github.com/Khoshkhah/duckOSM.git && cd duckOSM
python -m venv .venv && source .venv/bin/activate
pip install -e ".[dev,routing]"
```

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
| `rasterio` | `pip install "duckosm[elevation]"` |
| `geopandas` | `pip install geopandas` |
| a local PBF or built db (`pbf/sodermalm*.osm.pbf`, `data/db/sodermalm*.duckdb`) | build it with the matching `config/*.yaml` |

A skip is not a pass: run `-rs` to see what didn't run.

CI (`.github/workflows/ci.yml`) runs the tests on Python 3.10 and 3.12 and builds the package with
`twine check --strict` on every push to `main` and every pull request.

## Checking a build

Every build can check its own invariants (single dominant component, no stranded named edge) and
fail on a breach: `validation.enabled: true` (on in `config/template.yaml`, off by default in code).

For the geometry of a finished db:

```bash
python scripts/validate_geometry.py --db data/db/<area>.duckdb
```

| Check | Expected |
|---|---|
| self-loops (`source = target`) | 0: loops are split at a virtual node |
| edge geometry starts / ends at its `source` / `target` node | 0 mismatches |
| zero-length edges | 0 |

## Docs

```bash
pip install "mkdocs>=1.6,<2" "mkdocs-material>=9.5,<10"
mkdocs serve                  # live preview at http://127.0.0.1:8000
mkdocs build --strict         # what CI runs: a broken link fails the build
```

`.github/workflows/docs.yml` builds the site and deploys it to GitHub Pages on every push to `main`
that touches `docs/` or `mkdocs.yml`. Pages under `docs/design/` are design notes: they stay in the
repo but are not published.
