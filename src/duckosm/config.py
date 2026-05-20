"""
Configuration handling for duckOSM.
"""

from dataclasses import dataclass, field
from pathlib import Path
from typing import Optional
import yaml


@dataclass
class Options:
    """Processing options."""
    build_graph: bool = True
    h3_indexing: bool = True
    h3_resolution: int = 8
    simplify: bool = False
    process_speeds: bool = True
    extract_restrictions: bool = True
    calculate_costs: bool = True


@dataclass
class Config:
    """Configuration for duckOSM import."""
    
    name: str = "default"
    pbf_path: str = ""
    output_path: str = "output.duckdb"
    boundary_path: Optional[str] = None
    h3_cell: Optional[str] = None
    modes: list[str] = field(default_factory=lambda: ["driving"])
    options: Options = field(default_factory=Options)
    
    @classmethod
    def from_yaml(cls, path: str) -> "Config":
        """Load configuration from YAML file."""
        with open(path, 'r') as f:
            data = yaml.safe_load(f)
        
        options_data = data.pop('options', {})
        options = Options(**options_data)
        
        return cls(
            name=data.get('name', 'default'),
            pbf_path=data.get('pbf_path', ''),
            output_path=data.get('output_path', 'output.duckdb'),
            boundary_path=data.get('boundary_path'),
            h3_cell=data.get('h3_cell'),
            modes=data.get('modes', ["driving"]),
            options=options
        )
    
    @classmethod
    def from_args(
        cls,
        pbf_path: str,
        output_path: str,
        name: str = "cli_import",
        boundary_path: Optional[str] = None,
        h3_cell: Optional[str] = None,
        modes: list[str] = None,
        **options_kwargs
    ) -> "Config":
        """Create configuration from CLI arguments."""
        options = Options(**{k: v for k, v in options_kwargs.items() if v is not None})
        return cls(
            name=name,
            pbf_path=pbf_path,
            output_path=output_path,
            boundary_path=boundary_path,
            h3_cell=h3_cell,
            modes=modes if modes else ["driving"],
            options=options
        )
    
    def validate(self) -> None:
        """Validate configuration."""
        if not self.pbf_path:
            raise ValueError("pbf_path is required")
        
        pbf_file = Path(self.pbf_path)
        if not pbf_file.exists():
            raise FileNotFoundError(f"PBF file not found: {self.pbf_path}")
        
        if self.boundary_path:
            boundary_file = Path(self.boundary_path)
            if not boundary_file.exists():
                raise FileNotFoundError(f"Boundary file not found: {self.boundary_path}")
    
    def get_db_path(self) -> Path:
        """Get the full path to the output DuckDB file.
        
        If output_path ends with .duckdb, use it as-is.
        Otherwise, treat output_path as a directory and create name.duckdb inside.
        """
        output = Path(self.output_path)
        if output.suffix == '.duckdb':
            return output
        else:
            # output_path is a directory, create name.duckdb inside
            output.mkdir(parents=True, exist_ok=True)
            return output / f"{self.name}.duckdb"
