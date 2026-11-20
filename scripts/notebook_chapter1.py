"""Write docs/notebooks/01_area_to_network.ipynb: chapter 1 of the notebook manual, "Get and build an area"."""
import sys

import nbformat

out = sys.argv[1]
md = nbformat.v4.new_markdown_cell
code = nbformat.v4.new_code_cell
NB = "https://colab.research.google.com/github/Khoshkhah/duckOSM/blob/main/docs/notebooks/01_area_to_network.ipynb"
DL = "https://khoshkhah.github.io/duckOSM/notebooks/01_area_to_network.ipynb"
cells = []

# ---- the look: HTML with inline styles, so that Colab, JupyterLab and the manual show the same boxes -------------
import html as _html
import re as _re

ACCENT = "#1f6feb"
BOX = {  # kind: (title, border, background); transparent backgrounds work in a light and in a dark theme
    "output": ("What the output shows", "#388bfd", "rgba(56,139,253,0.10)"),
    "tip": ("Tip", "#2ea043", "rgba(46,160,67,0.10)"),
    "warning": ("Warning", "#d29922", "rgba(210,153,34,0.14)"),
}


def inline(text):
    """Markdown of one paragraph to HTML: `code`, **bold**, *italic*, [text](address)."""
    t = _html.escape(" ".join(text.split()), quote=False)
    t = _re.sub(r"`([^`]+)`", r'<code>\1</code>', t)
    t = _re.sub(r"\*\*([^*]+)\*\*", r"<b>\1</b>", t)
    t = _re.sub(r"(?<!\*)\*([^*]+)\*(?!\*)", r"<i>\1</i>", t)
    return _re.sub(r"\[([^\]]+)\]\(([^)]+)\)", r'<a href="\2">\1</a>', t)


def box(kind, text):
    title, border, bg = BOX[kind]
    text = text.strip()
    text = text[:1].upper() + text[1:]      # the first word after the title starts with a capital
    return (f'<div style="border-left:4px solid {border};background:{bg};border-radius:6px;padding:10px 14px;'
            f'margin:10px 0;font-size:0.95em;line-height:1.55">'
            f'<div style="font-weight:700;color:{border};margin-bottom:4px">{title}</div>{inline(text)}</div>')


def styled(source):
    """The boxes of one Markdown cell: a paragraph that starts with **What the output shows.**, and a quote that
    starts with **Tip:** or **Warning:**."""
    out = []
    for par in source.split("\n\n"):
        s = par.strip()
        if s.startswith("**What the output shows.**"):
            out.append(box("output", s[len("**What the output shows.**"):]))
        elif s.startswith("> **Tip:**") or s.startswith("> **Warning:**"):
            body = " ".join(line[1:].strip() for line in s.splitlines())
            kind = "tip" if body.startswith("**Tip:**") else "warning"
            out.append(box(kind, body.split(":**", 1)[1]))
        else:
            out.append(par)
    return "\n\n".join(out)


def header(number, title, line, card):
    """The top of a chapter: the title (a Markdown heading, for the tables of contents), a band and an info card."""
    rows = "".join(f'<tr><td style="width:7em;padding:6px 14px 6px 0;font-weight:700;vertical-align:top;white-space:nowrap">{k}</td>'
                   f'<td style="padding:6px 0">{inline(v)}</td></tr>' for k, v in card)
    return (f"# {number}. {title}\n\n"
            f'[![Open in Colab](https://colab.research.google.com/assets/colab-badge.svg)]({NB}) '
            f'<a href="{DL}" download><img src="https://img.shields.io/badge/Download-notebook-F37626?logo=jupyter&logoColor=white" alt="Download the notebook"></a>\n\n'
            f'<div style="background:linear-gradient(135deg,{ACCENT},#0b4f8a);color:#fff;border-radius:10px;padding:16px 20px;margin:12px 0">'
            f'<div style="font-size:0.8em;letter-spacing:0.08em;text-transform:uppercase;opacity:0.85">duckOSM notebook manual · chapter {number}</div>'
            f'<div style="font-size:1.15em;margin-top:6px;line-height:1.5">{inline(line)}</div></div>\n\n'
            f'<div style="border:1px solid rgba(128,128,128,0.35);border-radius:8px;padding:8px 16px;margin:10px 0">'
            f'<table style="border-collapse:collapse;width:100%;margin:0">{rows}</table></div>')

