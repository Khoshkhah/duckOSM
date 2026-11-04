"""Time the network step of duckOSM (Python) and duckosm-rust on one PBF file.

    python bench/run_bench.py --pbf area.osm.pbf --label monaco --runs 3 [--e2e-runs 3]

For each program: one untimed run (to warm the file cache), then N timed runs under `/usr/bin/time -v`.
It prints the minimum and the median of the wall time and the peak memory (maximum resident set size).
For the network step alone it adds the sum of the step times that each program reports itself:
  duckOSM: _load_pbf, _build_global_junctions, _filter_roads, _build_edges, _simplify_graph, _split_private
  duckosm-rust: read ways, read nodes, build network
"""
import argparse
import re
import statistics
import subprocess
import sys
import tempfile
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent
VENV = Path("/home/kaveh/projects/duckOSM/.venv/bin")
RUST = ROOT / "target/release/duckosm-rust"
PY_NET_STEPS = ["_load_pbf", "_build_global_junctions", "_filter_roads", "_build_edges", "_simplify_graph", "_split_private"]
RS_NET_STEPS = ["read ways", "read nodes", "build network"]


def run(cmd, cwd):
    t = time.perf_counter()
    p = subprocess.run(["/usr/bin/time", "-v", *map(str, cmd)], cwd=cwd, capture_output=True, text=True)
    wall = time.perf_counter() - t
    if p.returncode:
        sys.exit(f"command failed: {cmd}\n{p.stderr[-2000:]}")
    rss = int(re.search(r"Maximum resident set size \(kbytes\): (\d+)", p.stderr).group(1)) / 1024
    steps = {m.group(1): float(m.group(2)) for m in re.finditer(r"STEP (\S+) ([\d.]+)", p.stdout)}
    steps.update({m.group(1): float(m.group(2)) for m in re.finditer(r"^\s+([a-z ]+): ([\d.]+) s$", p.stderr, re.M)})
    return wall, rss, steps


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--pbf", required=True)
    ap.add_argument("--label", required=True)
    ap.add_argument("--runs", type=int, default=3)
    ap.add_argument("--e2e-runs", type=int, default=None, help="runs of the plain `duckosm build` (default: --runs; 0 skips it)")
    a = ap.parse_args()
    e2e_runs = a.runs if a.e2e_runs is None else a.e2e_runs
    pbf = Path(a.pbf).resolve()
    work = Path(tempfile.mkdtemp(prefix=f"bench-{a.label}-", dir="/tmp/claude-1000"))
    cfg = work / "config.yaml"
    cfg.write_text((HERE / "python_network_step.yaml").read_text().replace("PBF", str(pbf)).replace("name: bench", f"name: {a.label}"))
    jobs = []   # (name, command, runs, step names)
    # start-up: the program starts and stops without a build (imports, loading the libraries)
    jobs.append(("start-up only, duckOSM: `duckosm --version`", [VENV / "duckosm", "--version"], a.runs, None))
    jobs.append(("start-up only, duckosm-rust: `duckosm-rust --version`", [RUST, "--version"], a.runs, None))
    if e2e_runs:
        jobs.append(("duckOSM 0.6.0: `duckosm build --config config.yaml`", [VENV / "duckosm", "build", "--config", cfg], e2e_runs, None))
    jobs.append(("duckOSM 0.6.0: same, lane profile off (`bench/python_steps.py config.yaml --no-lane-profile`)",
                 [VENV / "python", HERE / "python_steps.py", cfg, "--no-lane-profile"], a.runs, PY_NET_STEPS))
    jobs.append(("duckosm-rust: `--format duckdb`", [RUST, pbf, "-o", work / "rs_db", "--name", a.label, "--no-component-filter", "--no-merge-segments", "--no-speeds", "--no-functional-types", "--format", "duckdb", "--timing"], a.runs, RS_NET_STEPS))
    jobs.append(("duckosm-rust: `--format geoparquet`", [RUST, pbf, "-o", work / "rs_pq", "--no-component-filter", "--no-merge-segments", "--no-speeds", "--no-functional-types", "--format", "geoparquet", "--timing"], a.runs, RS_NET_STEPS))
    rows = []
    for name, cmd, runs, net in jobs:
        for k in range(runs + 1):           # run 0 is the warm-up
            for f in (work / f"{a.label}.duckdb", work / f"{a.label}.duckdb.wal"):
                f.unlink(missing_ok=True)
            wall, rss, steps = run(cmd, work)
            if k == 0:
                continue
            rows.append((name, wall, rss, sum(steps[s] for s in net) if net else None))
        print(f"done: {name}", file=sys.stderr, flush=True)
    print(f"\n### {a.label}: {pbf.name} ({pbf.stat().st_size / 1e6:.1f} MB), after 1 warm-up run\n")
    print("| Program | runs | wall min (s) | wall median (s) | peak memory (MB) | network steps min (s) | network steps median (s) |")
    print("|---|---|---|---|---|---|---|")
    for name, *_ in jobs:
        r = [x for x in rows if x[0] == name]
        w = [x[1] for x in r]
        n = [x[3] for x in r if x[3] is not None]
        ns = (f"{min(n):.2f}", f"{statistics.median(n):.2f}") if n else ("-", "-")
        print(f"| {name} | {len(r)} | {min(w):.2f} | {statistics.median(w):.2f} | {max(x[2] for x in r):,.0f} | {ns[0]} | {ns[1]} |")
    print(f"\nwall times (s): " + "; ".join(f"{n.split(':')[0]} {n.split(': ')[1][:30]}: " + ", ".join(f"{x[1]:.2f}" for x in rows if x[0] == n) for n, *_ in jobs))
    print(f"work folder: {work}")


if __name__ == "__main__":
    main()
