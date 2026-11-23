"""Make the chapters of the Quarto book (book/) from the notebooks of the manual (docs/notebooks/), with their saved
outputs. The notebooks stay the one source: run this before `quarto render book`.

    python book/make_book.py"""
import json
import re

import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
NOTEBOOKS = os.path.join(HERE, "..", "docs", "notebooks")
CHAPTERS = ["01_area_to_network", "02_inspect", "03_maps", "04_join_by_edge_id", "05_osm_fixes", "06_exports",
            "07_routing", "08_routing_advanced", "09_analysis", "10_gis", "11_mobility", "12_sim", "13_planning",
            "14_hdmap", "15_ml"]          # the chapters in the book style (scripts/notebook_book.py)
# a short opening of each chapter (the notebook has it in its band picture)
OPENING = {
    "01_area_to_network": "This chapter shows how you get the road network of an area into one file: from a place name, an "
    "OSM id, a box, a point, an H3 cell, a polygon of your own, a map, or an OSM file that you already have. It also "
    "shows the options of the build, and how you save the network.",
    "05_osm_fixes": "OSM data has errors: an old speed limit, a wrong one-way, a road that is closed for road works. In "
    "duckOSM you write each fix one time, in a fixes file. The build reads the file, so the network, the routes, the "
    "maps and every export show the corrected data.",
    "06_exports": "A duckOSM network is a DuckDB file, and many tools cannot read it. An export writes the network in the "
    "format of such a tool: SUMO, MATSim, GMNS, OpenDRIVE, a GIS file or vector tiles. In each format, the id of a road "
    "is the `edge_id`, so the results of each tool join back to the network.",
    "07_routing": "A route in duckOSM goes from edge to edge on the turn graph. In this graph a node is a road in one "
    "direction, and an arc is a legal turn. So every route obeys the one-way rules and the turn restrictions of OSM. A "
    "route can start at an edge id, a point or a street name, and it can change mode on the way.",
    "08_routing_advanced": "One route answers one question. Many questions need many routes: the travel time between all "
    "pairs of places, the area that you reach in 10 minutes, the nearest stop from each street. This chapter shows the "
    "routing calls for these questions. They all search the same graph of turns, so their results agree.",
    "09_analysis": "A road network has a form: its junctions, its dead ends, the directions of its streets and its "
    "connected parts. Street network analysis describes this form with numbers. This chapter gives the numbers of "
    "OSMnx on a duckOSM network, and shows that OSMnx itself works on it.",
    "10_gis": "An OSM file holds more than roads: buildings, shops, parks, bus stops and administrative areas. This "
    "chapter gives these OSM features as GIS layers, writes the network as OSM XML, and finds the quarter of each road. "
    "The features come from the same OSM file as the network, so the two agree.",
    "11_mobility": "GPS traces, traffic sensors and counts become useful when each record has an edge. This chapter puts "
    "GPS traces on the driving network of Monaco with map matching, puts sensors on their nearest edges and counts the "
    "points on each edge. It also shows how your data stays on the correct edges after a rebuild.",
    "12_sim": "A traffic simulation needs a network, a demand and a simulator. This chapter makes the SUMO and MATSim "
    "networks of Monaco, runs a short SUMO simulation with random trips and puts the counts back on the duckOSM edges. "
    "The id of each SUMO edge and of each MATSim link is the `edge_id`, so no table of ids is necessary.",
    "13_planning": "Transport planning tools need a network with lanes and turns, not only roads. This chapter writes "
    "the GMNS tables of Monaco, checks them, makes the meso and micro networks, loads the network in path4gmns and "
    "makes an AequilibraE project.",
    "14_hdmap": "An automated vehicle needs more than the roads. It needs each lane, its borders, and the lane that comes "
    "next through a junction. In this chapter, you make the lanes of Monaco and write them as a CommonRoad map, a "
    "Lanelet2 map, a lane graph and an OpenDRIVE file.",
    "15_ml": "A road network is a graph, and duckOSM gives it in two forms: junctions joined by roads, and roads joined "
    "by legal turns. In this chapter, you make both graphs of Monaco for networkx, rustworkx and PyTorch Geometric, and "
    "train a small graph neural network that predicts the speed limit of each road. Each prediction goes back to its "
    "road by its `edge_id`.",
}
IMG = "https://raw.githubusercontent.com/Khoshkhah/duckOSM/main/docs/notebooks/images"
COLAB = "https://colab.research.google.com/github/Khoshkhah/duckOSM/blob/main/docs/notebooks/01_area_to_network.ipynb"
KINDS = {"output": ("note", "What the output shows", "output-note"), "tip": ("tip", "Tip", ""),
         "warning": ("warning", "Warning", ""), "key": ("important", "Key idea", "key-idea")}
