"""One progress line for get, build and the exports: it updates in place in a notebook, and is quiet elsewhere."""
import time


def in_notebook():
    """True in a Jupyter kernel (JupyterLab, Notebook, VS Code, Colab, nbclient)."""
    try:
        from IPython.core.getipython import get_ipython
    except ImportError:
        return False
    return type(get_ipython()).__name__ in ("ZMQInteractiveShell", "Shell")      # Shell: Colab


def reporter(progress):
    """A function of one line of text, or None. ``progress``: None (a line in a notebook, nothing elsewhere), True (a
    line), False (nothing) or a function of one line of text."""
    if callable(progress):
        return progress
    if progress is False or (progress is None and not in_notebook()):
        return None
    return Line()


class Line:
    """A line of text that each call replaces in a notebook (one output that updates); elsewhere a line on stderr."""

    def __init__(self):
        self.handle = None

    def __call__(self, text):
        if in_notebook():
            from IPython.display import display
            if self.handle is None:
                self.handle = display(_Text(text), display_id=True)
            else:
                self.handle.update(_Text(text))
        else:
            import sys
            print(text, file=sys.stderr, flush=True)


class _Text:
    """Plain text for display(). Not IPython's Pretty: it reads a text that names a file as that file."""

    def __init__(self, text):
        self.text = text

    def __repr__(self):
        return self.text


def timed(progress, label, fn):
    """Run ``fn()`` with the lines "<label> ..." and "<label>: done in <s> s"."""
    say = reporter(progress)
    t = time.time()
    if say:
        say(f"{label} ...")
    result = fn()
    if say:
        say(f"{label}: done in {time.time() - t:.1f} s")
    return result
