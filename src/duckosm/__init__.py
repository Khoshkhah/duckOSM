"""
duckOSM - High-performance OSM-to-routing-network converter built on DuckDB.
"""

from duckosm.importer import DuckOSM
from duckosm.config import Config

__version__ = "0.1.0"
__all__ = ["DuckOSM", "Config"]