FIGS = {"build-flow.svg": ("fig-build-flow", "The steps of a build. `duckosm.get` makes the boundary and the cut; `area.build` makes the network; `duckosm.network` does all of it."),
        "modes-monaco.png": ("fig-modes", "The three networks of Monaco. In each panel all the roads are gray, and the roads of the mode have its color."),
        "area-boundary.png": ("fig-boundary", "Two boundaries for Monaco. The name finds the country, with its sea; the OSM id 2220322 is the city. Roads that cross the city boundary stay whole."),
        "area-picker.png": ("fig-picker", "The AreaPicker widget: the OSM id 2220322 gives the city of Monaco.")}


def callout(kind, text):
    typ, title, extra = KINDS[kind]
    cls = f".callout-{typ}" + (f" .{extra}" if extra else "")
    return f'::: {{{cls} title="{title}"}}\n{text.strip()}\n:::\n\n'


def fix_links(s):
    """Links of a notebook in the book: a manual page (../x/y.md) becomes the page of the manual next to the book
    (../x/y/); a chapter in the book stays (n_name.md becomes n_name.ipynb, Quarto links it); another chapter becomes
    plain text until it is in the book."""
    def link(m):
        text, target = m.group(1), m.group(2)
        if target.startswith(("http", "#")) or not target.split("#")[0].endswith(".md"):
            return m.group(0)
        path, _, anchor = target.partition("#")
        if path.startswith("../"):
            return f"[{text}]({path[:-3]}/{'#' + anchor if anchor else ''})"
        slug = os.path.basename(path)[:-3]
        return f"[{text}]({slug}.ipynb{'#' + anchor if anchor else ''})" if slug in CHAPTERS else text
    return re.sub(r"(?<!!)\[([^\]]+)\]\(([^)\s]+)\)", link, s)


def convert_md(s):
    # the boxes: "> <img ...icon-KIND.svg ...> **Title**\n>\n> text"
    def box(m):
        kind = m.group(1)
        body = "\n".join(line[1:].strip() if line.startswith(">") else line for line in m.group(2).splitlines()).strip()
        return callout(kind, body)
    s = re.sub(r'> <img src="[^"]*icon-(\w+)\.svg"[^>]*> \*\*[^*]+\*\*\n>\n((?:>.*(?:\n|$))+)', box, s)
    # the pictures: figures with a caption and a label
    def fig(m):
        url, alt = m.group(1), m.group(2)
        name = url.rsplit("/", 1)[1]
        label, cap = FIGS.get(name, (None, alt))
        return f"![{cap}]({url}){{#{label}}}" if label else f"![{alt}]({url})"
    s = re.sub(r'<img src="([^"]+)" alt="([^"]*)" width="100%">', fig, s)
    s = re.sub(r'!\[([^\]]*)\]\((https://raw\.githubusercontent\.com/[^)]*area-picker\.png)\)',
               lambda m: f"![{FIGS['area-picker.png'][1]}]({m.group(2)}){{#fig-picker width=70%}}", s)
    # exercises: an answer behind a fold becomes a collapsed callout
    s = re.sub(r"<details><summary>Show the answer</summary>\n\n(```python\n.*?\n```)\n</details>",
               lambda m: f'::: {{.callout-tip collapse="true" title="Show the answer"}}\n{m.group(1)}\n:::', s, flags=re.S)
    return s



def strip_comments(src):
    """The code without its comments (the text of the chapter explains the code). Python's tokenizer finds the
    comments, so a # inside a string stays. A line that is only a comment goes."""
    import io
    import tokenize
    lines = src.splitlines()
    cut = {}
    try:
        for tok in tokenize.generate_tokens(io.StringIO(src).readline):
            if tok.type == tokenize.COMMENT:
                cut[tok.start[0]] = tok.start[1]
    except (tokenize.TokenError, IndentationError):
        return src
    out = []
    for i, line in enumerate(lines, 1):
        if i in cut:
            line = line[:cut[i]].rstrip()
            if not line:
                continue
        out.append(line)
    return "\n".join(out)



