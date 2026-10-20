"""The level editor of a duckOSM database (``duckosm levels-edit DB``): roadstyle's editor, started on the area of the file, with mapstyle's
base map (water, land, buildings, railways, signals) and every road of the file: the roads of all its modes and their ``private_edges``
(bus, private, pedestrian, delivery). A file with one mode gives a simple editor of that mode; with several, the editor's own boxes show only some.
Each Apply writes into the file (docs/design/levels_in_build.md)."""
from __future__ import annotations

import contextlib
import json
import logging
import os
from pathlib import Path

from duckosm.levels import is_current, load_roads, make_and_solve, roadstyle_module, sync_inputs

logger = logging.getLogger(__name__)


# the colours of the restricted roads, as in mapstyle's Roads box (a road that no mode can use)
ACCESS_COLORS = {"private": "#c8c8c8", "bus": "#9db8d9", "pedestrian": "#dddde8", "delivery": "#e8d2b0"}

# added to the editor's page: paints those roads (by their `access`) as the base of rsColor. The editor paints the picked roads with rsColor too,
# which replaces the roads painted before: so rsColor is wrapped, and every call keeps these groups after its own (its own win).
_SCRIPT = """<script>(function(){const C=__COLORS__;let base=[];const orig=window.rsColor;
window.rsColor=function(a,b,c){if(c!=null)return orig(a,b,c);let g;
if(a==null)g=[];else if(Array.isArray(a)&&a.length&&Array.isArray(a[0])&&a[0].length===2&&typeof a[0][1]==="string")g=a;else g=[[a,b]];
return orig(g.concat(base));};
let n=0;const tick=()=>{const g=Object.entries(C).map(([k,c])=>[rsQuery(p=>p.access===k),c]).filter(([ids])=>ids.length);
if(g.length){base=g;window.rsColor(null);}else if(++n<80)setTimeout(tick,300);};tick();})();</script>"""


def items_for(db):
    """``f(roads)`` for roadstyle's editor (``serve(items=f)``): the overlays of mapstyle's feature layers (water, land, buildings, …) of ``db``,
    not clickable (decoration), on the blank base map; and each road's ``access`` (private, bus, pedestrian, delivery: a road that no mode can use,
    as mapstyle's all-modes map says it). ``None`` if mapstyle is not installed (the editor is then plain)."""
    try:
        from mapstyle.map import _access, feature_overlays, load_layers, load_roads
        from mapstyle.style import load_style
    except ImportError:
        logger.warning('the editor has no base map layers: pip install "duckosm[viz]" for mapstyle')
        return None
    roads = load_roads(db)
    access = {str(e): a for e, a in zip(roads["edge_id"], _access(roads, "all"), strict=True) if isinstance(a, str)}
    # the layers a mapstyle page opens ticked, as areas and lines: a point layer (signals, crossings, stops) is a plain dot here, without mapstyle's icons
    names = [sp["name"] for sp in load_style("layers")["layers"] if sp.get("ticked", True) and sp["kind"] != "point"]

    def items(draw):
        if draw is not None:
            draw["access"] = [access.get(str(e)) for e in draw["edge"]]
        overlays, _config = feature_overlays(load_layers(db, names))
        for o in overlays:
            o.visible = True                    # mapstyle's layers.js shows them once styled; in the editor they start visible
            o.popup, o.tooltip = [], None       # decoration: a hover would highlight every feature that has the same number
        kw = {"basemap": "blank", "tooltip": ["access"]}
        if os.environ.get("GOOGLE_MAPS_KEY"):
            kw["google_3d"] = True              # the Google 3D window, linked to the map (it needs this key and a page address on the key's website list)
        return overlays, kw
    return items


@contextlib.contextmanager
def _access_colours(on):
    """While the editor is served, its page has the script that paints the restricted roads (roadstyle's ``Area.page``, wrapped; restored after)."""
    if not on:
        yield
        return
    from roadstyle import level_editor as le
    original = le.Area.page
    script = _SCRIPT.replace("__COLORS__", json.dumps(ACCESS_COLORS))
    le.Area.page = property(lambda self: original.fget(self).replace("</body>", script + "</body>", 1))
    try:
        yield
    finally:
        le.Area.page = original


