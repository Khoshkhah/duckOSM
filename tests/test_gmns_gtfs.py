"""Feeding a GTFS feed to GMNS: its stops become `location` rows with their gtfs_stop_id."""
import csv
import io
import zipfile

import duckdb
import pytest

pytest.importorskip("pandas")
from duckosm.gmns import to_gmns  # noqa: E402
from duckosm.gtfs import read_stops  # noqa: E402
from tests.test_gmns import A, AR, _source  # noqa: E402


def _feed(path, stops, service=None, folder=False):
    """A GTFS feed: ``stops`` = [(id, name, lat, lon, location_type)], ``service`` = {stop_id: route_type}."""
    files = {"stops.txt": ["stop_id", "stop_name", "stop_lat", "stop_lon", "location_type"] and
             [["stop_id", "stop_name", "stop_lat", "stop_lon", "location_type"]] + [list(s) for s in stops]}
    if service is not None:
        types = sorted(set(service.values()))
        files["routes.txt"] = [["route_id", "route_type"]] + [[f"r{t}", t] for t in types]
        files["trips.txt"] = [["trip_id", "route_id"]] + [[f"t{t}", f"r{t}"] for t in types]
        files["stop_times.txt"] = [["trip_id", "stop_id"]] + [[f"t{t}", sid] for sid, t in service.items()]
    text = {n: "\n".join(",".join(map(str, r)) for r in rows) + "\n" for n, rows in files.items()}
    if folder:
        path.mkdir()
        for n, t in text.items():
            (path / n).write_text(t)
    else:
        with zipfile.ZipFile(path, "w") as z:
            for n, t in text.items():
                z.writestr(n, t)
    return path


STOPS = [("S1", "Main Street", 59.32004, 18.065, 0),       # 4.4 m north of road A: a bus stop
         ("S2", "Rail halt", 59.32004, 18.0652, 0),         # served by a train only
         ("S3", "Far away", 59.40, 18.05, 0),               # 9 km from any link
         ("ST", "The station", 59.32004, 18.0655, 1),       # a station: groups stops, not a place to board
         ("E1", "Station entrance", 59.32004, 18.0656, 2)]  # an entrance
SERVICE = {"S1": 3, "S2": 2, "S3": 3, "E1": 2}


def test_read_stops_tells_buses_from_trains_and_leaves_out_stations(tmp_path):
    for folder in (False, True):
        feed = _feed(tmp_path / ("f" if folder else "f.zip"), STOPS, SERVICE, folder)
        d = read_stops(feed).set_index("stop_id")
        assert set(d.index) == {"S1", "S2", "S3", "E1"}                      # the station (type 1) is left out
        assert d.loc["S1", "loc_type"] == "bus_stop" and bool(d.loc["S1", "is_bus"])
        assert d.loc["S2", "loc_type"] == "train_station" and not d.loc["S2", "is_bus"]
        assert d.loc["E1", "loc_type"] == "entrance"
        assert d.loc["S1", "name"] == "Main Street" and d.loc["S1", "lat"] == pytest.approx(59.32004)
    # without service data every stop is taken for a bus stop; two feeds are one table
    only = _feed(tmp_path / "only.zip", STOPS[:1])
    assert read_stops(only).loc_type.tolist() == ["bus_stop"]
    assert len(read_stops([only, _feed(tmp_path / "two.zip", STOPS[:2])])) == 3
    with pytest.raises(ValueError, match="no stops.txt"):
        with zipfile.ZipFile(tmp_path / "bad.zip", "w") as z:
            z.writestr("agency.txt", "agency_name\nx\n")
        read_stops(tmp_path / "bad.zip")


