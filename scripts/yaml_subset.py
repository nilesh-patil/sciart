#!/usr/bin/env python3
"""A parser for the YAML subset the GitHub workflows use (SPEC-008 DIST-R10).

The release checks read `.github/workflows/*.yml` as data, with nothing but the Python
standard library, so they run the same under plain `python3` in CI and locally. This
covers what the workflows use and refuses everything else loudly (`YamlError`), so a
construct it does not understand fails the check instead of being misread:

- block mappings and sequences by indentation (spaces only), `- key: value` items;
- plain, single- and double-quoted scalars; `true`/`false`, integers and `null`/`~`;
- block scalars `|`, `|-`, `|+`, `>`, `>-` (literal text kept, folded joined);
- flow sequences and mappings of scalars (`["v*"]`, `[a, b]`, `{}`);
- comments (a line starting with `#`, or ` #` after a value).

Not supported (refused): anchors and aliases, tags, `? ` complex keys, several
documents, tabs for indentation, nested flow collections.
"""

from __future__ import annotations

import re
from dataclasses import dataclass

KEY = re.compile(r"^([A-Za-z0-9_.\-/]+|\"[^\"]*\"|'[^']*')\s*:(?:\s+(.*)|)$")


class YamlError(ValueError):
    pass


@dataclass
class Line:
    number: int
    indent: int
    text: str  # without the indentation


def parse(text: str) -> object:
    raw = text.split("\n")
    lines: list[Line] = []
    for i, line in enumerate(raw, 1):
        if "\t" in line[: len(line) - len(line.lstrip())]:
            raise YamlError(f"line {i}: tab in indentation")
        stripped = line.strip()
        if stripped in ("---", "...") and i > 1:
            raise YamlError(f"line {i}: several documents")
        lines.append(Line(i, len(line) - len(line.lstrip(" ")), line.lstrip(" ")))
    parser = _Parser(lines)
    value = parser.node(-1)
    parser.skip_blank()
    if parser.pos < len(lines):
        bad = lines[parser.pos]
        raise YamlError(f"line {bad.number}: unexpected indentation: {bad.text[:40]!r}")
    return value


def _is_blank(line: Line) -> bool:
    return not line.text or line.text.startswith("#")


def _strip_comment(value: str) -> str:
    """A plain scalar ends at ` #`; quoted scalars are handled before this."""
    found = re.search(r"\s#", value)
    return (value[: found.start()] if found else value).rstrip()


def _scalar(value: str, number: int) -> object:
    value = value.strip()
    if not value:
        return None
    head = value[0]
    if head in "&*!" or value.startswith(("? ", "%")):
        raise YamlError(f"line {number}: unsupported YAML ({value[:20]!r})")
    if head in "\"'":
        text, rest = _quoted(value, number)
        if rest.strip() and not rest.strip().startswith("#"):
            raise YamlError(f"line {number}: text after a quoted scalar: {rest!r}")
        return text
    if head == "[":
        return _flow_sequence(value, number)
    if head == "{":
        return _flow_mapping(value, number)
    plain = _strip_comment(value)
    if plain in ("true", "false"):
        return plain == "true"
    if plain in ("null", "~"):
        return None
    if re.fullmatch(r"-?\d+", plain):
        return int(plain)
    return plain


def _quoted(value: str, number: int) -> tuple[str, str]:
    quote = value[0]
    out = []
    i = 1
    while i < len(value):
        c = value[i]
        if quote == "'" and c == "'":
            if value[i + 1 : i + 2] == "'":
                out.append("'")
                i += 2
                continue
            return "".join(out), value[i + 1 :]
        if quote == '"' and c == "\\":
            esc = value[i + 1 : i + 2]
            out.append({"n": "\n", "t": "\t", '"': '"', "\\": "\\", "/": "/"}.get(esc, "\\" + esc))
            i += 2
            continue
        if quote == '"' and c == '"':
            return "".join(out), value[i + 1 :]
        out.append(c)
        i += 1
    raise YamlError(f"line {number}: unterminated quoted scalar")


def _flow_items(body: str, number: int) -> list[str]:
    if any(c in body for c in "[]{}"):
        raise YamlError(f"line {number}: nested flow collections are not supported")
    items, cur, quote = [], [], None
    for c in body:
        if quote:
            cur.append(c)
            if c == quote:
                quote = None
        elif c in "\"'":
            quote = c
            cur.append(c)
        elif c == ",":
            items.append("".join(cur).strip())
            cur = []
        else:
            cur.append(c)
    if quote:
        raise YamlError(f"line {number}: unterminated quote in a flow collection")
    last = "".join(cur).strip()
    if last:
        items.append(last)
    return items


