"""Make the golden files of the Rust exports (rust/duckosm-export/tests/golden.rs): the output of the Python exports
on the Monaco database of that test.

Run the Rust test once (it builds the database), then this tool, from the root of the repository:
    cargo test --release -p duckosm-export --manifest-path rust/Cargo.toml
    python tools/make_golden_exports.py rust/target/tmp/duckosm-exports/golden.duckdb tests/data/exports
Each golden file is the output of the Python export before the port. Do not change a golden file to make a test pass.
"""
import gzip
import sys
from pathlib import Path

from duckosm.matsim import to_matsim
from duckosm.railml import to_railml


def main(db, out):
    out = Path(out)
    out.mkdir(parents=True, exist_ok=True)
    to_matsim(db, out / "monaco_network.xml.gz", mode="driving")
    to_matsim(db, out / "monaco_all_network.xml.gz", mode="all", crs="EPSG:3035")
    to_railml(db, out / "monaco.railml.xml")
    for p in sorted(out.iterdir()):
        n = len(gzip.open(p).read()) if p.suffix == ".gz" else p.stat().st_size
        print(f"{p}: {n} bytes")


if __name__ == "__main__":
    main(*sys.argv[1:])
