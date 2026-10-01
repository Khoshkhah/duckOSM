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
import logging

import yaml


def _only_known(cls, data: dict) -> dict:
    """Keep only keys that are fields of dataclass `cls`; warn about the others (a typo, or a key
    that no longer exists), so they aren't ignored silently."""
    known = {f.name for f in fields(cls)}
    unknown = sorted(set(data or {}) - known)
    if unknown:
        logging.getLogger("duckosm").warning(
            f"config: ignoring unknown {cls.__name__.lower()} key(s): {', '.join(unknown)}")
    return {k: v for k, v in (data or {}).items() if k in known}


# every key a config file may have at the top level (sections, and the flat shorthands)
_TOP_LEVEL_KEYS = {"name", "output_path", "osm_overrides", "modes", "options", "source", "boundary",
                   "clip", "validation", "report", "viz", "multimodal",
                   "pbf_path", "boundary_path", "h3_cell"}


@dataclass
class Options:
    """Processing options (used by source.type: pbf; mostly inert for a duckdb clip)."""
    build_graph: bool = True
    h3_indexing: bool = True
    h3_resolution: int = 8
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
    sea: Optional[str] = "overture"        # features.ocean, the sea (OSM only has coastline lines): 'overture' = Overture Maps' ocean polygons, read for the area from S3 (needs the network; offline it is skipped), or false. See docs/design/sea.md
    clip_strategy: Optional[str] = None    # osmium extract strategy: 'smart' | 'complete_ways' | 'simple'. Default: 'smart' when build_features (completes multipolygon relations — rivers/landcover/coastlines), else 'complete_ways'


@dataclass
class Source:
    """Where the network comes from."""
    type: str = "pbf"                              # 'pbf' | 'duckdb'
    # type: pbf
    pbf_path: Optional[str] = None
    # type: duckdb (derive this area by clipping a parent build; edge ids are copied as they are)
    source_db: Optional[str] = None


@dataclass
class Boundary:
    """The clip region."""
    path: Optional[str] = None                     # a GeoJSON file; wins over the three below
    place: Optional[str] = None                    # a place name: the PBF's borders, else Nominatim
    bbox: Optional[list[float]] = None             # [west, south, east, north], degrees
    h3_cell: Optional[str] = None                  # an H3 cell id (hex)
    buffer_m: float = 0.0


@dataclass
class Clip:
    """Clip / clean-up behaviour."""
    predicate: str = "intersects"                  # 'within' | 'intersects' | 'centroid'
    keep_largest_component: bool = True
    min_component_edges: int = 1
    connectivity_rescue: bool = True               # reconnect dangling path ends (PathConnector,
    connect_snap_m: float = 10.0                   #   cycling/walking) within this many metres,
                                                   #   BEFORE the component filter. See docs/design/connectivity_repair.md


@dataclass
class Validation:
    """Build-time invariant checks."""
    enabled: bool = False
    fail_on_error: bool = True
    assert_single_component: bool = True
    assert_no_stranded_named: bool = True
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
    # optional per-direction overrides, keyed "from->to" e.g. {"walking->driving": 60,
    # "driving->walking": 30}; any pair not listed falls back to `transfer_s`.
    transfer_costs: Optional[dict] = None


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
        # a misspelled section (`clipp:`) would otherwise drop its settings without a word
        unknown = sorted(set(data) - _TOP_LEVEL_KEYS)
        if unknown:
            logging.getLogger("duckosm").warning(f"config: ignoring unknown key(s): {', '.join(unknown)}")

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

    def materialize_boundary(self) -> None:
        """Turn ``boundary.bbox`` / ``h3_cell`` / ``place`` into a GeoJSON file next to the output
        (``<name>.boundary.geojson``), so every later step treats all kinds of boundary alike. A
        ``boundary.path`` wins; with none of them, nothing happens."""
        b = self.boundary
        cell = b.h3_cell or self.h3_cell
        if self.effective_boundary_path or not (b.bbox or cell or b.place):
            return
        from duckosm.area import bbox_geojson, find_boundary, h3_geojson, write_boundary
        if b.bbox:
            geom, info = bbox_geojson(b.bbox), {"name": self.name, "source": "bbox"}
        elif cell:
            geom, info = h3_geojson(cell), {"name": self.name, "source": f"h3 {cell}"}
        else:
            geom, info = find_boundary(b.place, pbf=self.effective_pbf_path)
        out = self.get_db_path().with_name(f"{self.name}.boundary.geojson")
        write_boundary(geom, info, out)
        b.path = str(out)

    def validate(self) -> None:
        # values with a fixed set of choices: a typo would otherwise be accepted (an unknown
        # clip.predicate quietly became intersects)
        for key, val, allowed in (
                ("modes", self.modes, {"driving", "walking", "cycling"}),
                ("options.clip_strategy", [self.options.clip_strategy or "smart"], {"smart", "complete_ways", "simple"}),
                ("clip.predicate", [self.clip.predicate], {"intersects", "within", "centroid"}),
                ("options.sea", [self.options.sea or "false"], {"overture", "false"})):
            bad = [v for v in val if v not in allowed]
            if bad:
                raise ValueError(f"{key}: {', '.join(map(str, bad))} is not one of {', '.join(sorted(allowed))}")
        self.materialize_boundary()
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