def prepare(db, area=None):
    """The area folder of ``db`` ready to edit: its files written from the tables (the master), and solved if the stored numbers are not those of
    the roads in the file. Returns the folder."""
    roadstyle_module()
    area = Path(area) if area else Path(db).with_suffix(".levels")
    roads = load_roads(db)
    if is_current(db, area, roads):
        sync_inputs(db, area, roads)
    else:
        res = make_and_solve(db, area)
        logger.info(f"level area {area}: {res['roads']:,} roads solved")
    return area


def _front(port, editor_port, reports):
    """The one server on ``port`` (default 8000: the port of the maps, so the Google key's website list has one line for ever): a file of the
    ``reports`` folder is served as it is (the maps ``duckosm viz`` made, never cached), and every other address goes to the editor, which listens
    on ``editor_port`` (only on this machine)."""
    import http.client
    import mimetypes
    from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
    from urllib.parse import unquote, urlparse

    reports = Path(reports).resolve() if reports else None

    class Front(BaseHTTPRequestHandler):
        def _file(self):
            path = unquote(urlparse(self.path).path)
            if reports is None or path == "/" or path.startswith("/api/"):
                return None
            f = (reports / path.lstrip("/")).resolve()
            return f if f.is_file() and reports in f.parents else None

        def _send_file(self, f, body=True):
            data = f.read_bytes()
            self.send_response(200)
            self.send_header("Content-Type", mimetypes.guess_type(str(f))[0] or "application/octet-stream")
            self.send_header("Content-Length", str(len(data)))
            self.send_header("Cache-Control", "no-store")             # a map changes after every edit: never reuse a copy
            self.end_headers()
            if body:
                self.wfile.write(data)

        def _forward(self):
            n = int(self.headers.get("Content-Length") or 0)
            conn = http.client.HTTPConnection("127.0.0.1", editor_port, timeout=600)
            try:
                conn.request(self.command, self.path, self.rfile.read(n) if n else None,
                             {k: v for k, v in self.headers.items() if k.lower() not in ("host", "connection")})
                r = conn.getresponse()
                data = r.read()
                self.send_response(r.status)
                for k, v in r.getheaders():
                    if k.lower() not in ("transfer-encoding", "connection", "content-length"):
                        self.send_header(k, v)
                self.send_header("Content-Length", str(len(data)))
                self.end_headers()
                if self.command != "HEAD":
                    self.wfile.write(data)
            finally:
                conn.close()

        def do_GET(self):
            f = self._file()
            self._send_file(f) if f else self._forward()

        def do_HEAD(self):
            f = self._file()
            self._send_file(f, body=False) if f else self._forward()

        def do_POST(self):
            self._forward()

        def log_message(self, *args):
            pass

    return ThreadingHTTPServer(("127.0.0.1", port), Front)


def start(db, area=None, port=8000, plain=False, reports="reports"):
    """Prepare the area of ``db`` and start the editor behind one server at http://localhost:``port``/ (see :func:`_front`). Returns
    ``(front, area)``: ``front.serve_forever()`` runs it. ``plain``: no mapstyle layers and colours."""
    import socket
    import threading
    import time

    from roadstyle.level_editor import serve

    area = prepare(db, area)
    items = None if plain else items_for(db)
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        editor_port = s.getsockname()[1]                       # the editor's own port: free, and no one but the front server uses it

    def run():
        with _access_colours(items is not None):
            serve(area, editor_port, items=items)
    threading.Thread(target=run, daemon=True).start()
    for _ in range(600):                                       # the editor builds its map first: wait until it listens
        try:
            socket.create_connection(("127.0.0.1", editor_port), timeout=1).close()
            break
        except OSError:
            time.sleep(0.5)
    return _front(port, editor_port, reports), area


def edit(db, area=None, port=8000, plain=False, reports="reports"):
    """Start the editor of ``db`` at http://localhost:``port``/ until Ctrl+C. The files of ``reports`` (the maps) are on the same port."""
    front, area = start(db, area, port, plain, reports)
    print(f"level editor: http://localhost:{port}/  (maps of {reports}/ at the same port; Ctrl+C to stop)", flush=True)
    front.serve_forever()