cells.append(md(header(1, "Get and build an area",
    "Get the road network of an area into one file: from a place, an OSM id, a box, a point, an H3 cell, a polygon of your own, a map, or an OSM file that you already have.",
    [("You learn", "the parts of a duckOSM build; seven ways to choose an area; the modes; your own PBF file; the build options and `config=`; where the files go"),
     ("Time", "about 3 minutes (the first download included)"),
     ("Extras", "`duckosm[widget,viz]` (the install cell installs them)"),
     ("Next", "[2. Look inside the file](02_inspect.md): the tables, the columns and SQL")])))

cells.append(code("""# Install duckOSM when it is not installed (for example in Colab). It takes approximately 1 minute.
# Until duckOSM 0.9.0 is on PyPI, the SDK of these notebooks is in the nightly wheel of main (the GitHub pre-release "nightly").
import importlib.util, json, urllib.request
def nightly_wheel():       # the address of the newest nightly wheel
    release = json.load(urllib.request.urlopen("https://api.github.com/repos/Khoshkhah/duckOSM/releases/tags/nightly"))
    return next(a["browser_download_url"] for a in release["assets"] if a["name"].endswith(".whl"))
if importlib.util.find_spec("duckosm") is None:      # not installed: install it with the extras of this notebook
    %pip install -q "duckosm[widget,viz] @ {nightly_wheel()}"
import duckosm             # the SDK
if not hasattr(duckosm, "network"):                    # an old duckOSM (0.8.0 or older), without the SDK
    %pip install -q --force-reinstall --no-deps "duckosm @ {nightly_wheel()}"
    raise SystemExit("duckOSM is updated: restart the session (in Colab: Runtime > Restart session), then run this cell again")
print("duckosm", duckosm.__version__)    # the version"""))

cells.append(md("""## The parts of a build

A duckOSM build has three steps. Each step has its own call, and `duckosm.network` does all three in one call.

| Step | What happens | The call | The result |
|---|---|---|---|
| 1. The boundary | The area becomes a polygon: from a name or an OSM id (Nominatim), a box, a point, an H3 cell or your own GeoJSON | part of `duckosm.get` | `<name>.geojson` |
| 2. The OSM data | duckOSM finds the smallest Geofabrik extract that holds the boundary, downloads it once into a cache, and cuts it to the boundary | `duckosm.get` | `<name>.osm.pbf`, and an `Area` |
| 3. The network | The Rust engine reads the cut file and writes the roads of each mode, the nodes, the graph of turns, the base map and more | `area.build` or `duckosm.build` | `<name>.duckdb`, and a `Network` |

The result is **one DuckDB file**. Each mode is a schema in it (`driving`, `walking`, `cycling`), with the tables
`edges` and `nodes`. Each edge has an `edge_id`, a number that stays the same when you build the area again, so your
own data can join to it (chapter 5).

> **Tip:** you can open the file with any DuckDB tool (the `duckdb` program, DBeaver, Python's `duckdb`), not only
> with duckOSM."""))

cells.append(md("""## Quick start: one call

`duckosm.network` gets the area and builds it. Here the area is the city of Monaco, OSM relation 2220322. The
argument `modes` chooses the networks: driving and walking."""))
cells.append(code("""net = duckosm.network(osm_id=2220322, modes=["driving", "walking"], name="monaco")    # get and build Monaco
net                                                                                 # the card of the network"""))
cells.append(md("""**What the output shows.** The progress lines name each step: the boundary from Nominatim, the extract and the
cut, then the build. The card at the end shows the file and, for each mode, the number of edges and nodes and the
length in km. A `Network` is also the path of its file, so `str(net)` gives `data/monaco.duckdb`.

> **Warning:** the name "Monaco" finds the *country* of Monaco, whose boundary includes a large part of the sea. That
> is why this chapter uses the OSM id of the city. When a name gives the wrong area, find its OSM id on
> [openstreetmap.org](https://www.openstreetmap.org) (search, then the "Relation" number) and use `osm_id=`."""))

