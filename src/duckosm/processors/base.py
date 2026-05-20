"""
Base processor class.
"""

from abc import ABC, abstractmethod
import duckdb


class BaseProcessor(ABC):
    """Base class for all DuckOSM processors."""
    
    def __init__(self, con: duckdb.DuckDBPyConnection):
        """
        Initialize processor.
        
        Args:
            con: DuckDB connection
        """
        self.con = con
    
    @abstractmethod
    def run(self) -> None:
        """Execute the processor."""
        pass
    
    def execute(self, sql: str) -> None:
        """Execute SQL statement."""
        self.con.execute(sql)
    
    def fetchone(self, sql: str):
        """Execute SQL and fetch one result."""
        return self.con.execute(sql).fetchone()
    
    def fetchall(self, sql: str):
        """Execute SQL and fetch all results."""
        return self.con.execute(sql).fetchall()
