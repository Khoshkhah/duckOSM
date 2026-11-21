# The notebook manual: the style of a chapter

The notebooks in `docs/notebooks/` are a learning manual: one notebook is one chapter. A reader opens a chapter in
Colab (the badge), in JupyterLab or on the manual page, and learns one part of duckOSM from the start to the end.
Chapter 1 (`docs/notebooks/01_area_to_network.ipynb`) is the model.

A chapter is made by a script, `scripts/notebook_chapter<N>.py`, with the kit `scripts/notebook_book.py`. Do not edit a
chapter notebook by hand: change its script, run the script, then run the notebook to save its outputs.

```bash
python scripts/notebook_chapter1.py docs/notebooks/01_area_to_network.ipynb
cd docs/notebooks && IPYTHONDIR=../../book/ipython DUCKOSM_MAP_SHOW=plot jupyter nbconvert --to notebook --execute --inplace 01_area_to_network.ipynb
```

`IPYTHONDIR=book/ipython` gives the matplotlib pictures two times the pixels at the same size (retina). In the book,
each output picture can zoom in the page: the buttons +, - and Fit, Ctrl + mouse wheel or two fingers, and a drag
moves the zoomed picture. The button Full, or a click on the picture, opens it full screen (`lightbox: auto` in
`book/_quarto.yml`). There, a click zooms in where you click, and a second click shows the whole picture again
(`book/zoom.html`, with `book/panzoom.min.js`).

## What Colab draws

Colab removes inline CSS from Markdown. So a chapter uses only what Colab draws:

| Part | Made of |
|---|---|
| The band | an SVG picture, `docs/notebooks/images/banner-<N>.svg` (the kit writes it) |
| The info card | a Markdown table |
| A box | a quote block with a colored icon and a bold title |
| An answer | `<details><summary>Show the answer</summary> ... </details>` |
| A diagram or a map | an SVG or PNG picture in `docs/notebooks/images/`, from the raw GitHub address |

Never use `<div style="...">`, colors in HTML, or `!!! tip`: Colab shows them as plain text. The manual gives each
kind of box its colors in `docs/assets/extra.css` (by the icon of the box).

## The parts of a chapter, in this order

1. **The top** (`Chapter(...)`): the `#` title, the badges, the band, and the card (You learn, Time, Extras, Next).
2. **The install cell** (the kit writes it): in Colab it installs the nightly wheel with the extras of the chapter.
3. **The concepts**: the ideas of the chapter in full text, with a diagram when it helps, and one Key idea box.
4. **One section for each task**: a `##` heading (a task, for example "Seven ways to choose an area"; `###` for
   its parts), then:
   - text: what the call does and when to use it; a table of its main arguments when it has several;
   - a code cell with a comment on each line that does something;
   - a **What the output shows** box with the real numbers of the output.
5. **Try it yourself** (`ch.exercises`): two to four exercises, each with its answer behind a fold.
6. **Words in this chapter** (`ch.glossary`): the new words of the chapter.
7. **Summary** (`ch.summary`): a table of all the calls of the chapter, the reference page and the next chapter.

Each `##` and `###` heading is an entry of the table of contents in Colab and on the manual page, so give each task
its own heading.

## The boxes

Write a box as a paragraph that starts with its marker; the kit turns it into the box.

| Box | Marker in the script | Use it for |
|---|---|---|
| What the output shows (blue) | `**What the output shows.**` | after each output: what the numbers and the card mean |
| Key idea (purple) | `> **Key idea:**` | one idea of a section that the reader must keep |
| Tip (green) | `> **Tip:**` | a good way to do something, a short way, a tool |
| Warning (amber) | `> **Warning:**` | a trap: a wrong area, a slow call, an error and its cause |

Use at most one Key idea for each section. A box is one paragraph.

## Pictures

- Put the pictures in `docs/notebooks/images/` and use the raw GitHub address (`IMAGES` in the kit), so Colab finds
  them. Use SVG for diagrams, PNG for maps.
- A diagram has light colors on a white background, dark text, and the blue `#1f6feb`, the green `#2ea043`, the red
  `#d1242f` (driving), the green `#1a7f37` (walking) and the blue `#0969da` (cycling).
- Numbers in a picture must be the numbers of the chapter's own outputs, or leave them out.

## Text

- ASD-STE100 Simplified Technical English: short sentences, one idea in each, active voice, `can` and `must`.
- The text tells the reader what the code does and why, before the code. The box after the output tells what the
  output means, with its real numbers.
- Every public call of the SDK is in one chapter at least, with text.
- Test on Monaco (`osm_id=2220322`, the city: the name "Monaco" finds the country with its sea).
