#!/usr/bin/env python3
"""Build the website: site/src + site/assets -> build/site (no dependencies).

Each page in site/src/pages/*.html is a content fragment with two header comments,
<!-- title: ... --> and <!-- description: ... -->, wrapped in site/src/layout.html.
Content that lives elsewhere in the repo is pulled in at build time, so it can't drift:

  {{changelog}}  CHANGELOG.md
  {{fidelity}}   docs/fidelity.md
  {{formulas}}   docs/formulas.md (the function list)
  {{shortcuts}}  the menu key equivalents in macos/Sources/Waffle/App/MainMenu.swift
  {{version}}    Cargo.toml's workspace version

Usage:  python3 tools/site/build.py   (or: make site), then open build/site/index.html
"""
import html
import pathlib
import re
import shutil

ROOT = pathlib.Path(__file__).resolve().parents[2]
SRC = ROOT / "site" / "src"
OUT = ROOT / "build" / "site"
REPO = "https://github.com/salilchincholikar/waffle"

NAV = [("features.html", "Features"), ("download.html", "Download"), ("guide.html", "Guide"),
       ("changelog.html", "Changelog"), ("faq.html", "FAQ")]


# ---- A small Markdown subset: headings, paragraphs, lists, tables, `code`, **bold**, links.

def inline(text, base):
    parts = re.split(r"(`[^`]+`)", text)
    out = []
    for p in parts:
        if p.startswith("`") and p.endswith("`") and len(p) > 1:
            out.append(f"<code>{html.escape(p[1:-1])}</code>")
            continue
        p = html.escape(p, quote=False)
        p = re.sub(r"\*\*(.+?)\*\*", r"<b>\1</b>", p)

        def link(m):
            label, url = m.group(1), m.group(2)
            if not re.match(r"[a-z]+:", url) and not url.startswith("#"):
                url = f"{REPO}/blob/main/{base}{url}"
            return f'<a href="{html.escape(url)}">{label}</a>'
        out.append(re.sub(r"\[([^\]]+)\]\(([^)]+)\)", link, p))
    return "".join(out)


def markdown(md, base="", shift=0):
    """HTML for `md`; headings are pushed down `shift` levels. Links resolve under `base`."""
    lines = md.splitlines()
    out, i = [], 0
    while i < len(lines):
        line = lines[i]
        if not line.strip():
            i += 1
            continue
        m = re.match(r"(#{1,6})\s+(.*)", line)
        if m:
            level = min(6, len(m.group(1)) + shift)
            text = m.group(2)
            slug = re.sub(r"[^a-z0-9]+", "-", text.lower()).strip("-")
            out.append(f'<h{level} id="{slug}">{inline(text, base)}</h{level}>')
            i += 1
            continue
        if line.startswith("|"):
            rows = []
            while i < len(lines) and lines[i].startswith("|"):
                cells = [c.strip() for c in lines[i].strip().strip("|").split("|")]
                if not all(re.fullmatch(r":?-+:?", c) for c in cells):
                    rows.append(cells)
                i += 1
            head, body = rows[0], rows[1:]
            t = "<table><thead><tr>" + "".join(f"<th>{inline(c, base)}</th>" for c in head) + "</tr></thead><tbody>"
            t += "".join("<tr>" + "".join(f"<td>{inline(c, base)}</td>" for c in r) + "</tr>" for r in body)
            out.append(t + "</tbody></table>")
            continue
        if re.match(r"\s*[-*]\s", line):
            items = []  # (indent, text)
            while i < len(lines) and (re.match(r"\s*[-*]\s", lines[i]) or (lines[i].startswith("  ") and lines[i].strip() and items)):
                mm = re.match(r"(\s*)[-*]\s+(.*)", lines[i])
                if mm:
                    items.append((len(mm.group(1)), mm.group(2)))
                else:
                    items[-1] = (items[-1][0], items[-1][1] + " " + lines[i].strip())
                i += 1
            html_list, depth = [], -1
            stack = []
            for indent, text in items:
                level = indent // 2
                while stack and stack[-1] > level:
                    html_list.append("</li></ul>"); stack.pop()
                if stack and stack[-1] == level:
                    html_list.append("</li>")
                if not stack or stack[-1] < level:
                    html_list.append("<ul>"); stack.append(level)
                html_list.append(f"<li>{inline(text, base)}")
            while stack:
                html_list.append("</li></ul>"); stack.pop()
            out.append("".join(html_list))
            continue
        para = []
        while i < len(lines) and lines[i].strip() and not re.match(r"(#{1,6}\s|\||\s*[-*]\s)", lines[i]):
            para.append(lines[i].strip())
            i += 1
        out.append(f"<p>{inline(' '.join(para), base)}</p>")
    return "\n".join(out)