def _build(tmp_path, feed, **kw):
    src = tmp_path / "src.duckdb"
    if not src.exists():
        _source(src)
        con = duckdb.connect(str(src))
        # an OpenStreetMap bus stop on A, 5 m from S1: the same stop, which the GTFS row replaces
        con.execute("INSERT INTO raw.nodes VALUES (7, 59.32, 18.0651, MAP{'highway':'bus_stop','name':'Main St (OSM)'})")
        con.execute("LOAD spatial; CREATE SCHEMA walking")
        con.execute("CREATE TABLE walking.nodes AS SELECT * FROM driving.nodes")
        con.execute("CREATE TABLE walking.edges AS SELECT * FROM driving.edges")
        con.close()
    out = tmp_path / "out.duckdb"
    out.unlink(missing_ok=True)
    to_gmns(str(src), str(out), gtfs=[str(feed)], **kw)
    c = duckdb.connect(str(out), read_only=True)
    c.execute("LOAD spatial")
    return c


def test_gtfs_stops_become_location_rows_on_the_kerb_side_link(tmp_path):
    c = _build(tmp_path, _feed(tmp_path / "f.zip", STOPS, SERVICE))
    rows = {r[0]: r for r in c.execute("""SELECT gtfs_stop_id, link_id, lr, loc_type, name, x_coord, y_coord, ref_node_id, zone_id
                                          FROM gmns_driving.location WHERE gtfs_stop_id IS NOT NULL""").fetchall()}
    assert set(rows) == {"S1"}                       # S2 is a train (no place on a road), S3 too far, E1 an entrance of rail
    s1 = rows["S1"]
    assert s1[3] == "bus_stop" and s1[4] == "Main Street" and s1[8] == 1
    assert (s1[5], s1[6]) == (18.065, 59.32004)       # its own coordinates, not the road's
    # north of the road is the right of westbound AR: the kerb side with right-hand traffic
    assert s1[1] == AR and s1[7] == 2                 # AR starts at node 2
    length = c.execute(f"SELECT length FROM gmns_driving.link WHERE link_id = {AR}").fetchone()[0]
    assert s1[2] == pytest.approx(length / 2, rel=0.01)
    # the OpenStreetMap bus stop 5 m away was the same stop: its rows (on A and AR) are gone; the others stay
    assert c.execute("SELECT count(*) FROM gmns_driving.location WHERE osm_id = 7").fetchone()[0] == 0
    assert c.execute("SELECT count(*) FROM gmns_driving.location WHERE loc_type = 'crossing'").fetchone()[0] > 0
    # left-hand traffic: the kerb is on the other side
    c.close()
    c = _build(tmp_path, tmp_path / "f.zip", drive_side="left")
    assert c.execute("SELECT link_id FROM gmns_driving.location WHERE gtfs_stop_id = 'S1'").fetchone()[0] == A


def test_every_stop_is_a_place_to_walk_to_in_the_walking_schema(tmp_path):
    c = _build(tmp_path, _feed(tmp_path / "f.zip", STOPS, SERVICE))
    got = dict(c.execute("SELECT gtfs_stop_id, loc_type FROM gmns_walking.location WHERE gtfs_stop_id IS NOT NULL").fetchall()) \
        if c.execute("SELECT count(*) FROM duckdb_tables() WHERE schema_name = 'gmns_walking'").fetchone()[0] else {}
    assert got == {"S1": "bus_stop", "S2": "train_station", "E1": "entrance"}


def test_the_csv_carries_gtfs_stop_id_and_a_name_only_as_an_extension(tmp_path):
    src = tmp_path / "src.duckdb"
    _source(src)
    to_gmns(str(src), str(tmp_path / "o.duckdb"), to_csv=str(tmp_path / "c"), gtfs=[str(_feed(tmp_path / "f.zip", STOPS, SERVICE))])
    head = next(csv.reader(open(tmp_path / "c" / "location.csv", newline="")))
    assert "gtfs_stop_id" in head and "name" not in head and "u_name" not in head
    to_gmns(str(src), str(tmp_path / "o2.duckdb"), to_csv=str(tmp_path / "c2"), csv_extensions=True,
            gtfs=[str(_feed(tmp_path / "f2.zip", STOPS, SERVICE))])
    assert "u_name" in next(csv.reader(open(tmp_path / "c2" / "location.csv", newline="")))
