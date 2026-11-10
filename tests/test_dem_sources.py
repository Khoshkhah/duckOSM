"""The DEM source catalog (src/duckosm/data/dem_sources.json, docs/reference/elevation-sources.md)."""
import json
import re
import subprocess
import sys
from pathlib import Path

from duckosm import dem_sources
from duckosm.elevation import EUROPE_BBOX, PROVIDERS

ROOT = Path(__file__).resolve().parent.parent
MONACO = (7.41, 43.73, 7.44, 43.75)
TARTU = (26.60, 58.30, 26.85, 58.45)
STOCKHOLM_COUNTY = (17.2, 58.6, 19.0, 60.1)     # north of 60°N: no SRTM


def ids(bbox, **kw):
    return [s["id"] for s in dem_sources.sources_for_bbox(bbox, **kw)]


def test_monaco_has_no_national_model():
    got = ids(MONACO)
    # Monaco is inside the outline of France, but IGN does not cover it (an `exclude` box)
    assert got[:3] == ["eudtm", "fabdem", "copernicus-glo30"]
    assert "fr-ign-rgealti" not in got and "gebco-etopo" not in got and "aws-terrain-tiles" not in got


def test_tartu_starts_with_the_estonian_dtm():
    assert ids(TARTU)[:4] == ["ee-maaamet-dtm", "eudtm", "fabdem", "copernicus-glo30"]


def test_stockholm_starts_with_the_swedish_dtm():
    got = ids(STOCKHOLM_COUNTY)
    assert got[:4] == ["se-lantmateriet-1m", "eudtm", "fabdem", "copernicus-glo30"]
    assert "srtm" not in got and "nasadem" not in got        # they stop at 60°N


def test_outside_europe_and_the_poles():
    assert ids((-123.2, 49.2, -123.0, 49.3))[:3] == ["ca-nrcan-hrdem", "fabdem", "copernicus-glo30"]   # Vancouver
    assert ids((10.0, 85.0, 11.0, 86.0)) == ["copernicus-glo30", "copernicus-glo90"]                   # no FABDEM, ASTER, ...


def test_box_across_a_border_is_not_covered_by_a_national_model():
    assert "ee-maaamet-dtm" not in ids((27.0, 58.0, 29.0, 59.0))        # reaches into Russia


def test_the_fallback_is_only_on_request():
    assert ids(MONACO, include_fallback=True)[-1] == "aws-terrain-tiles"


def test_entries_are_complete():
    need = {"id", "name", "rank", "coverage", "covers_text", "resolution_m", "resolution_text", "product", "product_text", "crs",
            "vertical_datum", "license", "license_text", "commercial_use", "access", "cors", "duckosm_source"}
    seen = set()
    for s in dem_sources.load():
        assert need <= set(s), s["id"]
        assert s["id"] not in seen
        seen.add(s["id"])
        assert s["product"] in ("DTM", "DSM")
        assert s["access"]["account"] in ("none", "free", "key") and s["access"]["url"].startswith("https://")
        assert s["commercial_use"] in (True, False, None) and s["cors"] in (True, False, None)
        assert ("bbox" in s["coverage"]) != ("polygon" in s["coverage"])
        if s["rank"] == 1:
            assert s["product"] == "DTM" and s["country"]


def test_fabdem_is_not_for_commercial_use():
    assert dem_sources.get("fabdem")["commercial_use"] is False


def test_the_copernicus_cog_files_have_no_cors():
    assert dem_sources.get("copernicus-glo30")["cors"] is False and dem_sources.get("aws-terrain-tiles")["cors"] is True


def test_the_python_providers_agree_with_the_catalog():
    for source in PROVIDERS.values():
        e = dem_sources.get({"copernicus": "copernicus-glo30", "eudtm": "eudtm"}[next(k for k, v in PROVIDERS.items() if v is source)])
        assert (source["product"], source["vertical_datum"], source["license"], source["resolution_m"], source["dem_crs"]) == \
               (e["product"], e["vertical_datum"], e["license"], e["resolution_m"], e["crs"])
    assert dem_sources.get("eudtm")["coverage"]["bbox"] == list(EUROPE_BBOX)
    assert {s["duckosm_source"] for s in dem_sources.load() if s["duckosm_source"]} == set(PROVIDERS)


def test_the_docs_page_has_the_tables_of_the_catalog():
    r = subprocess.run([sys.executable, str(ROOT / "scripts/update_elevation_sources_doc.py"), "--check"], cwd=ROOT)
    assert r.returncode == 0, "run: python scripts/update_elevation_sources_doc.py"


def test_the_docs_page_names_every_source():
    text = (ROOT / "docs/reference/elevation-sources.md").read_text(encoding="utf-8")
    for s in dem_sources.load():
        if not s.get("fallback"):
            assert s["name"].split(" (")[0].split(",")[0] in text, s["id"]


def test_coarse_models_are_hidden_by_default():
    assert "gebco-etopo" not in ids(MONACO) and "gebco-etopo" in ids(MONACO, include_all=True)