# ---- Generated sections

def changelog():
    md = (ROOT / "CHANGELOG.md").read_text()
    sections = re.split(r"^## ", md, flags=re.M)[1:]
    return "\n".join(f'<section class="release">{markdown("## " + s)}</section>' for s in sections)


def fidelity():
    md = (ROOT / "docs" / "fidelity.md").read_text()
    md = re.sub(r"^# .*\n", "", md, count=1)
    return markdown(md, base="docs/", shift=1)


def formulas():
    md = (ROOT / "docs" / "formulas.md").read_text()
    out = []
    for title, body in re.findall(r"^## (.+)\n+(.+)$", md, flags=re.M):
        names = re.findall(r"`([^`]+)`", body)
        if not names:
            continue
        chips = "".join(f"<code>{html.escape(n)}</code>" for n in names)
        out.append(f"<h3>{html.escape(title)}</h3><div class=\"fn-group\">{chips}</div>")
    return "\n".join(out)


MOD = {"control": "⌃", "option": "⌥", "shift": "⇧", "command": "⌘"}
KEYNAME = {"\\r": "↩", "\\u{8}": "⌫", " ": "Space"}


def shortcuts():
    swift = (ROOT / "macos" / "Sources" / "Waffle" / "App" / "MainMenu.swift").read_text()
    item = re.compile(r'item\("([^"]+)",\s*#selector\(.*?\)\)(?:,\s*"((?:\\.|[^"])*)"(?:,\s*(\[[^\]]*\]|\.\w+))?)?\)')
    menus, current = {}, None
    for line in swift.splitlines():
        m = re.search(r'menu\("([^"]+)",\s*\[', line)
        if m:
            current = m.group(1)
            menus.setdefault(current, [])
        for title, key, mods in item.findall(line):
            if not key or current is None:
                continue
            mods = re.findall(r"\.(\w+)", mods) if mods else ["command"]
            keys = "".join(MOD[k] for k in ["control", "option", "shift", "command"] if k in mods)
            label = KEYNAME.get(key, key.upper() if len(key) == 1 else key)
            menus[current].append((title, keys + label))
    out = ['<div class="keys-grid">']
    for menu, rows in menus.items():
        if not rows:
            continue
        out.append(f'<div class="keys"><h3>{html.escape(menu)}</h3><dl>')
        out.extend(f"<dt>{html.escape(t)}</dt><dd><kbd>{html.escape(k)}</kbd></dd>" for t, k in rows)
        out.append("</dl></div>")
    out.append("</div>")
    return "\n".join(out)


def version():
    return re.search(r'^version = "([^"]+)"', (ROOT / "Cargo.toml").read_text(), flags=re.M).group(1)


# ---- Build

def main():
    if OUT.exists():
        shutil.rmtree(OUT)
    shutil.copytree(ROOT / "site" / "assets", OUT / "assets")
    shutil.copy(SRC / "style.css", OUT / "assets" / "style.css")
    (OUT / ".nojekyll").write_text("")
    for extra in ["CNAME"]:
        if (ROOT / "site" / extra).exists():
            shutil.copy(ROOT / "site" / extra, OUT / extra)

    layout = (SRC / "layout.html").read_text()
    generated = {"changelog": changelog, "fidelity": fidelity, "formulas": formulas,
                 "shortcuts": shortcuts, "version": version}
    pages = sorted((SRC / "pages").glob("*.html"))
    for page in pages:
        text = page.read_text()
        for key, fn in generated.items():
            if "{{" + key + "}}" in text:
                text = text.replace("{{" + key + "}}", fn())
        meta = dict(re.findall(r"<!--\s*(title|description):\s*(.*?)\s*-->", text))
        body = re.sub(r"<!--\s*(title|description):.*?-->\n?", "", text)
        nav = "\n      ".join(
            f'<a href="{href}"{" aria-current=page" if href == page.name else ""}>{label}</a>' for href, label in NAV)
        path = "" if page.name == "index.html" else page.name
        out = (layout.replace("{{home_current}}", ' aria-current="page"' if page.name == "index.html" else "")
                     .replace("{{title}}", html.escape(meta.get("title", "Waffle")))
                     .replace("{{description}}", html.escape(meta.get("description", "")))
                     .replace("{{path}}", path)
                     .replace("{{page}}", page.stem)
                     .replace("{{nav}}", nav)
                     .replace("{{content}}", body.strip("\n")))
        leftover = re.findall(r"\{\{\w+\}\}", out)
        assert not leftover, f"{page.name}: unfilled {leftover}"
        (OUT / page.name).write_text(out)
    print(f"built {len(pages)} pages -> {OUT.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
