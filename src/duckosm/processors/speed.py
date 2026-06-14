"""
Speed processor - assigns and normalizes speed limits.
"""

from duckosm.processors.base import BaseProcessor


class SpeedProcessor(BaseProcessor):
    """
    Process and fill missing speed limits.
    
    - Converts mph to km/h
    - Fills missing speeds with defaults based on highway type or mode
    """
    
    # Default speeds by highway type (km/h) for driving
    DRIVING_SPEED_DEFAULTS = {
        'motorway': 110,
        'motorway_link': 110,
        'trunk': 90,
        'trunk_link': 90,
        'primary': 70,
        'primary_link': 70,
        'secondary': 60,
        'secondary_link': 60,
        'tertiary': 50,
        'tertiary_link': 50,
        'residential': 30,
        'living_street': 20,
        'service': 20,
        'unclassified': 40,
        'road': 40,
    }
    WALKING_SPEED = 5.0
    CYCLING_SPEED = 15.0
    DEFAULT_DRIVING_SPEED = 50.0
    
    def __init__(self, con, mode: str = "driving"):
        super().__init__(con)
        self.mode = mode
    
    def run(self) -> None:
        """Process speed limits."""
        self._add_maxspeed_kmh_column()
        self._drop_raw_maxspeed()
    
    def _add_maxspeed_kmh_column(self) -> None:
        """Add normalized maxspeed_kmh column to edges."""
        if self.mode == "walking":
            update_sql = f"UPDATE edges SET maxspeed_kmh = {self.WALKING_SPEED}"
        elif self.mode == "cycling":
            update_sql = f"UPDATE edges SET maxspeed_kmh = {self.CYCLING_SPEED}"
        else:
            # Driving mode uses existing logic
            cases = "\n".join(
                f"WHEN highway = '{hw}' THEN {speed}"
                for hw, speed in self.DRIVING_SPEED_DEFAULTS.items()
            )
            update_sql = f"""
                UPDATE edges SET maxspeed_kmh = 
                    CASE
                        -- Already numeric
                        WHEN TRY_CAST(maxspeed AS DOUBLE) IS NOT NULL 
                            THEN CAST(maxspeed AS DOUBLE)
                        -- MPH to km/h
                        WHEN maxspeed LIKE '%mph%' 
                            THEN CAST(regexp_extract(maxspeed, '(\\d+)', 1) AS DOUBLE) * 1.60934
                        -- Default by highway type
                        {cases}
                        ELSE {self.DEFAULT_DRIVING_SPEED}
                    END
            """
            
        self.execute(f"""
            ALTER TABLE edges ADD COLUMN IF NOT EXISTS maxspeed_kmh FLOAT;
            {update_sql};
        """)

    def _drop_raw_maxspeed(self) -> None:
        """Drop the raw OSM maxspeed string once maxspeed_kmh has been derived.

        maxspeed_kmh is the normalized, always-populated speed; the raw string is
        redundant after the conversion above, so it is removed from the edges table.
        """
        self.execute("ALTER TABLE edges DROP COLUMN IF EXISTS maxspeed")
