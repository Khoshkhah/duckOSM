"""
Multimodal (intermodal) transfer-graph builder — the ``mm.*`` tables.

duckOSM builds one **independent** graph per mode (``driving`` / ``walking`` / ``cycling``), each in
its own schema, with no arcs connecting them. This processor stitches them into a single **layered
graph** so a trip can *switch mode mid-route* (walk → drive → walk / park-and-ride):

- ``mm.edges``     — every present mode's ``edges`` unioned with a ``mode`` column. Key
                     ``(mode, edge_id)`` (``edge_id`` collides across modes — no mode in the hash).
- ``mm.transfers`` — the mode-change arcs: ``(node_id, from_mode, to_mode, cost_s, kind)``. A
                     transfer connects the *same physical junction* in two layers (``node_id`` is
                     the raw OSM node id, so it is identical across modes — a plain equality join).

Everything is weighted in **seconds** (``edges.cost_s``), so the flat transfer penalty adds directly
to travel cost. Route across the result with :func:`duckosm.routing.route_multimodal`.

Two fidelity levels (see docs/multimodal.md):

* **v1 coarse** (implemented, default): a transfer at *every* junction shared by walking and a
  vehicular mode. Good for reachability/coverage; unrealistic for trip planning (you can "grab a
  car" at any junction). Needs no extra OSM extraction.
* **v2 realistic** (:class:`TransferPointExtractor`, stubbed): restrict car ingress/egress to
  parking POIs (``amenity=parking`` / ``park_ride`` / ``bicycle_parking``). Not yet implemented.

Transfers always go **through walking** (the pedestrian hub): only ``walking↔driving`` and
``walking↔cycling`` rows are emitted, never ``driving↔cycling`` directly — you don't teleport
car→bike, and this also brackets every vehicular leg with walking.
"""

import logging

from duckosm.processors.base import BaseProcessor

logger = logging.getLogger("duckosm")

# The pedestrian layer every transfer routes through.
WALKING = "walking"

# Per-vehicular-mode transfer kinds: (walking->vehicular, vehicular->walking).
TRANSFER_KINDS = {
    "driving": ("park", "retrieve"),
    "cycling": ("bike_park", "bike_unpark"),
}
_DEFAULT_KINDS = ("enter", "exit")

# Columns carried into mm.edges (must exist in every mode's `edges`; all do — see graph_builder).
_EDGE_COLS = ["edge_id", "source", "target", "cost_s", "length_m", "highway", "name", "geometry"]

# Schemas that are never transport-mode layers.
_NON_MODE_SCHEMAS = ("information_schema", "pg_catalog", "main", "raw")


