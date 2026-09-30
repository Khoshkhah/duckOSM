# Public launch plan (0.1.0)

**Status:** decisions agreed 2026-09-29; README / logo work next.

## The pitch

> **Stable edge IDs that survive every rebuild, clip and export.**

duckOSM turns an `.osm.pbf` into a routable network in one DuckDB file. Its edge IDs stay the same
across three operations:

- **rebuild** from the same OSM data gives the same `edge_id`s (content hash, not a counter);
- **clip** a sub-area out of a parent build (`sodermalm ← sweden`) and the sub-area keeps the
  parent's ids;
- **export** to SUMO / MATSim / GMNS / GIS / networkx and the target format's own id *is*
  the `edge_id`.

So per-edge data (counts, speeds, matches, demand) attaches once and is never re-matched.
Every other feature supports this promise or sits in a clearly marked "experimental" section.

## Tiers

| Tier | Contents | README treatment |
|---|---|---|
| **Core** | `build` (PBF → driving / walking / cycling), clip from a parent db, `extract`, stable `edge_id` + `edge_id_hash` macro, `route()` / `Router` | Top of the README; one diagram of an id flowing through rebuild → clip → export |
| **Supported exports** | SUMO (netconvert assembles it), MATSim network + lanes/signals (validated against the official DTD/XSD), GMNS (spec tables), GeoPackage / shapefile, networkx / GraphML | Table: format · command · how it's verified |
| **Experimental** | OpenDRIVE, Lanelet2, railML, lane-level routing, GMNS meso/micro + viewer/map, multimodal `mm.*`, elevation, admin boundaries, `features.*` base map | One "Also included (experimental)" section, one line each, linking to `docs/` |

## Decisions

| # | Question | Decision |
|---|---|---|
| 1 | OpenDRIVE in the headline? | **No — experimental.** Structural checks only; not load-tested in a simulator. |
| 2 | `osm_overrides/` (Kaveh's hand-fixes for known OSM errors) | **Keep and document** as a feature: fix known OSM errors locally without editing OSM. The existing rules stay as real examples. |
| 3 | `duckosm way` (debug: everything the db knows about one OSM way) | **Keep**, one line under an "Inspect" heading; not part of the pitch. |
| 4 | `duckosm gis-debug` (re-reads a GIS export through GDAL, diffs the ids) | **Use as proof**: it is the "how verified" entry for GIS in the exports table. |
| 5 | `features.*` base-map schema (built for duckmap, off by default) | **Experimental**, one line in that section. |

## Blocker: DuckDB trademark guidelines

[duckdb.org/trademark_guidelines](https://duckdb.org/trademark_guidelines) (checked 2026-09-29):

- *"Names derived from 'Duck' (such as 'duckling', 'duckhouse', or similar variations) may not be
  used for software or services that are associated with, built on, or competing with DuckDB."*
  duckOSM is built on DuckDB, so the **name itself** is affected — repo, PyPI package, CLI.
- *"Modified or look-alike 'duck' logos are not permitted."* Rules out a DuckDB-inspired mark,
  the current logo (it has a "DuckDB beak"), and concept C (route duck).

Options: ask the DuckDB Foundation (the page says "when in doubt, please ask"), or rename before
launch. A rename is cheapest now, before anyone installs it. "QuackOSM" is taken (an existing
PyPI package that reads PBFs with DuckDB).

## Release checklist

- [x] CI: `.github/workflows/ci.yml` — tests on 3.10 / 3.12, build + `twine check --strict`
- [x] Declare `pandas` (the GMNS exporter imported it undeclared; a clean install failed 21 tests)
- [x] Switch `project.license` to the SPDX string (setuptools deprecation warning)
- [x] README rewrite per the tiers above (draft, awaiting review)
- [x] Logo: concept D1 "Edge" (one road edge on a disc; ideas from DuckDB's logo, no duck) → banner / icon / mark, wordmark outlined in Space Grotesk
- [x] README images and links as absolute URLs (relative paths break on the PyPI page)
- [ ] At release (repo public): switch the README banner to an absolute URL so it shows on PyPI too — a relative path works on GitHub but not on PyPI, and an absolute raw URL 404s while the repo is private
- [ ] Make the GitHub repo public (needed before PyPI, for the README images)
- [ ] Publish 0.1.0 to PyPI (the `duckosm` name is already reserved by the 0.0.1 placeholder)
