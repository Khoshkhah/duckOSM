"""``duckosm.fixes``: edit the OSM fixes from Python (docs/proposals/python-sdk.md, section 5.9).

``Fixes(path)`` opens or makes a fixes file (``osm_fixes.yaml``, the format of the guide "Fix the OSM data"). ``set``,
``remove`` and ``exclude`` add a rule to ``overrides`` and write the file at once, as ``duckosm edit`` does. The
Network gives ``net.fixes``, ``net.rebuild(fixes=...)``, ``net.diff(other)`` and ``net.edit()``.
"""
from __future__ import annotations

import os
from pathlib import Path

__all__ = ["Fixes", "diff"]

# the fields of a rule of ``overrides`` (each other keyword of ``set`` is an OSM tag, in ``tags``)
RULE_FIELDS = ("oneway", "lanes", "lanes_forward", "lanes_backward", "layer", "turn_lanes_forward", "turn_lanes_backward",
               "keep_nodes", "exclude_modes", "remove", "tags", "note")


class Fixes(os.PathLike):
    """A fixes file. It is made when the first rule is added. ``os.fspath(fixes)`` is its path, so ``fixes=`` of each
    call (``duckosm.build``, ``duckosm.get``, ``duckosm.gis.quack``) takes a ``Fixes``. The file is written again at each
    change: its comments are not kept.

    >>> fixes = duckosm.fixes.Fixes("osm_fixes.yaml")
    >>> fixes.set(way=4392632, oneway=True, maxspeed=30)"""

    def __init__(self, path: str | os.PathLike = "osm_fixes.yaml"):
        import yaml
        self.path = Path(path)
        self.data = (yaml.safe_load(self.path.read_text()) or {}) if self.path.is_file() else {}

    def __fspath__(self) -> str:
        return str(self.path)

    def __str__(self) -> str:
        return str(self.path)

    @property
    def overrides(self) -> list[dict]:
        """The rules of ``overrides``, one dict for each way."""
        return self.data.setdefault("overrides", [])

    def _rule(self, way: int) -> dict:
        for rule in self.overrides:
            if rule.get("osm_id") == way:
                return rule
        rule = {"osm_id": int(way)}
        self.overrides.append(rule)
        return rule

    def save(self) -> "Fixes":
        """Write the file."""
        import yaml
        self.path.parent.mkdir(parents=True, exist_ok=True)
        self.path.write_text(yaml.safe_dump(self.data, sort_keys=False, allow_unicode=True))
        return self

    def set(self, way: int, **fields) -> "Fixes":
        """Change the way ``way`` (its OSM id). The fields of a rule (``oneway``, ``lanes``, ``lanes_forward``,
        ``lanes_backward``, ``layer``, ``turn_lanes_forward``, ``turn_lanes_backward``, ``keep_nodes``, ``note``) go in
        the rule; each other keyword is an OSM tag (``maxspeed=30``, ``highway="residential"``). A field of the way that
        was set before gets the new value.

        >>> fixes.set(way=4392632, oneway=True, maxspeed=30)"""
        rule = self._rule(way)
        for key, value in fields.items():
            if key == "tags":
                rule.setdefault("tags", {}).update({k: str(v) for k, v in value.items()})
            elif key in RULE_FIELDS:
                rule[key] = value
            else:
                rule.setdefault("tags", {})[key] = str(value)
        return self.save()

    def remove(self, way: int) -> "Fixes":
        """Remove the way ``way`` from the build (no road, no building, no feature).

        >>> fixes.remove(way=4392632)"""
        self._rule(way)["remove"] = True
        return self.save()

    def exclude(self, way: int, modes) -> "Fixes":
        """Remove the way ``way`` from the networks of ``modes`` (``"driving"``, ``"walking"``, ``"cycling"``).

        >>> fixes.exclude(way=4392632, modes=["driving"])"""
        self._rule(way)["exclude_modes"] = [modes] if isinstance(modes, str) else list(modes)
        return self.save()

    def drop(self, way: int) -> "Fixes":
        """Delete the rule of the way ``way`` (the way is as in OSM again)."""
        self.data["overrides"] = [r for r in self.overrides if r.get("osm_id") != way]
        return self.save()

    def rules(self):
        """The rules of ``overrides`` as a DataFrame: one row for each way, one column for each field."""
        import pandas as pd
        return pd.DataFrame(self.overrides)

    def __len__(self) -> int:
        return len(self.overrides)

    def __repr__(self) -> str:
        return f"Fixes({str(self.path)!r}, {len(self)} rules)"

    def _repr_html_(self) -> str:
        from html import escape
        other = [k for k in self.data if k != "overrides" and self.data[k]]
        extra = f"; also {', '.join(escape(k) for k in other)}" if other else ""
        table = self.rules().to_html(index=False, na_rep="") if len(self) else "<p>no rules</p>"
        return f"<div><b>Fixes</b> <code>{escape(str(self.path))}</code>: {len(self)} rules{extra}{table}</div>"


