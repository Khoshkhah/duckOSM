# GMNS spec schemas (vendored)

The table schemas duckOSM's `gmns` command writes, `shared_categories.json` and `datapackage.json`,
copied unchanged from the GMNS standard, release **v0.97** (2026-03-04):
https://github.com/zephyr-data-specs/GMNS (`spec/`), Apache-2.0 (`LICENSE-2.0.txt`).
`tests/test_gmns_spec.py` validates duckOSM's `--to-csv` output against them. To update: copy the
files of the new release, rerun the tests, fix what they report.