def _closing(value: str, close: str, number: int) -> str:
    rest = _strip_comment(value)
    if not rest.endswith(close):
        raise YamlError(f"line {number}: a flow collection must close on its line")
    return rest[1:-1]


def _flow_sequence(value: str, number: int) -> list[object]:
    return [_scalar(item, number) for item in _flow_items(_closing(value, "]", number), number)]


def _flow_mapping(value: str, number: int) -> dict[str, object]:
    out: dict[str, object] = {}
    for item in _flow_items(_closing(value, "}", number), number):
        key, sep, val = item.partition(":")
        if not sep:
            raise YamlError(f"line {number}: flow mapping item without a colon: {item!r}")
        out[_key(key.strip(), number)] = _scalar(val, number)
    return out


def _key(key: str, number: int) -> str:
    if key[:1] in "\"'":
        return _quoted(key, number)[0]
    return key


class _Parser:
    def __init__(self, lines: list[Line]) -> None:
        self.lines = lines
        self.pos = 0

    def skip_blank(self) -> None:
        while self.pos < len(self.lines) and _is_blank(self.lines[self.pos]):
            self.pos += 1

    def peek(self) -> Line | None:
        self.skip_blank()
        return self.lines[self.pos] if self.pos < len(self.lines) else None

    def node(self, parent: int) -> object:
        line = self.peek()
        if line is None or line.indent <= parent and not (line.text.startswith("- ") or line.text == "-"):
            return None
        if line.indent < parent:
            return None
        if line.text.startswith("- ") or line.text == "-":
            return self.sequence(line.indent)
        return self.mapping(line.indent)

    def mapping(self, indent: int) -> dict[str, object]:
        out: dict[str, object] = {}
        while (line := self.peek()) is not None and line.indent == indent:
            if line.text.startswith("- ") or line.text == "-":
                break
            found = KEY.match(line.text)
            if not found:
                raise YamlError(f"line {line.number}: not a `key: value` line: {line.text[:40]!r}")
            key = _key(found.group(1), line.number)
            if key in out:
                raise YamlError(f"line {line.number}: duplicate key {key!r}")
            self.pos += 1
            out[key] = self.value(found.group(2) or "", indent, line.number)
        nxt = self.peek()
        if nxt is not None and nxt.indent > indent:
            raise YamlError(f"line {nxt.number}: unexpected indentation")
        return out

    def sequence(self, indent: int) -> list[object]:
        out: list[object] = []
        while (line := self.peek()) is not None and line.indent == indent and \
                (line.text.startswith("- ") or line.text == "-"):
            rest = line.text[2:] if line.text.startswith("- ") else ""
            inner = indent + 2 + (len(rest) - len(rest.lstrip(" ")))
            rest = rest.lstrip(" ")
            if rest and KEY.match(rest) and rest[0] not in "\"'[{":
                # `- key: value`: a mapping whose first line starts after the dash.
                self.lines[self.pos] = Line(line.number, inner, rest)
                out.append(self.mapping(inner))
            else:
                self.pos += 1
                out.append(self.value(rest, indent, line.number, item=True))
        return out

    def value(self, text: str, indent: int, number: int, item: bool = False) -> object:
        text = text.strip()
        if text and text[0] in "|>":
            return self.block(text, indent, number)
        if text and not text.startswith("#"):
            return _scalar(text, number)
        nxt = self.peek()
        if nxt is None:
            return None
        if nxt.indent > indent:
            return self.node(indent)
        if not item and nxt.indent == indent and (nxt.text.startswith("- ") or nxt.text == "-"):
            return self.sequence(indent)  # `key:` then a sequence at the key's indent
        return None

    def block(self, header: str, indent: int, number: int) -> str:
        found = re.fullmatch(r"([|>])([+-]?)\s*(#.*)?", header)
        if not found:
            raise YamlError(f"line {number}: unsupported block scalar header {header!r}")
        style, chomp = found.group(1), found.group(2)
        body: list[str] = []
        block_indent = None
        while self.pos < len(self.lines):
            line = self.lines[self.pos]
            if line.text and line.indent <= indent:
                break
            if line.text and block_indent is None:
                block_indent = line.indent
            if line.text and block_indent is not None and line.indent < block_indent:
                raise YamlError(f"line {line.number}: block scalar line less indented than its first")
            body.append(" " * (line.indent - block_indent) + line.text if line.text and block_indent is not None
                        else "")
            self.pos += 1
        while body and body[-1] == "" and chomp != "+":
            body.pop()
        if style == "|":
            text = "\n".join(body)
        else:
            text = re.sub(r"(?<!\n)\n(?!\n)", " ", "\n".join(body))
        return text if chomp == "-" else text + "\n"