# the columns that diff compares (those of both files); geometry is compared as WKB
_SKIP = {"edge_id", "geometry", "from_cell", "to_cell", "lca_res", "from_tile", "to_tile", "lca_zoom"}


def diff(net, other, mode=None):
    """The edges that are different in ``other`` (a Network or a path), by ``edge_id``: one row for each edge that was
    added, removed or changed, with the columns ``mode``, ``edge_id``, ``change`` (``"added"``, ``"removed"`` or
    ``"changed"``) and ``changed`` (the columns with a new value). The modes of both files are compared, or ``mode``.
    DuckDB joins the two files.

    >>> net.diff(fixed)"""
    import pandas as pd
    from duckosm.network import ModeView, Network
    if isinstance(net, ModeView):
        net, mode = net.network, mode or net.mode
    net, other = Network(net), Network(getattr(other, "network", other))
    modes = [mode] if mode else [m for m in net.modes if m in other.modes]
    con = net._connect()
    try:
        con.execute(f"ATTACH '{str(other.path).replace(chr(39), chr(39) * 2)}' AS other (READ_ONLY)")
        frames = []
        for m in modes:
            cols = [c for c, in con.execute(
                "SELECT column_name FROM duckdb_columns() WHERE schema_name = ? AND table_name = 'edges' "
                "AND database_name = current_database() INTERSECT SELECT column_name FROM duckdb_columns() "
                "WHERE schema_name = ? AND table_name = 'edges' AND database_name = 'other'", [m, m]).fetchall()]
            cols = sorted(c for c in cols if c not in _SKIP) + ["geometry"]
            changed = ", ".join(
                f"CASE WHEN ST_AsWKB(a.geometry) IS DISTINCT FROM ST_AsWKB(b.geometry) THEN 'geometry' END" if c == "geometry"
                else f"CASE WHEN a.\"{c}\" IS DISTINCT FROM b.\"{c}\" THEN '{c}' END" for c in cols)
            frames.append(con.execute(f"""
                SELECT '{m}' AS mode, coalesce(a.edge_id, b.edge_id) AS edge_id,
                       CASE WHEN a.edge_id IS NULL THEN 'added' WHEN b.edge_id IS NULL THEN 'removed' ELSE 'changed' END AS change,
                       CASE WHEN a.edge_id IS NULL OR b.edge_id IS NULL THEN []
                            ELSE list_filter([{changed}], x -> x IS NOT NULL) END AS changed
                FROM "{m}".edges a FULL JOIN other."{m}".edges b ON a.edge_id = b.edge_id
                WHERE a.edge_id IS NULL OR b.edge_id IS NULL OR len(list_filter([{changed}], x -> x IS NOT NULL)) > 0
                ORDER BY change, edge_id""").df())
    finally:
        con.close()
    return pd.concat(frames, ignore_index=True) if frames else pd.DataFrame(columns=["mode", "edge_id", "change", "changed"])
