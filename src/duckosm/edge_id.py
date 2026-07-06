"""Stable, deterministic ``edge_id`` — a content hash of an edge's identity.

**Current (v2):**

    edge_id = (hash(osm_id, source, target) >> 1)::BIGINT               (DuckDB hash)

**Previous (v1)** — kept for migration / old→new crosswalks:

    edge_id_v1 = (hash(osm_id, source, target, is_reverse) >> 1)::BIGINT

v2 drops ``is_reverse`` from the hash. Direction is already encoded by ``source -> target`` (the
forward and reverse of a two-way road get distinct ids because their endpoints are swapped);
``is_reverse`` only ever served as a uniqueness tiebreaker for self-crossing ways that produced two
edges on the same directed ``(osm_id, source, target)``. Once the graph is segmented so that triple
is **globally unique** — self-loops and two-way antiparallel arcs are split with virtual nodes (see
``GraphSimplifier._split_self_loops`` / ``_split_antiparallel_pairs``) — the flag is redundant in the
hash and comes out. It is retained as a *column* (it still routes ``lanes_fwd``/``lanes_bwd`` and
other along-vs-against-digitisation attributes).

**Migration.** Every ``edges`` row still carries ``osm_id/source/target/is_reverse``, so the v1 id is
recomputable — build an old→new crosswalk with
``edge_id_hash_v1(osm_id, source, target, is_reverse)`` (old) vs
``edge_id_hash(osm_id, source, target)`` (new). Both macros ship on every output db.

Reuse in other projects
-----------------------
* Reproducible in **any DuckDB**: integer types are interchangeable (``INTEGER`` == ``BIGINT``,
  ``BOOLEAN`` == ``0/1``), but the **argument order matters**.
* **Not** reproducible outside DuckDB (``hash()`` is a DuckDB-internal function), and **not
  guaranteed identical across major DuckDB versions**. For cross-project matching either pin the
  DuckDB version, or **join on the natural key** ``(osm_id, source, target)`` and read ``edge_id``.

Three ways to use it elsewhere:
  1. DuckDB SQL — the persisted macros ``edge_id_hash(osm_id, source, target)`` (current) and
     ``edge_id_hash_v1(osm_id, source, target, is_reverse)`` (previous). Recreate with
     :func:`create_edge_id_macro`.
  2. Python — :func:`edge_id_hash` / :func:`edge_id_hash_v1` (single) or :func:`edge_id_expr` /
     :func:`edge_id_expr_v1` (a SQL fragment to embed in your own query).
  3. Best when you only need the id: join your edges to ``<mode>.edges`` on
     ``(osm_id, source, target)``.
"""
from __future__ import annotations

MACRO_NAME = "edge_id_hash"           # current (v2)
MACRO_NAME_V1 = "edge_id_hash_v1"     # previous (v1), kept for migration


def edge_id_expr(osm_id: str = "osm_id", source: str = "source",
                 target: str = "target") -> str:
    """The SQL expression for the current (v2) stable edge_id, parameterised by column names.

    >>> edge_id_expr()                    # over an edges table
    '(hash(osm_id::BIGINT, source::BIGINT, target::BIGINT) >> 1)::BIGINT'
    >>> edge_id_expr('?', '?', '?')       # for a parameterised query
    """
    return f"(hash({osm_id}::BIGINT, {source}::BIGINT, {target}::BIGINT) >> 1)::BIGINT"


def edge_id_expr_v1(osm_id: str = "osm_id", source: str = "source",
                    target: str = "target", is_reverse: str = "is_reverse") -> str:
    """The SQL expression for the PREVIOUS (v1) edge_id — includes ``is_reverse``. Kept so old ids
    remain reproducible for migration / crosswalks."""
    return (f"(hash({osm_id}::BIGINT, {source}::BIGINT, {target}::BIGINT, "
            f"{is_reverse}::BOOLEAN) >> 1)::BIGINT")


def create_edge_id_macro(con) -> None:
    """(Re)create BOTH persisted edge_id macros on the connected DuckDB database, so the canonical
    formula and its predecessor travel with every output db:

    * ``edge_id_hash(osm_id, source, target)`` — current (v2).
    * ``edge_id_hash_v1(osm_id, source, target, is_reverse)`` — previous (v1), for old→new crosswalks.
    """
    con.execute(
        f"CREATE OR REPLACE MACRO main.{MACRO_NAME}(osm_id, source, target) AS "
        + edge_id_expr("osm_id", "source", "target"))
    con.execute(
        f"CREATE OR REPLACE MACRO main.{MACRO_NAME_V1}(osm_id, source, target, is_reverse) AS "
        + edge_id_expr_v1("osm_id", "source", "target", "is_reverse"))


def edge_id_hash(osm_id: int, source: int, target: int, con=None) -> int:
    """Compute the current (v2) stable edge_id for one edge (in-memory DuckDB if no connection given).

    For bulk work, embed :func:`edge_id_expr` in your own ``duckdb.sql`` over a table/dataframe
    instead of calling this per row.
    """
    import duckdb
    c = con or duckdb.connect()
    return c.execute("SELECT " + edge_id_expr("?", "?", "?"),
                     [int(osm_id), int(source), int(target)]).fetchone()[0]


def edge_id_hash_v1(osm_id: int, source: int, target: int, is_reverse: bool, con=None) -> int:
    """Compute the PREVIOUS (v1) edge_id (with ``is_reverse``). For migration / old→new crosswalks."""
    import duckdb
    c = con or duckdb.connect()
    return c.execute("SELECT " + edge_id_expr_v1("?", "?", "?", "?"),
                     [int(osm_id), int(source), int(target), bool(is_reverse)]).fetchone()[0]
