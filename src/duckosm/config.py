"""
Configuration handling for duckOSM.

Supports two source modes:
  source.type: pbf     -> build a network from OSM (optionally clipped by a boundary)
  source.type: duckdb  -> clip an area out of an existing duckOSM db, preserving edge_ids

The old flat keys (pbf_path, boundary_path, h3_cell) are still accepted as shorthand for
source.type: pbf. See config/template.yaml and PLAN.md for the full schema.
"""

from dataclasses import dataclass, field, fields
from pathlib import Path
from typing import Optional
import yaml


def _only_known(cls, data: dict) -> dict:
    """Keep only keys that are fields of dataclass `cls` (ignore extras like report/viz)."""
    known = {f.name for f in fields(cls)}
    return {k: v for k, v in (data or {}).items() if k in known}


@dataclass
class Options:
    """Processing options (used by source.type: pbf; mostly inert for a duckdb clip)."""
    build_graph: bool = True
    h3_indexing: bool = True
    h3_resolution: int = 8
    simplify: bool = False
    merge_segments: bool = False           # contract degree-2 chains of SAME-osm_id segments
    process_speeds: bool = True
    extract_restrictions: bool = True
    calculate_costs: bool = True
    memory_limit: Optional[str] = None
    threads: Optional[int] = None
    simplify_batches: int = 0
    boundary_cells: bool = False
    boundary_cell_resolutions: Optional[list[int]] = None
    timezone: bool = False


@dataclass
class Source:
    """Where the network comes from."""
    type: str = "pbf"                              # 'pbf' | 'duckdb'
    # type: pbf
    pbf_path: Optional[str] = None
    country: Optional[str] = None                  # Geofabrik code (auto-download) — reserved
    country_url: Optional[str] = None
    # type: duckdb (derive this area by clipping a parent build)
    source_db: Optional[str] = None
    source_modes: Optional[list[str]] = None
    preserve_edge_ids: bool = True


@dataclass
class Boundary:
    """The clip region."""
    path: Optional[str] = None
    place: Optional[str] = None                    # Nominatim — reserved
    bbox: Optional[list[float]] = None
    h3_cell: Optional[str] = None
    buffer_m: float = 0.0


@dataclass
class Clip:
    """Clip / clean-up behaviour."""
    predicate: str = "intersects"                  # 'within' | 'intersects' | 'centroid'
    keep_largest_component: bool = True
    min_component_edges: int = 1
    connectivity_rescue: bool = True
    strongly_connected: bool = False


@dataclass
class Validation:
    """Build-time invariant checks."""
    enabled: bool = False
    fail_on_error: bool = True
    assert_single_component: bool = True
    assert_no_stranded_named: bool = True
    assert_edge_id_stable: bool = False


@dataclass
class Report:
    """Build report (reports/<name>_<ts>.{md,html})."""
    enabled: bool = False


@dataclass
class Viz:
    """roadstyle network visualization (reports/<name>_network.html)."""
    enabled: bool = False
    basemap: str = "voyager"


@dataclass
class Config:
    """Configuration for a duckOSM build."""

    name: str = "default"
    pbf_path: str = ""                             # back-compat (== source.pbf_path)
    output_path: str = "output.duckdb"
    boundary_path: Optional[str] = None            # back-compat (== boundary.path)
    h3_cell: Optional[str] = None
    modes: list[str] = field(default_factory=lambda: ["driving"])
    options: Options = field(default_factory=Options)
    source: Source = field(default_factory=Source)
    boundary: Boundary = field(default_factory=Boundary)
    clip: Clip = field(default_factory=Clip)
    validation: Validation = field(default_factory=Validation)
    report: Report = field(default_factory=Report)
    viz: Viz = field(default_factory=Viz)

    # ---- effective accessors (unify the new blocks with the back-compat flat keys) ----
    @property
    def source_type(self) -> str:
        return self.source.type or "pbf"

    @property
    def effective_pbf_path(self) -> str:
        return self.source.pbf_path or self.pbf_path or ""

    @property
    def effective_boundary_path(self) -> Optional[str]:
        return self.boundary.path or self.boundary_path

    @property
    def source_db(self) -> Optional[str]:
        return self.source.source_db

    @classmethod
    def from_yaml(cls, path: str) -> "Config":
        with open(path, "r") as f:
            data = yaml.safe_load(f) or {}

        options = Options(**_only_known(Options, data.get("options")))
        if data.get("source"):
            source = Source(**_only_known(Source, data["source"]))
        else:
            source = Source(type="pbf", pbf_path=data.get("pbf_path"))
        if data.get("boundary"):
            boundary = Boundary(**_only_known(Boundary, data["boundary"]))
        else:
            boundary = Boundary(path=data.get("boundary_path"), h3_cell=data.get("h3_cell"))
        clip = Clip(**_only_known(Clip, data.get("clip")))
        validation = Validation(**_only_known(Validation, data.get("validation")))
        report = Report(**_only_known(Report, data.get("report")))
        viz = Viz(**_only_known(Viz, data.get("viz")))

        return cls(
            name=data.get("name", "default"),
            pbf_path=source.pbf_path or data.get("pbf_path", "") or "",
            output_path=data.get("output_path", "output.duckdb"),
            boundary_path=boundary.path or data.get("boundary_path"),
            h3_cell=boundary.h3_cell or data.get("h3_cell"),
            modes=data.get("modes") or ["driving"],
            options=options, source=source, boundary=boundary,
            clip=clip, validation=validation, report=report, viz=viz,
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
        source_db: Optional[str] = None,
        source_type: Optional[str] = None,
        **options_kwargs
    ) -> "Config":
        options = Options(**{k: v for k, v in options_kwargs.items() if v is not None})
        stype = source_type or ("duckdb" if source_db else "pbf")
        source = Source(type=stype, pbf_path=pbf_path or None, source_db=source_db)
        boundary = Boundary(path=boundary_path, h3_cell=h3_cell)
        return cls(
            name=name,
            pbf_path=pbf_path or "",
            output_path=output_path,
            boundary_path=boundary_path,
            h3_cell=h3_cell,
            modes=modes if modes else ["driving"],
            options=options, source=source, boundary=boundary,
        )

    def validate(self) -> None:
        if self.source_type == "duckdb":
            if not self.source_db:
                raise ValueError("source.source_db is required for source.type: duckdb")
            if not Path(self.source_db).exists():
                raise FileNotFoundError(f"source_db not found: {self.source_db}")
            if not self.effective_boundary_path:
                raise ValueError("a boundary is required to clip from a duckdb source")
        else:
            if not self.effective_pbf_path:
                raise ValueError("pbf_path (or source.pbf_path) is required")
            if not Path(self.effective_pbf_path).exists():
                raise FileNotFoundError(f"PBF file not found: {self.effective_pbf_path}")
        if self.effective_boundary_path and not Path(self.effective_boundary_path).exists():
            raise FileNotFoundError(f"Boundary file not found: {self.effective_boundary_path}")

    def get_db_path(self) -> Path:
        """Full path to the output DuckDB file.

        If output_path ends with .duckdb use it as-is; otherwise treat it as a directory
        and create <name>.duckdb inside.
        """
        output = Path(self.output_path)
        if output.suffix == ".duckdb":
            return output
        output.mkdir(parents=True, exist_ok=True)
        return output / f"{self.name}.duckdb"
