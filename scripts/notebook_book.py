"""The style of the notebook manual (docs/notebooks/): one chapter is one notebook. The rules are in
docs/design/notebook_book.md.

Colab removes inline CSS from Markdown, so the look is made only of what Colab draws: a picture (the band), a
Markdown table (the card), quote blocks with a colored icon (the boxes) and <details> (the answers). The manual
colors each kind of box by its icon (docs/assets/extra.css).

    from notebook_book import Chapter
    ch = Chapter(1, "01_area_to_network", "Get and build an area", "From a place ... to one network file",
                 learn="...", time="about 3 minutes", extras="widget,viz", next_=("02_inspect", "2. Look inside the file", "the tables"))
    ch.md("## A section\\n\\nText.\\n\\n**What the output shows.** ...")
    ch.code("net = duckosm.network(...)")
    ch.exercise("Get ...", "net = duckosm.network(...)")
    ch.write("docs/notebooks/01_area_to_network.ipynb")
"""
import html
import os

import nbformat

REPO = "Khoshkhah/duckOSM"
IMAGES = f"https://raw.githubusercontent.com/{REPO}/main/docs/notebooks/images"
CHAPTERS = 15
# the kinds of box: the start of the paragraph in a chapter script, and the title in the notebook
BOXES = {"output": ("**What the output shows.**", "What the output shows"), "tip": ("> **Tip:**", "Tip"),
         "warning": ("> **Warning:**", "Warning"), "key": ("> **Key idea:**", "Key idea")}


def box(kind, text):
    """A quote block with the icon and the title of its kind."""
    text = " ".join(text.split())
    text = text[:1].upper() + text[1:]
    icon = f'<img src="{IMAGES}/icon-{kind}.svg" width="18" height="18" alt="">'
    return f"> {icon} **{BOXES[kind][1]}**\n>\n> {text}"


def styled(source):
    """The boxes of one Markdown cell: each paragraph that starts as in BOXES becomes its box."""
    out = []
    for par in source.split("\n\n"):
        s = par.strip()
        for kind, (start, _) in BOXES.items():
            if s.startswith(start):
                body = " ".join(line[1:].strip() if line.startswith(">") else line for line in s.splitlines())
                out.append(box(kind, body[len(start.lstrip("> ")):] if start.startswith(">") else body[len(start):]))
                break
        else:
            out.append(par)
    return "\n\n".join(out)


def install_cell(extras, more="", book_network=False):
    """The first code cell: in Colab (no duckosm), the newest nightly wheel with the extras of the chapter."""
    return f'''# Install duckOSM when it is not installed (for example in Colab). It takes approximately 1 minute.
# Until duckOSM 0.9.0 is on PyPI, the SDK of these notebooks is in the nightly wheel of main (the GitHub pre-release "nightly").
# roadstyle (the maps) also comes from its nightly wheel, so the newest roadstyle parts work with no roadstyle release.
import importlib.util, json, os, urllib.request
def nightly_wheel(repo="{REPO}", name=".whl"):        # the address of the newest nightly wheel of a repository
    release = json.load(urllib.request.urlopen(f"https://api.github.com/repos/{{repo}}/releases/tags/nightly"))
    return next(a["browser_download_url"] for a in release["assets"] if name in a["name"] and a["name"].endswith(".whl"))
if importlib.util.find_spec("duckosm") is None:      # not installed: install it with the extras of this notebook
    %pip install -q "duckosm[{extras}] @ {{nightly_wheel()}}"{more}
    %pip install -q --force-reinstall --no-deps "roadstyle @ {{nightly_wheel('Khoshkhah/roadstyle', 'manylinux')}}"
import duckosm             # the SDK
if not hasattr(duckosm, "network"):                    # an old duckOSM (0.8.0 or older), without the SDK
    %pip install -q --force-reinstall --no-deps "duckosm @ {{nightly_wheel()}}"
    raise SystemExit("duckOSM is updated: restart the session (in Colab: Runtime > Restart session), then run this cell again")
print("duckosm", duckosm.__version__)    # the version''' + ('''
# The network of chapter 1 (monaco.duckdb). The chapter opens it. When you run this chapter alone (in Colab), the file
# is not there, so this cell makes it one time.
if not os.path.exists("monaco.duckdb") or not duckosm.open("monaco.duckdb").has_mode("driving", "walking", "cycling"):
    duckosm.network(osm_id=2220322, modes=["driving", "walking", "cycling"], path="monaco.duckdb")''' if book_network else "")


