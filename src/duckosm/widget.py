"""``duckosm.AreaPicker``: a Jupyter widget to choose an area on a map and get its OSM file (docs/start/area-picker.md).

The widget is an anywidget (``pip install "duckosm[widget]"``): ``templates/area_picker.js`` is the map and the
controls, this class is the state. The work is the program ``duckosm get`` (:func:`duckosm.get`): ``--dry-run`` for
the boundary and the extract before the download, then the download and the cut.
"""

import json
import tempfile
import threading
from pathlib import Path

import anywidget
import traitlets


def get_args(area):
    """The arguments of :func:`duckosm.get` for the widget's ``area``: ``{"mode": "osm_id", "osm_id": 2220322}``,
    ``{"mode": "place", "place": "Monaco"}``, ``{"mode": "bbox", "bbox": [w, s, e, n]}``, ``{"mode": "h3",
    "h3_cell": "883969a40bfffff"}`` or ``{"mode": "boundary", "geojson": {...}}``. A ``name`` goes to the file names."""
    mode = area.get("mode")
    if mode == "osm_id":
        kw = {"osm_id": int(area["osm_id"])}
    elif mode == "place":
        kw = {"place": str(area["place"])}
    elif mode == "bbox":
        w, s, e, n = (float(v) for v in area["bbox"])
        if not (-180 <= w < e <= 180 and -90 <= s < n <= 90):
            raise ValueError(f"the box {area['bbox']} is not west < east, south < north in degrees")
        kw = {"bbox": (w, s, e, n)}
    elif mode == "h3":
        kw = {"h3_cell": str(area["h3_cell"])}
    elif mode == "boundary":
        kw = {"boundary": area["geojson"]}
    else:
        raise ValueError(f"no area: choose one on the map (mode {mode!r})")
    if area.get("name"):
        kw["name"] = area["name"]
    return kw


class AreaPicker(anywidget.AnyWidget):
    """Choose an area on a map, then click **Get** (or call :meth:`get`): ``picker.result`` is the
    :class:`duckosm.Area`. ``out``, ``fixes`` and ``cache`` go to :func:`duckosm.get`."""

    _esm = Path(__file__).parent / "templates" / "area_picker.js"
    area = traitlets.Dict().tag(sync=True)      # the choice, see get_args
    plan = traitlets.Dict().tag(sync=True)      # the dry run: name, boundary, extract; or {"error": ...}
    status = traitlets.Unicode().tag(sync=True)
    busy = traitlets.Bool(False).tag(sync=True)

    def __init__(self, out=".", fixes=None, cache=None, **kwargs):
        super().__init__(**kwargs)
        self.out, self.fixes, self.cache = out, fixes, cache
        self.result = None
        self._getting = None      # the thread of the Get button while it runs
        self._seq = 0
        self.on_msg(self._on_msg)

    def _kwargs(self):
        kw = get_args(self.area)
        if isinstance(kw.get("boundary"), dict):
            # TODO: give the dict to get() when its boundary= takes GeoJSON (the level 0 follow-up PR)
            f = tempfile.NamedTemporaryFile("w", suffix=".geojson", prefix="area_", delete=False)
            json.dump(kw["boundary"], f)
            f.close()
            kw["boundary"] = f.name
            kw.setdefault("name", "area")
        return {**kw, "cache": self.cache}

    @traitlets.observe("area")
    def _on_area(self, change):
        self._seq += 1
        seq, self.plan, self.status = self._seq, {}, "finding the boundary and the extract ..."
        self.result = None        # the result of the area before

        def run():
            from duckosm import get
            try:
                plan = get(**self._kwargs(), dry_run=True)
            except Exception as e:  # noqa: BLE001 -- the widget shows the error
                plan = {"error": str(e)}
            if seq == self._seq:      # a newer choice came first: this one is old
                self.plan, self.status = plan, ""

        threading.Thread(target=run, daemon=True).start()

    def get(self):
        """``duckosm.get`` for the area: the download (when the extract is not in the cache) and the cut. Returns the
        :class:`duckosm.Area`, also in ``self.result``."""
        from duckosm import get
        if self._getting and self._getting.is_alive() and self._getting is not threading.current_thread():
            # Get was clicked: wait for that run, do not start a second one
            self._getting.join()
            if self.result is None:
                raise RuntimeError(self.status)
            return self.result
        self.busy = True
        try:
            self.result = get(**self._kwargs(), out=self.out, fixes=self.fixes,
                              progress=lambda line: setattr(self, "status", line))
            self.status = f"wrote {self.result.pbf} and {self.result.boundary}"
            return self.result
        except Exception as e:  # noqa: BLE001
            self.status = f"error: {e}"
            raise
        finally:
            self.busy = False

    def _on_msg(self, _, content, buffers):
        if content.get("type") == "get" and not self.busy:
            def run():
                try:
                    self.get()
                except Exception:  # noqa: BLE001 -- the status line shows it
                    pass
            self._getting = threading.Thread(target=run, daemon=True)
            self._getting.start()
