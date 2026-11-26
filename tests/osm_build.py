"""Build a small network with the Rust build from OSM elements written in a test (``duckosm._rust.write_pbf``).

    db = build(tmp_path, nodes={1: (0, 0), 2: (0.001, 0)}, ways={10: ([1, 2], {"highway": "residential"})})
    con = duckdb.connect(str(db), read_only=True)
"""
import subprocess
import sys

from duckosm import _rust


def write(path, nodes, ways, relations=None):
    """The ``.osm.pbf`` file ``path``. ``nodes``: ``{id: (lon, lat)}`` or ``{id: (lon, lat, tags)}``; ``ways``: ``{id: (refs, tags)}``;
    ``relations``: ``{id: (members, tags)}`` with members ``(type, ref, role)``."""
    n = [(i, float(v[0]), float(v[1]), list((v[2] if len(v) > 2 else {}).items())) for i, v in nodes.items()]
    w = [(i, list(refs), list(tags.items())) for i, (refs, tags) in ways.items()]
    r = [(i, list(members), list(tags.items())) for i, (members, tags) in (relations or {}).items()]
    _rust.write_pbf(str(path), n, w, r)
    return path


def build(tmp_path, nodes, ways, relations=None, modes=("driving",), args=(), name="t"):
    """The Rust build (``duckosm build``) of these elements: the path of the ``.duckdb`` file. ``args``: more options of the command."""
    pbf = write(tmp_path / f"{name}.osm.pbf", nodes, ways, relations)
    out = tmp_path / f"{name}.duckdb"
    cmd = [sys.executable, "-m", "duckosm", "build", "-p", str(pbf), "-o", str(out), "--no-features",
           *[x for m in modes for x in ("-m", m)], *map(str, args)]
    p = subprocess.run(cmd, capture_output=True, text=True)
    assert p.returncode == 0, p.stderr
    return out