cells.append(md("""## The summary of each mode

`net.summary()` gives one row for each mode: the edges, the nodes, the length in km and the length of the one-way
roads. `net.modes` lists the modes of the file."""))
cells.append(code("""print(net.modes)       # the modes in the file
net.summary()          # one row for each mode"""))
cells.append(md("""**What the output shows.** The walking network is much larger than the driving network: it has the footways,
the steps and the paths, and both sides of the streets where OSM draws the sidewalks. A two-way road is two edges,
one for each direction."""))

cells.append(md("""## The modes

A mode is one network. Each mode keeps the roads that it can use, with its own rules.

| Mode | Keeps | Leaves out |
|---|---|---|
| `driving` | the roads for cars (motorway to residential, service roads, ramps), with one-way rules and turn restrictions | footways, paths, steps, private roads |
| `walking` | footways, paths, steps, pedestrian streets, sidewalks, and the roads where people can walk | motorways and roads where walking is not allowed |
| `cycling` | cycleways, roads for bicycles, and footways where you can push the bicycle | motorways, steps |

`net.walking` (or `net["walking"]`) is the view of one mode: the same calls, without `mode=`."""))
cells.append(code("""net.walking            # the card of one mode"""))

cells.append(md("""## Seven ways to choose an area

`duckosm.get` does steps 1 and 2 and gives an `Area`: the cut OSM file and its boundary. It does not build. Use it
to look at an area before the build, or to build it later with other options. Each way below is one argument.

| Argument | The area | Example |
|---|---|---|
| `place` | the first answer of Nominatim for a name | `duckosm.get("Monaco")` |
| `osm_id` | an OSM relation (a city, a district, a park) | `duckosm.get(osm_id=2220322)` |
| `bbox` | a box: west, south, east, north, in degrees | `duckosm.get(bbox=(7.41, 43.72, 7.44, 43.75))` |
| `point` and `dist_m` | the box `dist_m` metres around a point (lon, lat) | `duckosm.get(point=(7.4246, 43.7384), dist_m=500)` |
| `h3_cell` | one H3 cell | `duckosm.get(h3_cell="883969a40bfffff")` |
| `boundary` | your own polygon: a GeoJSON file, a GeoJSON dict, or a shapely or GeoPandas object | `duckosm.get(boundary="district.geojson")` |
| `pbf` (in `network`) | an OSM file that you have | see "Your own PBF file" below |

The extract stays in the cache, so each next area in Monaco needs no download."""))

cells.append(md("""### A box around a point

`point` with `dist_m` is the same as OSMnx's `graph_from_point`: a box of 2 × 500 m around the casino square."""))
cells.append(code("""casino = duckosm.get(point=(7.4280, 43.7393), dist_m=500, name="casino", out="data")   # an Area: the file and the boundary
casino                                                                                  # the card of the area"""))
cells.append(md("""**What the output shows.** The progress line counts the nodes, ways and relations of the cut. The card names the
two files: the cut OSM file and the boundary. The box is a small part of the extract."""))

cells.append(md("""### A box in degrees

`bbox` takes west, south, east and north, in this order (the order of GeoJSON and of OSMnx 2)."""))
cells.append(code("""harbour = duckosm.get(bbox=(7.418, 43.730, 7.430, 43.738), name="harbour", out="data")   # the harbour of Monaco
harbour"""))

cells.append(md("""### Your own polygon

`boundary` takes a polygon. Here it is a shapely polygon; a GeoJSON file or a GeoPandas object work the same way.
duckOSM writes it to `<out>/<name>.geojson`."""))
cells.append(code("""from shapely.geometry import Polygon                       # any object with __geo_interface__ works
monte_carlo = Polygon([(7.420, 43.737), (7.432, 43.737), (7.432, 43.745), (7.420, 43.745)])
mc = duckosm.get(boundary=monte_carlo, name="monte_carlo", out="data")
mc"""))

