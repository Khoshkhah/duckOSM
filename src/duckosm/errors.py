"""The errors of the duckOSM SDK, and the check of an optional dependency."""
import importlib


class DuckOSMError(RuntimeError):
    """An error of duckOSM that says what to do: an unknown mode, a missing file, a missing optional dependency, a
    failed command. It is a ``RuntimeError``, as the errors of the commands were before."""


class MissingDependency(DuckOSMError, ImportError):
    """An optional dependency is not installed. The message gives the pip command."""


# the pip command for each optional module
_PIP = {
    "anywidget": 'pip install "duckosm[widget]"',
    "networkx": 'pip install "duckosm[graph]"',
    "rustworkx": 'pip install "duckosm[graph]"',
    "roadstyle": 'pip install "duckosm[viz]"',
    "geopandas": 'pip install "duckosm[viz]"',
    "matplotlib": 'pip install "duckosm[viz]"',
    "mapstyle": 'pip install "duckosm[viz]"',
    "sumolib": 'pip install "duckosm[sim]"',
    "commonroad": 'pip install "duckosm[hdmap]"',
}


def require(module, what):
    """Import ``module``, or raise :class:`MissingDependency` that says that ``what`` needs it and how to install it."""
    try:
        return importlib.import_module(module)
    except ImportError as e:
        raise MissingDependency(f"{what} needs {module}: {_PIP.get(module, f'pip install {module}')}") from e
