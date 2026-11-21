"""duckosm.sim on Monaco: sumo_net, edge_map (docs/proposals/python-sdk.md section 5.4)."""
import os
import shutil
from pathlib import Path

import pytest

import duckosm

DATA = Path(__file__).parent / "data"
# Monaco: the traffic lights, the stop signs (junctions) and the bus stops in the SUMO export
MONACO_SIGNS = [18, 12, 106]


@pytest.fixture(scope="module")
def net(tmp_path_factory):
    return duckosm.build(pbf=DATA / "monaco.osm.pbf", boundary=DATA / "monaco.geojson", modes=["driving"],
                         output=tmp_path_factory.mktemp("sim") / "monaco.duckdb", progress=False)


def test_edge_map_matsim(net, tmp_path):
    duckosm.sim.to_matsim(net.driving, tmp_path / "network.xml.gz")
    m = duckosm.sim.edge_map(tmp_path / "network.xml.gz")
    assert set(m.edge_id) == set(net.edges().edge_id) and (m.id == m.edge_id.astype(str)).all()


def test_edge_map_names_of_netconvert(tmp_path):
    (tmp_path / "x.net.xml").write_text(
        '<net><edge id=":j_0" function="internal"/><edge id="12#1"/><edge id="34+56"/><edge id="78"/></net>')
    m = duckosm.sim.edge_map(tmp_path / "x.net.xml")
    assert list(zip(m.id, m.edge_id)) == [("12#1", 12), ("34+56", 34), ("34+56", 56), ("78", 78)]


def test_sumo_net_and_edge_map(net, tmp_path):
    sumolib = pytest.importorskip("sumolib")
    if not shutil.which("netconvert"):
        pytest.skip("no netconvert")
    files = duckosm.sim.to_sumo(net.driving, tmp_path / "sumo", run_netconvert=True)
    sn = duckosm.sim.sumo_net(files)
    assert isinstance(sn, sumolib.net.Net) and len(sn.getEdges(withInternal=False)) > 0
    m = duckosm.sim.edge_map(files["net"])
    assert len(m) >= len(sn.getEdges(withInternal=False))
    assert set(m.edge_id) <= set(net.edges().edge_id)            # each SUMO edge goes back to a duckOSM edge


def test_to_sumo_dict_config(net, tmp_path):
    """A dict config goes to the Python code, which reads a connection: the network is opened for it."""
    if not shutil.which("netconvert"):
        pytest.skip("no netconvert")
    files = duckosm.sim.to_sumo(net.driving, tmp_path / "sumo", config={"junctions.join": "false"}, run_netconvert=True)
    assert files["net"].exists() and '<junctions.join value="false"/>' in files["netccfg"].read_text()


def test_to_sumo_finds_the_wheel_and_hides_the_path(net, tmp_path, monkeypatch, capfd):
    """The netconvert of the eclipse-sumo wheel, with no SUMO_HOME and no netconvert on PATH; the log line has the name of
    the program, not its full path (a notebook keeps the line, with the home folder of the user)."""
    pytest.importorskip("sumo")
    for k in ("SUMO_HOME", "VIRTUAL_ENV"):
        monkeypatch.delenv(k, raising=False)
    monkeypatch.setenv("PATH", "/usr/bin:/bin")
    files = duckosm.sim.to_sumo(net.driving, tmp_path / "sumo", run_netconvert=True)
    assert files["net"].exists()
    err = capfd.readouterr().err
    assert "running: netconvert -c" in err and "/netconvert" not in err


def test_sumo_signs_and_bus_stops_rust_equals_python(net, tmp_path):
    """The traffic lights and stop signs of OSM in the .nod.xml, the bus stops in the .add.xml: the Rust writer and the Python writer
    give the same files, and the numbers of Monaco."""
    import duckdb
    from duckosm import sumo
    r = duckosm.sim.to_sumo(net.driving, tmp_path / "rust")
    with duckdb.connect(os.fspath(net), read_only=True) as con:
        p = sumo.to_sumo(con, str(tmp_path / "py"))
    for ext in ("nod.xml", "edg.xml", "con.xml", "add.xml", "netccfg"):
        assert (tmp_path / "rust" / f"network.{ext}").read_bytes() == (tmp_path / "py" / f"network.{ext}").read_bytes(), ext
    counts = ("n_traffic_lights", "n_stop_signs", "n_bus_stops")
    assert [r[k] for k in counts] == [p[k] for k in counts] == MONACO_SIGNS
    nod = r["nod"].read_text()
    assert nod.count('type="traffic_light"') == r["n_traffic_lights"]
    assert nod.count('type="priority_stop"') + nod.count('type="allway_stop"') == r["n_stop_signs"]
    assert r["add"].read_text().count("<busStop ") == r["n_bus_stops"] and "net" not in r and "tll" not in r


def test_sumo_netconvert_reads_the_signs_and_the_programs(net, tmp_path):
    """netconvert makes a traffic light of each light node (lights within 20 m share one program), writes the programs to the
    .tll.xml, and reads them back from the .netccfg with no error; SUMO loads the bus stops on the lanes."""
    import re
    import subprocess
    from duckosm import sumo
    try:
        binary = sumo._find_netconvert()
    except FileNotFoundError:
        pytest.skip("no netconvert")
    f = duckosm.sim.to_sumo(net.driving, tmp_path, run_netconvert=True)
    text = f["net"].read_text()
    assert text.count('type="traffic_light"') == f["n_traffic_lights"]
    programs = len(set(re.findall(r'tl="([^"]+)"', f["nod"].read_text()))) + f["nod"].read_text().count('type="traffic_light"/>')
    assert text.count("<tlLogic ") == f["tll"].read_text().count("<tlLogic ") == programs
    assert '<tllogic-files value="network.tll.xml"/>' in f["netccfg"].read_text()
    subprocess.run([binary, "-c", str(f["netccfg"]), "-o", str(tmp_path / "again.net.xml")], check=True, capture_output=True)
    cfg = tmp_path / "run.sumocfg"
    cfg.write_text(f'<configuration><input><net-file value="{f["net"]}"/><additional-files value="{f["add"]}"/></input>'
                   '<time><begin value="0"/><end value="1"/></time></configuration>')
    subprocess.run([str(Path(binary).with_name("sumo")), "-c", str(cfg)], check=True, capture_output=True)
