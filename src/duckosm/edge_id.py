"""Stable, deterministic ``edge_id`` — a content hash of an edge's identity.

    edge_id = (hash(osm_id, source, target, is_reverse) >> 1)::BIGINT      (DuckDB hash)

Because the inputs are OSM way/node ids and a direction flag, the same physical edge gets the
**same id on every rebuild** — so rebuilds don't renumber the graph and downstream consumers
(map-matching, joins) survive without a full re-key.

Reuse in other projects
-----------------------
* The id is reproducible in **any DuckDB**: integer types are interchangeable
  (``INTEGER`` == ``BIGINT``) and ``BOOLEAN`` == ``0/1``, but the **argument order matters**
  (``osm_id, source, target, is_reverse``).
* It is **not** reproducible outside DuckDB (``hash()`` is a DuckDB-internal function), and is
  **not guaranteed identical across major DuckDB versions**. For cross-project id matching
  either pin the DuckDB version, or simply **join on the natural key** and read ``edge_id`` —
  that never depends on the hash.

Three ways to use it elsewhere:
  1. DuckDB SQL — call the persisted macro: ``edge_id_hash(osm_id, source, target, is_reverse)``
     (every duckOSM output db ships it; recreate with :func:`create_edge_id_macro`).
  2. Python — :func:`edge_id_hash` (single) or :func:`edge_id_expr` (a SQL fragment to embed in
     your own query / dataframe projection via ``duckdb.sql``).
  3. Best when you only need the id: join your edges to ``driving.edges`` on
     ``(osm_id, source, target, is_reverse)``.
"""
from __future__ import annotations

MACRO_NAME = "edge_id_hash"


def edge_id_expr(osm_id: str = "osm_id", source: str = "source",
                 target: str = "target", is_reverse: str = "is_reverse") -> str:
    """The SQL expression for the stable edge_id, parameterised by column names / placeholders.

    >>> edge_id_expr()                       # over an edges table
    '(hash(osm_id::BIGINT, source::BIGINT, target::BIGINT, is_reverse::BOOLEAN) >> 1)::BIGINT'
    >>> edge_id_expr('?', '?', '?', '?')     # for a parameterised query
    """
    return (f"(hash({osm_id}::BIGINT, {source}::BIGINT, {target}::BIGINT, "
            f"{is_reverse}::BOOLEAN) >> 1)::BIGINT")


def create_edge_id_macro(con) -> None:
    """(Re)create the persisted ``edge_id_hash(osm_id, source, target, is_reverse)`` macro on the
    connected DuckDB database, so the canonical formula travels with every output db."""
    con.execute(
        f"CREATE OR REPLACE MACRO main.{MACRO_NAME}(osm_id, source, target, is_reverse) AS "
        + edge_id_expr("osm_id", "source", "target", "is_reverse"))


def edge_id_hash(osm_id: int, source: int, target: int, is_reverse: bool, con=None) -> int:
    """Compute the stable edge_id for one edge (uses an in-memory DuckDB if no connection given).

    For bulk work, embed :func:`edge_id_expr` in your own ``duckdb.sql`` over a table/dataframe
    instead of calling this per row.
    """
    import duckdb
    c = con or duckdb.connect()
    return c.execute("SELECT " + edge_id_expr("?", "?", "?", "?"),
                     [int(osm_id), int(source), int(target), bool(is_reverse)]).fetchone()[0]
