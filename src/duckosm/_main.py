"""The program ``duckosm`` of the Python package: the Rust program (``duckosm._rust.cli``), in this Python process."""

import os
import signal
import sys


def main():
    from duckosm import _rust
    # Ctrl+C stops the program (Python only looks at it between its own statements)
    signal.signal(signal.SIGINT, signal.SIG_DFL)
    # the work in other processes (the tiles) starts this program again: `python -m duckosm`
    os.environ["DUCKOSM_PYTHON"] = sys.executable
    sys.exit(_rust.cli(["duckosm", *sys.argv[1:]]))


if __name__ == "__main__":
    main()