cells.append(md("""### Look before you get: `dry_run`

`dry_run=True` downloads and writes nothing. It tells you which extract the area needs, its size, and whether it is
in the cache already. Use it before a large area."""))
cells.append(code("""plan = duckosm.get(osm_id=2220322, dry_run=True)        # no download, no file
{k: plan["extract"][k] for k in ("name", "km2", "bytes", "cached")}"""))

cells.append(md("""## Build an area

`area.build` does step 3 for an `Area`. Its arguments are the same as those of `duckosm.network`: `modes`, `output`,
`fixes` and the build options (below)."""))
cells.append(code("""casino_net = casino.build(modes=["walking"])      # the walking network of the box around the casino
casino_net.summary()"""))

cells.append(md("""## Choose the area on a map

`duckosm.AreaPicker` is a map in the notebook. Choose an area with one of the tabs (Search, OSM id, Box, H3 cell,
File); the line under the map shows the extract and its size. Click **Get**, or call `picker.get()`. You can also set
the area from Python with `picker.area`: the widget then shows its tab and its value.

![The AreaPicker widget: the OSM id 2220322 gives the city of Monaco](https://raw.githubusercontent.com/Khoshkhah/duckOSM/main/docs/images/area-picker.png)"""))
cells.append(code("""picker = duckosm.AreaPicker(out="data")                 # the files go into the folder data/
picker.area = {"mode": "osm_id", "osm_id": 2220322}      # the area when you choose none on the map
picker                                                    # the map"""))
cells.append(code("""area = picker.result or picker.get()    # the result of Get, or get it now
area"""))

cells.append(md("""## Your own PBF file

`pbf=` takes an `.osm.pbf` file that you already have: from Geofabrik, from BBBike, from your own OSM editor, or a
file that you cut with osmium. Without `boundary=`, the whole file is the area. With it, duckOSM cuts the file to
the boundary before the build. Here the file is a Monaco extract (0.6 MB)."""))
cells.append(code("""import urllib.request                                                           # your own file: here, a download
mine = "data/my_monaco.osm.pbf"
urllib.request.urlretrieve("https://github.com/Khoshkhah/duckOSM/raw/main/tests/data/monaco.osm.pbf", mine)
whole = duckosm.network(pbf=mine, name="monaco_whole", modes=["driving"])        # the whole file
whole.summary()"""))
cells.append(code("""cut = duckosm.network(pbf=mine, boundary="data/monte_carlo.geojson", name="monte_carlo_cut", modes=["driving"])
cut.summary()                                                                     # only the roads in Monte Carlo"""))
cells.append(md("""**What the output shows.** The cut to the Monte Carlo polygon (from "Your own polygon" above) keeps about a
quarter of the driving edges of the whole file (779 of 2844).

> **Tip:** if the download of an extract fails on your machine (some cloud networks block Geofabrik), get the file
> another way and use `pbf=`."""))