def join_text_outputs(outputs):
    """Consecutive outputs that are only text (a progress line, a printed line, a short result) become one text
    output, so the page shows them as one block."""
    out, buf = [], []
    def text_of(o):
        if o.get("output_type") == "stream":
            return "".join(o.get("text", ""))
        data = o.get("data", {})
        if set(data) <= {"text/plain"} and "text/plain" in data:
            return "".join(data["text/plain"])
        return None
    for o in outputs:
        s = text_of(o)
        if s is None:
            if buf:
                out.append({"output_type": "stream", "name": "stdout", "text": "\n".join(x.rstrip("\n") for x in buf) + "\n"})
                buf = []
            out.append(o)
        else:
            buf.append(s)
    if buf:
        out.append({"output_type": "stream", "name": "stdout", "text": "\n".join(x.rstrip("\n") for x in buf) + "\n"})
    return [fold_long(o) for o in out]


def fold_long(o, limit=15):
    """A text output of more than ``limit`` lines starts folded: 'Show the output (N lines)'."""
    import html
    if o.get("output_type") != "stream":
        return o
    text = "".join(o["text"])
    n = text.count("\n")
    if n <= limit:
        return o
    body = (f'<details class="long-output"><summary>Show the output ({n} lines)</summary>'
            f"<pre><code>{html.escape(text)}</code></pre></details>")
    return {"output_type": "display_data", "metadata": {}, "data": {"text/html": body}}

def facts(head):
    """The title, the topics and the extras from the top cell of a notebook (scripts/notebook_book.py writes it)."""
    title = re.search(r"^# \d+\. (.+)$", head, re.M).group(1).strip()
    learn = re.search(r"^\| \*\*You learn\*\* \| (.+) \|$", head, re.M).group(1)
    extras = re.search(r"^\| \*\*Extras\*\* \| `([^`]+)`", head, re.M).group(1)
    return title, [x.strip() for x in learn.split(";")], extras


def chapter(slug):
    src = json.load(open(os.path.join(NOTEBOOKS, slug + ".ipynb")))
    title, learn, extras = facts("".join(src["cells"][0]["source"]))
    colab = f"https://colab.research.google.com/github/Khoshkhah/duckOSM/blob/main/docs/notebooks/{slug}.ipynb"
    topics = "\n".join(f"- {x}" for x in learn)
    cells = [{"cell_type": "markdown", "metadata": {}, "source": f"""---
title: "{title}"
---

{OPENING.get(slug, "")}

::: {{.chapter-facts}}
[You learn]{{.facts-label}}

{topics}

[Extras]{{.facts-label}} `{extras}`
:::

[Run this chapter in Colab]({colab}){{.colab-button}}
"""}]
    for c in src["cells"][1:]:
        if c["cell_type"] == "markdown":
            cells.append({"cell_type": "markdown", "metadata": {}, "source": local_images(fix_links(convert_md("".join(c["source"]))))})
        else:
            c = dict(c)
            if "nightly_wheel" not in "".join(c["source"]):
                c["source"] = strip_comments("".join(c["source"]))
            c["outputs"] = join_text_outputs(c.get("outputs", []))
            if "nightly_wheel" in "".join(c["source"]):
                c["source"] = '#| code-fold: true\n#| code-summary: "Install duckOSM (for Colab)"\n#| output: false\n' + "".join(c["source"])
            cells.append(c)
    for c in cells:          # the Jupyter form: a list of lines (Quarto loses the line ends of a single string)
        s = c["source"] if isinstance(c["source"], str) else "".join(c["source"])
        c["source"] = s.splitlines(keepends=True)
    nb = dict(src)
    nb["cells"] = cells
    json.dump(nb, open(os.path.join(HERE, slug + ".ipynb"), "w"), indent=1)
    return len(cells)


def local_images(s):
    """The pictures of the notebooks from the book's own copy (images/), so a new picture shows before it is on main."""
    return s.replace(IMG + "/", "images/")


if __name__ == "__main__":
    import glob
    import shutil
    shutil.copytree(os.path.join(NOTEBOOKS, "images"), os.path.join(HERE, "images"), dirs_exist_ok=True)
    for page in glob.glob(os.path.join(NOTEBOOKS, "*.html")):     # web pages that a chapter shows in a frame (lonboard)
        shutil.copy(page, HERE)
    for slug in CHAPTERS:
        print(slug, chapter(slug), "cells")
