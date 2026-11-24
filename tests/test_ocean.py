"""features.ocean, the sea from Overture Maps (docs/design/sea.md): a Monaco build with `--sea`.

The build reads Overture Maps from S3, so the test runs only when DUCKOSM_NETWORK_TESTS is set.
The Rust code has no test for the box of the area or for the offline case: see the report."""
import os
from pathlib import Path

import duckdb
import pytest

from cli_run import invoke

DATA = Path(__file__).resolve().parents[1] / "data" / "sample"


@pytest.mark.skipif(not os.environ.get("DUCKOSM_NETWORK_TESTS"), reason="reads Overture Maps from S3")
def test_monaco_sea_from_overture(tmp_path):
    r = invoke([DATA / "monaco.osm.pbf", "--boundary", DATA / "monaco.geojson", "--features", "--sea",
                "-o", tmp_path, "--name", "m", "-f", "duckdb", "--no-graph", "--no-bus-routes"])
    assert r.exit_code == 0, r.output
    con = duckdb.connect(str(tmp_path / "m.duckdb"), read_only=True)
    con.execute("LOAD spatial")
    assert [c[0] for c in con.execute("DESCRIBE features.ocean").fetchall()] == \
        ["osm_id", "osm_type", "kind", "name", "source", "geom"]
    assert con.execute("SELECT count(*) FROM features.ocean").fetchone()[0] >= 1
    sea, land = con.execute("SELECT bool_or(ST_Contains(geom, ST_Point(7.4300, 43.7250))), "
                            "bool_or(ST_Contains(geom, ST_Point(7.4280, 43.7390))) FROM features.ocean").fetchone()
    assert sea and not land                       # off Monaco-Ville / the Casino
