"""``duckosm.fixes``: edit the OSM fixes from Python (docs/proposals/python-sdk.md, section 5.9).

``Fixes(load)`` reads a fixes file (the format of the guide "Fix the OSM data"). ``set``, ``remove``, ``exclude`` and
``drop`` change the rules in memory, and ``save(path)`` writes them. The library chooses no file name, and a load never
writes the loaded file. The Network gives ``net.fixes``, ``net.rebuild(fixes=...)``, ``net.diff(other)`` and ``net.edit()``.
"""
from __future__ import annotations

import os
from pathlib import Path

__all__ = ["Fixes", "diff"]

# the fields of a rule of ``overrides`` (each other keyword of ``set`` is an OSM tag, in ``tags``)
RULE_FIELDS = ("oneway", "lanes", "lanes_forward", "lanes_backward", "layer", "turn_lanes_forward", "turn_lanes_backward",
               "keep_nodes", "exclude_modes", "remove", "tags", "note")


class Fixes(os.PathLike):
    """The rules of a fixes file. ``load``: the file to read (None: no rules). The changes stay in memory until
    ``save(path)``. ``os.fspath(fixes)`` is the saved file (or the loaded file, when nothing changed), so ``fixes=`` of
    each call (``duckosm.build``, ``net.rebuild``, ``duckosm.gis.quack``) takes a ``Fixes``. With changes that are not
    saved, those calls stop. ``save`` writes the whole file: its comments are not kept.

    >>> fixes = duckosm.fixes.Fixes(load="osm_fixes.yaml")
    >>> fixes.set(way=4392632, oneway=True, maxspeed=30)
    >>> fixes.save("osm_fixes_v2.yaml")"""

    def __init__(self, load: str | os.PathLike | None = None):
        import yaml
        self.loaded = Path(load) if load is not None else None
        self.path: Path | None = None          # the save file: the file of the last save
        self.changed = False                   # changes that are not saved
        self.data = (yaml.safe_load(self.loaded.read_text()) or {}) if self.loaded and self.loaded.is_file() else {}

    @classmethod
    def from_text(cls, text: str) -> "Fixes":
        """The rules of the YAML text ``text``, with no loaded file and no save file (``net.fixes``: the rules of the build)."""
        import yaml
        f = cls()
        f.data = yaml.safe_load(text) or {}
        return f

    def __fspath__(self) -> str:
        from duckosm.errors import DuckOSMError
        path = self.path or self.loaded
        if self.changed or path is None:
            raise DuckOSMError("the fixes have no file, or changes that are not saved: save the fixes first: fixes.save('name.yaml')")
        return str(path)

    def __str__(self) -> str:
        return str(self.path or self.loaded or "")

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

    def save(self, path: str | os.PathLike | None = None) -> "Fixes":
        """Write the rules to ``path``. The first save needs a path; a later ``save()`` writes the same file again. The
        path can be the loaded file.

        >>> fixes.save("osm_fixes.yaml")"""
        import yaml

        from duckosm.errors import DuckOSMError
        path = Path(path) if path is not None else self.path
        if path is None:
            raise DuckOSMError("fixes.save() needs a path the first time: fixes.save('name.yaml')")
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(yaml.safe_dump(self.data, sort_keys=False, allow_unicode=True))
        self.path, self.changed = path, False
        return self

    def _change(self) -> "Fixes":
        self.changed = True
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
        return self._change()

    def remove(self, way: int) -> "Fixes":
        """Remove the way ``way`` from the build (no road, no building, no feature).

        >>> fixes.remove(way=4392632)"""
        self._rule(way)["remove"] = True
        return self._change()

    def exclude(self, way: int, modes) -> "Fixes":
        """Remove the way ``way`` from the networks of ``modes`` (``"driving"``, ``"walking"``, ``"cycling"``).

        >>> fixes.exclude(way=4392632, modes=["driving"])"""
        self._rule(way)["exclude_modes"] = [modes] if isinstance(modes, str) else list(modes)
        return self._change()

    def drop(self, way: int) -> "Fixes":
        """Delete the rule of the way ``way`` (the way is as in OSM again)."""
        self.data["overrides"] = [r for r in self.overrides if r.get("osm_id") != way]
        return self._change()

    def rules(self):
        """The rules of ``overrides`` as a DataFrame: one row for each way, one column for each field."""
        import pandas as pd
        return pd.DataFrame(self.overrides)

    def __len__(self) -> int:
        return len(self.overrides)

    def __repr__(self) -> str:
        state = ", not saved" if self.changed else ""
        return f"Fixes(load={str(self.loaded) if self.loaded else None!r}, saved={str(self.path) if self.path else None!r}, {len(self)} rules{state})"

    def _repr_html_(self) -> str:
        from html import escape
        other = [k for k in self.data if k != "overrides" and self.data[k]]
        extra = f"; also {', '.join(escape(k) for k in other)}" if other else ""
        table = self.rules().to_html(index=False, na_rep="") if len(self) else "<p>no rules</p>"
        return f"<div><b>Fixes</b> <code>{escape(str(self))}</code>: {len(self)} rules{', not saved' if self.changed else ''}{extra}{table}</div>"


def diff(net, other, mode=None):
    """The edges that are different in ``other`` (a Network or a path), by ``edge_id``: one row for each edge that was
    added, removed or changed, with the columns ``mode``, ``edge_id``, ``change`` (``"added"``, ``"removed"`` or
    ``"changed"``) and ``changed`` (the columns with a new value). The modes of both files are compared, or ``mode``.
    DuckDB joins the two files (in Rust).

    >>> net.diff(fixed)"""
    import pandas as pd

    from duckosm import _rust
    from duckosm.network import ModeView, Network
    if isinstance(net, ModeView):
        net, mode = net.network, mode or net.mode
    net, other = Network(net), Network(getattr(other, "network", other))
    rows = _rust.fixes_diff(str(net.path), str(other.path), mode)
    return pd.DataFrame(rows, columns=["mode", "edge_id", "change", "changed"])
