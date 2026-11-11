"""Render the generated Markdown guide as a GitHub Pages site."""

import re
import sys
from html import escape
from pathlib import Path

from markdown_it import MarkdownIt


def render_guide(markdown: str) -> str:
    parser = MarkdownIt("commonmark", {"html": False})
    tokens = parser.parse(markdown)
    sections = []
    for index, token in enumerate(tokens):
        if token.type == "heading_open" and token.tag == "h2":
            title = tokens[index + 1].content
            anchor = re.sub(r"[^a-z0-9]+", "-", title.lower()).strip("-")
            token.attrSet("id", anchor)
            sections.append(f'<li><a href="#{anchor}">{escape(title)}</a></li>')

    content = parser.renderer.render(tokens, parser.options, {})
    toc = "\n".join(sections)
    heading_end = content.index("</h1>") + len("</h1>")
    return f"""<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Pizza Lunch Bot guide</title>
  <link rel="stylesheet" href="./guide.css">
</head>
<body>
<main>
{content[:heading_end]}
<nav aria-label="Guide sections">
  <ul>{toc}</ul>
</nav>
{content[heading_end:]}
</main>
</body>
</html>
"""


if __name__ == "__main__":
    source, destination = map(Path, sys.argv[1:])
    destination.write_text(render_guide(source.read_text(encoding="utf-8")), encoding="utf-8")
