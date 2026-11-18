"""The program ``duckosm`` of the Python package: the Rust program (``duckosm._rust.cli``), in this Python process.

The commands in ``PYTHON_COMMANDS`` have no Rust port yet: they run duckOSM's Python CLI (``duckosm.cli``).
"""

import os
import signal
import sys

# TODO (step 6 of docs/design/all_in_rust.md): remove each command when its Rust PR merges, then this list.
PYTHON_COMMANDS = {
    # duckOSM's `duckosm build` (--pbf, --output, --config, ...): the Python steps with the Rust network step (DuckOSM.run).
    # The whole build in Rust: `duckosm <PBF> -o <DIR>` and `duckosm --config` (not yet equal: docs/design/all_in_rust.md).
    "build",
    "levels", "bus", "multimodal",                   # no Rust port yet
}


def main():
    if len(sys.argv) > 1 and sys.argv[1] in PYTHON_COMMANDS:
        # a console that cannot show a character (Windows: cp1252 has no "✓") shows "?" in its place, not an error
        for stream in (sys.stdout, sys.stderr):
            if hasattr(stream, "reconfigure"):
                stream.reconfigure(errors="replace")
        from duckosm.cli import main as python_main
        return python_main()
    from duckosm import _rust
    # Ctrl+C stops the program (Python only looks at it between its own statements)
    signal.signal(signal.SIGINT, signal.SIG_DFL)
    # the work in other processes (the tiles) starts this program again: `python -m duckosm`
    os.environ["DUCKOSM_PYTHON"] = sys.executable
    sys.exit(_rust.cli(["duckosm", *sys.argv[1:]]))


if __name__ == "__main__":
    main()