class Chapter:
    """One chapter of the notebook manual."""

    def __init__(self, number, slug, title, line, *, learn, time, extras, next_=None, more_install=""):
        self.number, self.slug, self.title, self.line = number, slug, title, line
        self.cells = []
        card = [("You learn", learn), ("Time", time), ("Extras", f"`duckosm[{extras}]` (the install cell installs them)")]
        if next_:
            card.append(("Next", f"[{next_[1]}]({next_[0]}.md): {next_[2]}"))
        self.next_ = next_
        colab = f"https://colab.research.google.com/github/{REPO}/blob/main/docs/notebooks/{slug}.ipynb"
        download = f"https://khoshkhah.github.io/duckOSM/notebooks/{slug}.ipynb"
        rows = [f"| **{k}** | {v} |" for k, v in card]
        self._head = (f"# {number}. {title}\n\n"
                      f"[![Open in Colab](https://colab.research.google.com/assets/colab-badge.svg)]({colab}) "
                      f'<a href="{download}" download><img src="https://img.shields.io/badge/Download-notebook-F37626?logo=jupyter&logoColor=white" alt="Download the notebook"></a>\n\n'
                      f'<img src="{IMAGES}/banner-{number}.svg" alt="duckOSM notebook manual, chapter {number}: {html.escape(title)}" width="100%">\n\n'
                      + "\n".join([rows[0], "|---|---|", *rows[1:]]))
        self.cells.append(nbformat.v4.new_markdown_cell(self._head))
        self.cells.append(nbformat.v4.new_code_cell(install_cell(extras, more_install, number > 1)))

    def md(self, text):
        """A Markdown cell; its box paragraphs become boxes."""
        self.cells.append(nbformat.v4.new_markdown_cell(styled(text)))

    def code(self, source):
        self.cells.append(nbformat.v4.new_code_cell(source))

    def picture(self, file, alt, text=""):
        """A picture of docs/notebooks/images/, with an optional line of text above it."""
        self.md((text + "\n\n" if text else "") + f'<img src="{IMAGES}/{file}" alt="{html.escape(alt)}" width="100%">')

    def exercises(self, items):
        """'Try it yourself': each item is (the question, the code of the answer); the answer is behind a fold."""
        parts = ["## Try it yourself"]
        for i, (question, answer) in enumerate(items, 1):
            parts.append(f"**{i}.** {question}\n\n<details><summary>Show the answer</summary>\n\n```python\n{answer}\n```\n</details>")
        self.cells.append(nbformat.v4.new_markdown_cell("\n\n".join(parts)))

    def glossary(self, rows):
        """'Words in this chapter': (word, meaning) rows."""
        self.cells.append(nbformat.v4.new_markdown_cell(
            "## Words in this chapter\n\n| Word | Meaning |\n|---|---|\n" + "\n".join(f"| {w} | {m} |" for w, m in rows)))

    def summary(self, rows, reference="[Python SDK, Core](../reference/python.md)"):
        """'Summary': (call, what it does) rows, the reference page and the next chapter."""
        nxt = f" **Next:** [{self.next_[1]}]({self.next_[0]}.md), {self.next_[2]}." if self.next_ else ""
        self.cells.append(nbformat.v4.new_markdown_cell(
            "## Summary\n\n| Call | Does |\n|---|---|\n" + "\n".join(f"| {c} | {d} |" for c, d in rows)
            + f"\n\nThe reference: {reference}.{nxt}"))

    def banner_svg(self):
        """The band: a light card, a thin blue line on the left, the chapter label, the title and one short line."""
        return ('<svg xmlns="http://www.w3.org/2000/svg" width="900" height="132" viewBox="0 0 900 132" '
                'font-family="Segoe UI, Helvetica, Arial, sans-serif">'
                '<rect x="1" y="1" width="898" height="130" rx="12" fill="#f6f8fa" stroke="#d0d7de"/>'
                '<rect x="1" y="1" width="7" height="130" rx="3" fill="#1f6feb"/>'
                f'<text x="32" y="38" font-size="13" font-weight="600" letter-spacing="1.4" fill="#1f6feb">'
                f'DUCKOSM NOTEBOOK MANUAL · CHAPTER {self.number} OF {CHAPTERS}</text>'
                f'<text x="32" y="78" font-size="30" font-weight="700" fill="#1f2328">{html.escape(self.title)}</text>'
                f'<text x="32" y="108" font-size="17" fill="#57606a">{html.escape(self.line)}</text></svg>')

    def write(self, path):
        """The notebook, and its band in images/ next to it."""
        folder = os.path.join(os.path.dirname(os.path.abspath(path)), "images")
        os.makedirs(folder, exist_ok=True)
        with open(os.path.join(folder, f"banner-{self.number}.svg"), "w", encoding="utf-8") as f:
            f.write(self.banner_svg())
        nb = nbformat.v4.new_notebook()
        nb.cells = self.cells
        nb.metadata["kernelspec"] = {"name": "python3", "display_name": "Python 3", "language": "python"}
        nbformat.write(nb, path)
        return path


if __name__ == "__main__":       # a check of the boxes
    assert styled("**What the output shows.** five edges.").startswith("> <img") and "Five edges" in styled("**What the output shows.** five edges.")
    assert "**Tip**" in styled("> **Tip:** use it.") and "**Key idea**" in styled("> **Key idea:** one file.")
    print("ok")
