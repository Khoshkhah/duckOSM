"""
Configuration handling for duckOSM.

Supports two source modes:
  source.type: pbf     -> build a network from OSM (optionally clipped by a boundary)
  source.type: duckdb  -> clip an area out of an existing duckOSM db, preserving edge_ids

The old flat keys (pbf_path, boundary_path, h3_cell) are still accepted as shorthand for
source.type: pbf. See templates/config.yaml (`duckosm init-config`) and docs/reference/configuration.md.
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
    simplify: bool = True                  # contract degree-2 nodes. Must match the shipped configs: the simplifier is what writes edges.refs, which extract_restrictions requires (and the unsimplified edges table is a straight-line approximation, not a supported product)
    merge_segments: bool = True            # merge same-road degree-2 chains (across osm_id); writes <mode>.edge_id_map. Default on; set false for a no-merge build
    global_junctions: bool = True          # segment every mode at one mode-agnostic road-junction set (main.global_junctions) so a road keeps the SAME edge_id across driving/walking/cycling. See docs/design/global_junction_segmentation.md
    functional_types: bool = True          # add walk_type (walking) / cycle_type (cycling) functional-class columns from OSM sub-tags. See docs/design/walk_cycle_type.md
    cycling_dismount: bool = True          # cycling also gets footway/pedestrian as dismount=TRUE edges (push-the-bike: walking speed, bidirectional) so cycleways connected only via them survive the component clean-up. See docs/design/cycling_dismount_edges.md
    process_speeds: bool = True
    extract_restrictions: bool = True
    calculate_costs: bool = True
    memory_limit: Optional[str] = None
    threads: Optional[int] = None
    simplify_batches: int = 0
    boundary_cells: bool = False
    boundary_cell_resolutions: Optional[list[int]] = None

    build_features: bool = True            # build the features.* base-map layers (water, land, buildings, POIs…) for base maps (mapstyle); PBF builds only
    clip_strategy: Optional[str] = None    # osmium extract strategy: 'smart' | 'complete_ways' | 'simple'. Default: 'smart' when build_features (completes multipolygon relations — rivers/landcover/coastlines), else 'complete_ways'


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
    connectivity_rescue: bool = True               # reconnect dangling path ends (PathConnector,
    connect_snap_m: float = 10.0                   #   cycling/walking) within this many metres,
    strongly_connected: bool = False               #   BEFORE the component filter. See docs/design/connectivity_repair.md


@dataclass
class Validation:
    """Build-time invariant checks."""
    enabled: bool = False
    fail_on_error: bool = True
    assert_single_component: bool = True
    assert_no_stranded_named: bool = True
    assert_edge_id_stable: bool = False
    assert_unique_node_id: bool = True             # no duplicate node_id in nodes (virtual incl.)
    assert_way_length_conserved: bool = True       # no interior stretch of a kept way silently
                                                   #   deleted in favour of a parallel arc — see
                                                   #   docs/design/split_same_direction_parallels.md
    warn_layer_without_structure: bool = True      # layer≠0 but no bridge/tunnel tag (OSM tagging
                                                   #   smell; warn-only, never fails the build)


@dataclass
class Multimodal:
    """Intermodal (walk↔drive↔cycle) transfer graph — the `mm.*` tables. See docs/design/multimodal.md.

    When ``enabled``, a post-mode step builds ``mm.edges`` (the per-mode ``edges`` unioned with a
    ``mode`` column) and ``mm.transfers`` (the mode-change arcs), so a single trip can switch mode
    (park-and-ride). Needs ≥2 modes including ``walking`` (the pedestrian hub).
    """
    enabled: bool = False
    transfer_s: float = 60.0                        # flat transfer penalty (seconds), v1 coarse
    realistic: bool = False                         # v2 park-and-ride (not yet implemented)
    # optional per-direction overrides, keyed "from->to" e.g. {"walking->driving": 60,
    # "driving->walking": 30}; any pair not listed falls back to `transfer_s`.
    transfer_costs: Optional[dict] = None
    categories: Optional[list] = None               # v2 OSM POI categories (reserved)


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
    # Rules file of fixes for known OSM errors (way rules + turn rules; docs: "Fix OSM errors"),
    # applied only when named here or with `build --fixes`. No default: a build never picks up a
    # rules file just because one sits in the working directory.
    osm_overrides: Optional[str] = None
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
    multimodal: Multimodal = field(default_factory=Multimodal)

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
        multimodal = Multimodal(**_only_known(Multimodal, data.get("multimodal")))

        return cls(
            name=data.get("name", "default"),
            pbf_path=source.pbf_path or data.get("pbf_path", "") or "",
            output_path=data.get("output_path", "output.duckdb"),
            osm_overrides=data.get("osm_overrides"),
            boundary_path=boundary.path or data.get("boundary_path"),
            h3_cell=boundary.h3_cell or data.get("h3_cell"),
            modes=data.get("modes") or ["driving"],
            options=options, source=source, boundary=boundary,
            clip=clip, validation=validation, report=report, viz=viz,
            multimodal=multimodal,
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
        osm_overrides: Optional[str] = None,
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
            osm_overrides=osm_overrides,
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
            # Restrictions are mapped to edges through `refs` (the stitched node list), and only
            # the simplifier writes that column — so this pair would die deep in the build with a
            # bare binder error. Fail here instead, before any work. (Options are mostly inert for
            # a duckdb clip, hence the pbf-only scope.)
            if self.options.extract_restrictions and not self.options.simplify:
                raise ValueError(
                    "options.extract_restrictions requires options.simplify: turn restrictions are "
                    "mapped onto edges via edges.refs, which only the simplifier writes. "
                    "Set simplify: true, or extract_restrictions: false."
                )
        if self.osm_overrides and not Path(self.osm_overrides).exists():
            raise FileNotFoundError(f"OSM fixes file not found: {self.osm_overrides}")
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
