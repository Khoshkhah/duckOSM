"""Reference for the duckOSM edge_id (v2) in plain Python. It is the spec the Rust code must match.

    edge_id = (hash(osm_id, source, target) >> 1)           osm_id, source, target: BIGINT
    virtual_node_id = -(hash(osm_id, source, refs::VARCHAR) >> 2)

`hash` is DuckDB's hash(), written out here. It matches DuckDB 1.3.2, 1.4.4 and 1.5.3.
The integer part also matches DuckDB 1.2.2. DuckDB 1.0 and 1.1 give other values.

Check it against the installed DuckDB:    python tools/edge_id_reference.py
Check it against a duckOSM database:      python tools/edge_id_reference.py path/to/area.duckdb
"""
import random
import sys

M64 = (1 << 64) - 1
C = 0xD6E8FEB86659FD93        # multiplier of the integer mix
S = 0xC6A4A7935BD1E995        # multiplier of the string length
SEED = 0xE17A1465             # start value of the string hash


def mix(x):
    """Hash of one 64-bit integer. A negative integer is first read as unsigned (two's complement)."""
    x &= M64
    x ^= x >> 32
    x = (x * C) & M64
    x ^= x >> 32
    x = (x * C) & M64
    x ^= x >> 32
    return x


def combine(h, other):
    """Add the hash of the next argument to the hash of the arguments before it."""
    h ^= h >> 32
    return ((h * C) & M64) ^ other


def hash_str(s):
    b = s.encode()
    h = (SEED ^ (len(b) * S)) & M64
    b += b"\0" * (-len(b) % 8)                    # the last 8-byte word is padded with zero bytes
    for i in range(0, len(b), 8):
        h = ((h ^ int.from_bytes(b[i:i + 8], "little")) * C) & M64
    return mix(h)


def duck_hash(*args):
    """hash(a, b, ...) for BIGINT (int) and VARCHAR (str) arguments."""
    hs = [mix(a) if isinstance(a, int) else hash_str(a) for a in args]
    h = hs[0]
    for other in hs[1:]:
        h = combine(h, other)
    return h


def edge_id(osm_id, source, target):
    return duck_hash(osm_id, source, target) >> 1


def virtual_node_id(osm_id, source, refs):
    """refs is the node list of the arc that is split. DuckDB writes a list as '[1, 2, 3]'."""
    return -(duck_hash(osm_id, source, "[" + ", ".join(map(str, refs)) + "]") >> 2)


def _check_random(con, n=2000):
    rnd = random.Random(1)
    pick = lambda: rnd.choice((rnd.randint(-2**63, 2**63 - 1), rnd.randint(0, 10**9), rnd.randint(-10**7, 0)))
    bad = 0
    for _ in range(n):
        a = [pick() for _ in range(3)]
        want = con.execute("SELECT hash(?::BIGINT, ?::BIGINT, ?::BIGINT)", a).fetchone()[0]
        bad += want != duck_hash(*a)
        s = "".join(rnd.choice("0123456789[], -abc") for _ in range(rnd.randint(0, 80)))
        want = con.execute("SELECT hash(?::BIGINT, ?::BIGINT, ?)", [a[0], a[1], s]).fetchone()[0]
        bad += want != duck_hash(a[0], a[1], s)
    return bad


def _check_db(path):
    import duckdb
    con = duckdb.connect(path, read_only=True)
    for mode in ("driving", "walking", "cycling"):
        try:
            rows = con.execute(f"SELECT osm_id, source, target, edge_id FROM {mode}.edges").fetchall()
        except duckdb.Error:
            continue
        bad = sum(edge_id(o, s, t) != e for o, s, t, e in rows)
        print(f"{mode}: {len(rows)} edges, {bad} different")


if __name__ == "__main__":
    import duckdb
    print("duckdb", duckdb.__version__, "- different results in", _check_random(duckdb.connect()), "random checks")
    if len(sys.argv) > 1:
        _check_db(sys.argv[1])