cells.append(md("""## The build options

These keywords work in `duckosm.network`, `area.build` and `duckosm.build`. Each one sets one key of the config file.

| Keyword | Default | What it does |
|---|---|---|
| `speeds` | `True` | `False`: no `maxspeed_kmh` (the raw OSM `maxspeed` text stays), and no `cost_s` |
| `costs` | `True` | `False`: no `cost_s`, the travel time of each edge |
| `restrictions` | `True` | `False`: no turn restrictions (`driving.turn_restrictions`) |
| `bus_routes` | `True` | `False`: no bus routes (`bus.routes`, `bus.route_edges`) |
| `connect_paths` | `True` | `False`: the free ends of paths stay free; `True` connects each one to the nearest node |
| `snap_m` | `10` | the largest distance, in metres, for `connect_paths` |
| `features` | `True` | `False`: no base-map layers (`features.*`: water, land, buildings) |
| `levels` | `True` | `False`: no drawing order of the roads (bridges over tunnels) |
| `dem` | none | a DEM file (GeoTIFF): the heights of the nodes and of the edges, after the build |

A smaller build is faster. Here the build has no bus routes, no base map and no drawing order."""))
cells.append(code("""small = duckosm.network(osm_id=2220322, name="monaco_small", modes=["driving"],
                        bus_routes=False, features=False, levels=False)       # three options off
sorted({t.split(".")[0] for t in small.tables()})                           # the schemas of the file"""))
cells.append(md("""**What the output shows.** The file has only `driving` and `main`: no `bus` (no bus routes), no `features` and
`raw` (no base map) and no `visualization` (no drawing order). Compare it with the schemas of `net` from the quick
start."""))
cells.append(code("""sorted({t.split(".")[0] for t in net.tables()})          # the schemas of a build with the defaults"""))

cells.append(md("""### Every other option: `config=`

The config file has many more keys (`config/template.yaml` in the repository lists them all). `config=` takes such a
file (YAML or JSON) or a Python dict. A keyword wins over the same key of the config.

> **Warning:** a wrong keyword stops the call with the nearest name, for example
> `network has no argument 'bus_route': did you mean 'bus_routes'?`."""))
cells.append(code("""cfg = {"options": {"bus_routes": False}, "clip": {"connect_snap_m": 5}}      # two keys of the config file
tuned = duckosm.network(osm_id=2220322, name="monaco_tuned", modes=["walking"], config=cfg,
                        features=False, levels=False)
tuned.summary()"""))
cells.append(code("""try:
    duckosm.network(osm_id=2220322, name="x", modes=["driving"], bus_route=False)    # a wrong name
except duckosm.DuckOSMError as e:
    print(e)"""))

cells.append(md("""## Open the file again

The file stays on disk. `duckosm.open` gives its `Network` in a new session, without a build."""))
cells.append(code("""again = duckosm.open("data/monaco.duckdb")      # the file of the quick start
again.summary()"""))

cells.append(md("""## Where the files go

| File | Where | Set it with |
|---|---|---|
| the `.duckdb` file | `<out>/<name>.duckdb` (default `data/<name>.duckdb`) | `out=`, `name=`, or `output=` in `build` |
| the cut OSM file and the boundary | `<out>/<name>.osm.pbf`, `<out>/<name>.geojson` | `out=`, `name=` |
| the Geofabrik extracts | the cache: `~/.cache/duckosm` | the setting `DUCKOSM_CACHE` |"""))

cells.append(md("""## Try it yourself

1. Get the walking network of 1 km around a place that you know, with `point=` and `dist_m=1000`.
2. Build Monaco twice, once with `speeds=False`, and compare `net.columns("driving.edges")` of both files.
3. Use `dry_run=True` for your own city, and look at the size of its extract before you get it."""))

cells.append(md("""## Summary

| Call | Does |
|---|---|
| `duckosm.network(...)` | gets and builds an area in one call; gives a `Network` |
| `duckosm.get(...)` | the boundary and the cut OSM file; gives an `Area` (`dry_run=True`: only a plan) |
| `area.build(modes=...)` | builds an `Area` |
| `duckosm.build(pbf=..., boundary=...)` | builds from files |
| `duckosm.AreaPicker()` | chooses an area on a map |
| `duckosm.open(path)` | opens a file that you built before |
| `net.summary()`, `net.modes`, `net.walking` | the modes of a file |

The reference: [Python SDK, Core](../reference/python.md). **Next:** [2. Look inside the file](02_inspect.md), the
tables, the columns and SQL."""))

for c in cells[1:]:
    if c.cell_type == "markdown":
        c.source = styled(c.source)
nb = nbformat.v4.new_notebook()
nb.cells = cells
nb.metadata["kernelspec"] = {"name": "python3", "display_name": "Python 3", "language": "python"}
nbformat.write(nb, out)
print(len(cells), "cells")
