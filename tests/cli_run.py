"""Run the program ``duckosm`` of this package (``python -m duckosm``, the Rust CLI) in a test."""
import subprocess
import sys
from types import SimpleNamespace


def invoke(args, **kwargs):
    """``duckosm ARGS``: ``.exit_code``, ``.output`` (the standard output, then the standard error) and ``.stdout``."""
    p = subprocess.run([sys.executable, "-m", "duckosm", *map(str, args)], capture_output=True, text=True, **kwargs)
    return SimpleNamespace(exit_code=p.returncode, output=p.stdout + p.stderr, stdout=p.stdout)
