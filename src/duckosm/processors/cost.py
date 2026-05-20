"""
Cost calculator processor - calculates travel time costs.
"""

from duckosm.processors.base import BaseProcessor


class CostCalculator(BaseProcessor):
    """
    Calculate travel time costs for edges.
    
    Adds:
        - cost_s: Travel time in seconds
    """
    
    def run(self) -> None:
        """Calculate costs."""
        self._add_cost_column()
    
    def _add_cost_column(self) -> None:
        """Add cost_s column (travel time in seconds)."""
        self.execute("""
            ALTER TABLE edges ADD COLUMN IF NOT EXISTS cost_s FLOAT;
            
            UPDATE edges SET cost_s = 
                CASE
                    WHEN maxspeed_kmh > 0 
                        THEN (length_m / 1000.0) / maxspeed_kmh * 3600.0
                    ELSE (length_m / 1000.0) / 50.0 * 3600.0  -- Default 50 km/h
                END
        """)
