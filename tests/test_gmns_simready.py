"""The GMNS values that SimReady reads (docs/exports/gmns_tables.md): link.refs, link.z_vertices, main.elevation_metadata,
config.duckosm_version, and chains whose two directions of a two-way road end at the same nodes."""
import duckdb
import pytest

pytest.importorskip("pandas")
from duckosm import __version__  # noqa: E402
from duckosm.gmns import to_gmns  # noqa: E402
from tests.test_gmns import A, AR, B, _source  # noqa: E402


def _build(tmp_path, extra=None):
    src = tmp_path / "src.duckdb"
    _source(src)
    con = duckdb.connect(str(src))
    con.execute("LOAD spatial")
    con.execute("ALTER TABLE driving.edges ADD COLUMN refs BIGINT[]; ALTER TABLE driving.edges ADD COLUMN z_vertices DOUBLE[]")
    con.execute(f"UPDATE driving.edges SET refs = [1, 2], z_vertices = [10.0, 20.0] WHERE edge_id = {A}")
    con.execute(f"UPDATE driving.edges SET refs = [2, 1], z_vertices = [20.0, 10.0] WHERE edge_id = {AR}")
    con.execute(f"UPDATE driving.edges SET refs = [2, 3], z_vertices = [20.0, 30.0] WHERE edge_id = {B}")
    con.execute("CREATE TABLE main.elevation_metadata(ele_column VARCHAR, product VARCHAR, vertical_datum VARCHAR, sha256 VARCHAR)")
    con.execute("INSERT INTO main.elevation_metadata VALUES ('ele', 'DTM', 'EGM2008', 'abc123')")
    if extra:
        extra(con)
    con.close()
    to_gmns(str(src), str(tmp_path / "out.duckdb"))
    c = duckdb.connect(str(tmp_path / "out.duckdb"), read_only=True)
    c.execute("LOAD spatial")
    return c


def test_refs_heights_ground_and_version(tmp_path):
    c = _build(tmp_path)
    q = lambda s: c.execute(s).fetchall()
    rows = {r[0]: (r[1], r[2]) for r in q("SELECT link_id, refs, z_vertices FROM gmns_driving.link")}
    assert rows[A] == ([1, 2], [10.0, 20.0]) and rows[B] == ([2, 3], [20.0, 30.0])
    assert q("SELECT product, vertical_datum, sha256 FROM main.elevation_metadata") == [("DTM", "EGM2008", "abc123")]
    assert q("SELECT duckosm_version FROM gmns_driving.config") == [(f"duckosm {__version__}",)]


def test_two_way_road_ends_at_the_same_nodes(tmp_path):
    """Way 100 runs 1 -> 2 -> 4 (A, then C) and back (CR, then AR). The source chains join A and C, but not CR and AR (a
    rule on one side only): GMNS cuts A + C at node 2 too, so each direction has the same two chains."""
    C, CR = 7000000000000000001, 7000000000000000002

    def extra(con):
        con.execute("INSERT INTO driving.nodes VALUES (4, ST_Point(18.08, 59.32))")
        con.execute(f"""INSERT INTO driving.edges (edge_id, source, target, osm_id, highway, name, lanes, is_reverse, length_m,
            maxspeed_kmh, geometry) VALUES
            ({C}, 2, 4, 100, 'primary', 'Main', 2, false, 80, 50, ST_GeomFromText('LINESTRING(18.07 59.32, 18.08 59.32)')),
            ({CR}, 4, 2, 100, 'primary', 'Main', 2, true, 80, 50, ST_GeomFromText('LINESTRING(18.08 59.32, 18.07 59.32)'))""")
        con.execute("CREATE TABLE driving.chains(chain_id BIGINT, edge_ids BIGINT[])")
        con.execute(f"INSERT INTO driving.chains VALUES (1, [{A}, {C}]), ({CR}, [{CR}]), ({AR}, [{AR}]), ({B}, [{B}])")

    c = _build(tmp_path, extra)
    chains = sorted(r[0] for r in c.execute("SELECT link_ids FROM gmns_driving.chains").fetchall())
    assert [A] in chains and [C] in chains and [A, C] not in chains
    assert sorted(x for ch in chains for x in ch) == sorted([A, AR, B, C, CR])      # each link in one chain
