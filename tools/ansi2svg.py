#!/usr/bin/env python3
"""Convert `amdtop --frames ... --out file.ansi` output into an SVG screenshot.

Development helper only (the app itself is pure Rust). Usage:

    amdtop --demo --frames 1 --size 140x40 --out shot.ansi
    python3 tools/ansi2svg.py shot.ansi docs/dash.svg
"""

import re
import sys

ANSI16 = {
    30: "#3f3f3f", 31: "#cc0000", 32: "#4e9a06", 33: "#c4a000", 34: "#3465a4",
    35: "#75507b", 36: "#06989a", 37: "#d3d7cf", 90: "#555753", 91: "#ef2929",
    92: "#8ae234", 93: "#fce94f", 94: "#729fcf", 95: "#ad7fa8", 96: "#34e2e2",
    97: "#eeeeec",
}
BG16 = {
    40: "#2e3436", 41: "#cc0000", 42: "#4e9a06", 43: "#c4a000", 44: "#3465a4",
    45: "#75507b", 46: "#06989a", 47: "#d3d7cf", 100: "#555753", 101: "#ef2929",
    102: "#8ae234", 103: "#fce94f", 104: "#729fcf", 105: "#ad7fa8",
    106: "#34e2e2", 107: "#eeeeec",
}
DEFAULT_FG = "#d3d7cf"
DEFAULT_BG = "#000000"
SGR = re.compile(r"\x1b\[([0-9;]*)m")
CELL_W = 8.77          # measured advance of the rendered monospace font
LINE_H = 15.0
FONT_SIZE = 12.5
PAD = 12.0


def xterm256(index):
    index = int(index)
    if index < 16:
        return {
            0: "#000000", 1: "#cc0000", 2: "#4e9a06", 3: "#c4a000", 4: "#3465a4",
            5: "#75507b", 6: "#06989a", 7: "#d3d7cf", 8: "#555753", 9: "#ef2929",
            10: "#8ae234", 11: "#fce94f", 12: "#729fcf", 13: "#ad7fa8",
            14: "#34e2e2", 15: "#eeeeec",
        }[index]
    if index < 232:
        index -= 16
        r, g, b = index // 36, (index % 36) // 6, index % 6
        level = lambda v: 0 if v == 0 else 55 + 40 * v
        return "#%02x%02x%02x" % (level(r), level(g), level(b))
    grey = 8 + (index - 232) * 10
    return "#%02x%02x%02x" % (grey, grey, grey)


def escape(text):
    return text.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")


def runs(line):
    """Split an ANSI line into (fg, bg, text) runs.

    Handles the three forms btop-style output uses: basic 30-37/90-97,
    indexed ``38;5;N`` and truecolour ``38;2;R;G;B``.
    """
    out = []
    fg, bg = DEFAULT_FG, DEFAULT_BG
    pos = 0
    for match in SGR.finditer(line):
        text = line[pos:match.start()]
        if text:
            out.append((fg, bg, text))
        fields = match.group(1).split(";")
        index = 0
        while index < len(fields):
            field = fields[index]
            if field == "" or field == "0":
                fg, bg = DEFAULT_FG, DEFAULT_BG
            elif field == "39":
                fg = DEFAULT_FG
            elif field == "49":
                bg = DEFAULT_BG
            elif field == "38" and index + 4 < len(fields) and fields[index + 1] == "2":
                fg = "#%02x%02x%02x" % (
                    int(fields[index + 2]),
                    int(fields[index + 3]),
                    int(fields[index + 4]),
                )
                index += 4
            elif field == "48" and index + 4 < len(fields) and fields[index + 1] == "2":
                bg = "#%02x%02x%02x" % (
                    int(fields[index + 2]),
                    int(fields[index + 3]),
                    int(fields[index + 4]),
                )
                index += 4
            elif field == "38" and index + 2 < len(fields) and fields[index + 1] == "5":
                fg = xterm256(fields[index + 2])
                index += 2
            elif field == "48" and index + 2 < len(fields) and fields[index + 1] == "5":
                bg = xterm256(fields[index + 2])
                index += 2
            elif field.isdigit() and int(field) in ANSI16:
                fg = ANSI16[int(field)]
            elif field.isdigit() and int(field) in BG16:
                bg = BG16[int(field)]
            index += 1
        pos = match.end()
    text = line[pos:]
    if text:
        out.append((fg, bg, text))
    return out


def convert(src, dst):
    with open(src, encoding="utf-8") as handle:
        lines = handle.read().split("\n")
    if lines and lines[-1] == "":
        lines.pop()

    background = []          # rects must live outside <text> elements
    body = []
    widest = 0
    for index, line in enumerate(lines):
        column = 0
        y = PAD + (index + 1) * LINE_H
        spans = []
        for fg, bg, text in runs(line):
            if bg != DEFAULT_BG and text.strip():
                background.append(
                    '<rect x="%.2f" y="%.2f" width="%.2f" height="%.2f" fill="%s"/>'
                    % (PAD + column * CELL_W, y - FONT_SIZE, len(text) * CELL_W,
                       LINE_H, bg)
                )
            spans.append('<tspan fill="%s">%s</tspan>' % (fg, escape(text)))
            column += len(text)
        widest = max(widest, column)
        body.append(
            '<text x="%.2f" y="%.2f" xml:space="preserve">%s</text>'
            % (PAD, y, "".join(spans))
        )

    width = int(widest * CELL_W + 2 * PAD)
    height = int(len(lines) * LINE_H + 2 * PAD)
    svg = [
        '<?xml version="1.0" encoding="UTF-8"?>',
        '<svg xmlns="http://www.w3.org/2000/svg" width="%d" height="%d" '
        'viewBox="0 0 %d %d">' % (width, height, width, height),
        '<rect width="100%%" height="100%%" fill="%s"/>' % DEFAULT_BG,
        '<g font-family="DejaVu Sans Mono,Menlo,Consolas,monospace" '
        'font-size="%s" font-weight="500">' % FONT_SIZE,
    ]
    svg.extend(background)
    svg.extend(body)
    svg.append("</g></svg>")
    with open(dst, "w", encoding="utf-8") as handle:
        handle.write("\n".join(svg))
    return width, height


if __name__ == "__main__":
    if len(sys.argv) != 3:
        print(__doc__)
        sys.exit(2)
    w, h = convert(sys.argv[1], sys.argv[2])
    print("wrote %s (%dx%d)" % (sys.argv[2], w, h))