class MultimodalBuilder(BaseProcessor):
    """Build the ``mm`` schema (``mm.edges`` + ``mm.transfers``) from the per-mode schemas.

    Parameters
    ----------
    con : DuckDB connection to a built duckOSM db (must be writable).
    transfer_s : flat transfer penalty in seconds (v1 coarse default).
    transfer_costs : optional per-direction overrides, keyed ``"from->to"`` e.g.
        ``{"walking->driving": 60, "driving->walking": 30}``; any pair not listed uses ``transfer_s``.
    realistic : v2 park-and-ride (restrict transfers to parking POIs). Not yet implemented — raises.
    schema : target schema name (default ``"mm"``).
    """

    def __init__(self, con, transfer_s: float = 60.0, transfer_costs: dict | None = None,
                 realistic: bool = False, schema: str = "mm"):
        super().__init__(con)
        self.transfer_s = float(transfer_s)
        self.transfer_costs = transfer_costs or {}
        self.realistic = realistic
        self.schema = schema
        self.stats = {}

    # -- public API -----------------------------------------------------------------------------
    def run(self) -> dict:
        """Build ``mm.edges`` + ``mm.transfers``. Returns a small stats dict (edge/transfer counts).

        A no-op (empty ``mm.*``) when there is nothing to bridge (<2 modes, or no ``walking``);
        it logs a warning and still creates the (empty) tables so downstream reads don't error.
        """
        self.con.execute("INSTALL spatial; LOAD spatial;")
        modes = self._discover_modes()
        logger.info(f"[multimodal] modes present: {', '.join(modes) or 'none'}")

        self.con.execute(f"CREATE SCHEMA IF NOT EXISTS {self.schema}")
        self._build_edges(modes)

        vehicular = [m for m in modes if m != WALKING]
        can_bridge = (WALKING in modes) and vehicular
        self._create_transfers_table()
        if not can_bridge:
            logger.warning(
                "[multimodal] nothing to bridge — need `walking` plus >=1 vehicular mode "
                f"(have: {modes or 'none'}); mm.transfers left empty")
            self.stats["transfer_count"] = 0
            return self.stats

        if self.realistic:
            # v2: restrict transfers to parking POIs. Wired but not built.
            TransferPointExtractor(self.con, schema=self.schema).run()

        self._build_transfers_coarse(vehicular)

        self.stats["transfer_count"] = self.con.execute(
            f"SELECT COUNT(*) FROM {self.schema}.transfers").fetchone()[0]
        logger.info(f"[multimodal] mm.edges: {self.stats.get('edge_count', 0):,} edges "
                    f"({len(modes)} modes), mm.transfers: {self.stats['transfer_count']:,} arcs")
        return self.stats

    # -- steps ----------------------------------------------------------------------------------
    def _discover_modes(self) -> list[str]:
        """Present mode layers: schemas that own an `edges` table (excluding non-mode + target)."""
        excl = set(_NON_MODE_SCHEMAS) | {self.schema}
        rows = self.con.execute(
            "SELECT DISTINCT schema_name FROM duckdb_tables() "
            "WHERE table_name = 'edges' ORDER BY schema_name").fetchall()
        return [r[0] for r in rows if r[0] not in excl]

    def _build_edges(self, modes: list[str]) -> None:
        """``mm.edges`` = per-mode ``edges`` unioned with a ``mode`` column, key ``(mode, edge_id)``."""
        if not modes:
            # No layers at all — create an empty, correctly-typed table and stop.
            self.con.execute(
                f"CREATE OR REPLACE TABLE {self.schema}.edges AS "
                "SELECT NULL::VARCHAR AS mode, " +
                ", ".join(f"NULL AS {c}" for c in _EDGE_COLS) + " WHERE FALSE")
            self.stats["edge_count"] = 0
            return

        self._require_cost_s(modes[0])
        cols = ", ".join(_EDGE_COLS)
        parts = [f"SELECT '{m}' AS mode, {cols} FROM {m}.edges" for m in modes]
        self.con.execute(
            f"CREATE OR REPLACE TABLE {self.schema}.edges AS\n" + "\nUNION ALL\n".join(parts))
        self.con.execute(
            f"CREATE INDEX IF NOT EXISTS idx_mm_edges_src "
            f"ON {self.schema}.edges(mode, source)")
        self.stats["edge_count"] = self.con.execute(
            f"SELECT COUNT(*) FROM {self.schema}.edges").fetchone()[0]

    def _require_cost_s(self, mode: str) -> None:
        """Fail early with a clear message if the network has no travel-time costs."""
        cols = [r[1] for r in self.con.execute(
            f"PRAGMA table_info('{mode}.edges')").fetchall()]
        if "cost_s" not in cols:
            raise ValueError(
                "multimodal routing is weighted in seconds but `edges.cost_s` is missing — "
                "rebuild with options.calculate_costs = true (and process_speeds).")

    def _create_transfers_table(self) -> None:
        self.con.execute(f"""
            CREATE OR REPLACE TABLE {self.schema}.transfers (
                node_id   BIGINT,
                from_mode VARCHAR,
                to_mode   VARCHAR,
                cost_s    DOUBLE,
                kind      VARCHAR
            )""")

    def _cost(self, from_mode: str, to_mode: str) -> float:
        """Transfer penalty for a direction: per-direction override, else the flat default."""
        return float(self.transfer_costs.get(f"{from_mode}->{to_mode}", self.transfer_s))

    def _build_transfers_coarse(self, vehicular: list[str]) -> None:
        """v1: emit a transfer at every junction shared by walking and each vehicular mode.

        Both directions (walking->vehicular and vehicular->walking), each with its own cost/kind.
        The shared junctions come from ``walking.nodes INTERSECT <vehicular>.nodes`` on ``node_id``
        (identical raw-OSM ids across modes — junctions survive simplification in every layer).
        """
        for v in vehicular:
            enter_kind, exit_kind = TRANSFER_KINDS.get(v, _DEFAULT_KINDS)
            shared = (f"(SELECT node_id FROM {WALKING}.nodes "
                      f"INTERSECT SELECT node_id FROM {v}.nodes)")
            # walking -> vehicular
            self.con.execute(f"""
                INSERT INTO {self.schema}.transfers
                SELECT node_id, '{WALKING}', '{v}', {self._cost(WALKING, v)}, '{enter_kind}'
                FROM {shared}""")
            # vehicular -> walking
            self.con.execute(f"""
                INSERT INTO {self.schema}.transfers
                SELECT node_id, '{v}', '{WALKING}', {self._cost(v, WALKING)}, '{exit_kind}'
                FROM {shared}""")
            n = self.con.execute(
                f"SELECT COUNT(*) FROM {shared}").fetchone()[0]
            logger.info(f"[multimodal] {WALKING}<->{v}: {n:,} shared junctions "
                        f"-> {2 * n:,} transfers")


class TransferPointExtractor(BaseProcessor):
    """**v2 (park-and-ride) — NOT yet implemented.**

    Realistic intermodal routing restricts car ingress/egress to *parking*, not to every shared
    junction. This class is the placeholder for that: extract the parking POIs from raw OSM and snap
    each to the nearest node in every relevant mode, then generate ``mm.transfers`` only there.

    Sketch (for when it is built)::

        -- 1. pull point features from raw OSM (nodes + way centroids)
        CREATE TABLE mm.transfer_points AS
        SELECT osm_id AS point_id,
               tags['amenity'] AS category,      -- 'parking' | 'parking_entrance' | 'bicycle_parking'
               ST_Point(lon, lat) AS geom
        FROM raw.nodes
        WHERE tags['amenity'] IN ('parking', 'parking_entrance')
           OR tags['park_ride'] IS NOT NULL
           OR tags['amenity'] = 'bicycle_parking';
        -- 2. snap each point to the nearest node per relevant mode (ST_Distance / NN on nodes.geom);
        --    walk_node and drive_node may be DIFFERENT physical nodes (POIs aren't on the graph).
        -- 3. emit mm.transfers only at those snapped nodes.
    """

    def __init__(self, con, schema: str = "mm"):
        super().__init__(con)
        self.schema = schema

    def run(self) -> None:
        raise NotImplementedError(
            "realistic (v2 park-and-ride) transfers are not yet implemented; "
            "omit --realistic / multimodal.realistic for the coarse v1 build.")
