"""The questions to ask the AI agent (docs/guides/ask-questions.md): the page (rust/duckosm-edit/page/ask.js, QUESTIONS) and the guide have the same questions in the
same groups, and the examples of Ask come from them. Reading two files only: no browser."""
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
ASK = (ROOT / "rust" / "duckosm-edit" / "page" / "ask.js").read_text(encoding="utf-8")
GUIDE = (ROOT / "docs" / "guides" / "ask-questions.md").read_text(encoding="utf-8")


def _groups():
    block = ASK[ASK.index("const QUESTIONS = ["):]
    block = block[:block.index("];")]
    return [(t, re.findall(r'"((?:[^"\\]|\\.)*)"', items)) for t, items in re.findall(r'\{ title: "([^"]+)", items: \[(.*?)\] \}', block, flags=re.S)]


def test_every_group_and_question_is_in_the_guide_in_order():
    groups = _groups()
    assert len(groups) >= 6 and all(items for _, items in groups)
    for title, items in groups:
        section = GUIDE.split(f"## {title}\n", 1)
        assert len(section) == 2, f"the guide has no group {title!r}"
        body = section[1].split("\n## ", 1)[0]
        assert [l[2:] for l in body.splitlines() if l.startswith("- ")] == items, f"the questions of {title!r} differ"


def test_the_examples_of_ask_come_from_the_groups():
    assert "examples: QUESTIONS" in ASK, "the examples are made from QUESTIONS, not written twice"
