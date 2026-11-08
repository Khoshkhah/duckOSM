"""The questions to ask the AI agent (docs/guides/ask-questions.md). One source: web/ask/questions.js. The page (rust/duckosm-edit/page/ask.js) imports it and gives
it to Ask as the examples, the server of the page has it in the program, and the guide lists the same questions in the same groups. Reading files only: no browser
(tests/test_ask_page.py shows the examples in a browser)."""
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SOURCE = (ROOT / "web" / "ask" / "questions.js").read_text(encoding="utf-8")
ASK = (ROOT / "rust" / "duckosm-edit" / "page" / "ask.js").read_text(encoding="utf-8")
LIB = (ROOT / "rust" / "duckosm-edit" / "src" / "lib.rs").read_text(encoding="utf-8")
GUIDE = (ROOT / "docs" / "guides" / "ask-questions.md").read_text(encoding="utf-8")
# the six general groups of roadstyle's Ask (GENERAL in ask_panel.js): they show first, so a title of the page must not repeat one
GENERAL = {"Know the data", "Find and count", "Show on the map", "Colour and focus", "The map and the layers", "Charts"}


def _groups():
    block = SOURCE[SOURCE.index("export const QUESTIONS = ["):]
    return [(t, re.findall(r'^\s*"((?:[^"\\]|\\.)*)",$', items, flags=re.M))
            for t, items in re.findall(r'\{ title: "([^"]+)", items: \[(.*?)\] \}', block, flags=re.S)]


def test_every_group_has_a_heading_and_questions_that_are_text():
    groups = _groups()
    assert len(groups) >= 6
    for title, items in groups:
        assert title.strip() and title not in GENERAL, f"the title {title!r} is empty or is a general group of roadstyle"
        assert items, f"the group {title!r} has no question"
        assert all(q.strip() and q.endswith(("?", ".")) for q in items), f"a question of {title!r} is empty or is not a sentence"
    # every quoted line of a group is a question: the regex above did not skip a line
    assert sum(len(i) for _, i in groups) == len(re.findall(r'^\s*"[^"]*",$', SOURCE, flags=re.M))


def test_no_group_and_no_question_twice():
    groups = _groups()
    titles = [t for t, _ in groups]
    questions = [q for _, items in groups for q in items]
    assert len(set(titles)) == len(titles), "a group title twice"
    assert len(set(questions)) == len(questions), "a question twice"


def test_every_group_and_question_is_in_the_guide_in_order():
    for title, items in _groups():
        section = GUIDE.split(f"## {title}\n", 1)
        assert len(section) == 2, f"the guide has no group {title!r}"
        body = section[1].split("\n## ", 1)[0]
        assert [l[2:] for l in body.splitlines() if l.startswith("- ")] == items, f"the questions of {title!r} differ"


def test_the_page_gives_the_groups_to_ask():
    assert 'import("./ask/questions.js")' in ASK and "examples: QUESTIONS" in ASK, "the examples of Ask are the groups of web/ask/questions.js"
    assert "const QUESTIONS" not in ASK, "the questions are written once, in web/ask/questions.js"
    assert 'include_str!("../../../web/ask/questions.js")' in LIB, "the server has the file in the program and serves it at /ask/questions.js"
