"""Build "Agent Icons": a tiny TTF with the Claude, OpenAI and Cursor logos and
the orcha whale at private-use codepoints, so a terminal can draw them as text (one color, any fg).

usage: python build_agent_icons.py <logos dir> <out.ttf>
"""
import re
import sys
from pathlib import Path

import pathops
from fontTools.fontBuilder import FontBuilder
from fontTools.pens.cu2quPen import Cu2QuPen
from fontTools.pens.transformPen import TransformPen
from fontTools.pens.ttGlyphPen import TTGlyphPen
from fontTools.svgLib.path import parse_path

UPM = 1000
ADVANCE = 600  # one monospace cell (JetBrains Mono / Fira Code are 600/1000)
GLYPHS = {
    # glyph name: (svg file, codepoint, even-odd fill)
    "claude": ("claude.svg", 0x100000, False),
    "openai": ("openai.svg", 0x100001, False),
    "cursor": ("cursor.svg", 0x100002, True),
    # orcha's own icon (ours, shipped in this folder): a whale in a matcha bowl
    "orcha": ("orcha.svg", 0x100003, True),
}


def outline(svg_text, evenodd):
    """The svg's paths (24x24 viewBox) as one nonzero-winding pathops.Path."""
    path = pathops.Path()
    path.fillType = pathops.FillType.EVEN_ODD if evenodd else pathops.FillType.WINDING
    for d in re.findall(r'<path[^>]*\sd="([^"]+)"', svg_text):
        parse_path(d, path.getPen())
    path.simplify(fix_winding=True)  # TrueType wants nonzero winding, no overlaps
    return path


def glyph_of(path):
    # 24 svg units -> 600 font units, flip y (svg grows down), sit on the baseline
    # and lift a little so the logo is centred on lowercase/cap height.
    scale = ADVANCE / 24
    pen = TTGlyphPen(None)
    # TrueType outlines are quadratic: convert the svg's cubic curves
    quad = Cu2QuPen(pen, max_err=1.0, reverse_direction=True)
    path.draw(TransformPen(quad, (scale, 0, 0, -scale, 0, 24 * scale - 30)))
    return pen.glyph()


def main(logos, out):
    glyphs = {".notdef": TTGlyphPen(None).glyph()}
    cmap = {}
    for name, (file, cp, evenodd) in GLYPHS.items():
        glyphs[name] = glyph_of(outline((Path(logos) / file).read_text(), evenodd))
        cmap[cp] = name

    fb = FontBuilder(UPM, isTTF=True)
    fb.setupGlyphOrder(list(glyphs))
    fb.setupCharacterMap(cmap)
    fb.setupGlyf(glyphs)
    fb.setupHorizontalMetrics({n: (ADVANCE, 0) for n in glyphs})
    fb.setupHorizontalHeader(ascent=800, descent=-200)
    # full name set: without fullName / psName macOS files the font under .LastResort
    fb.setupNameTable({
        "familyName": "Agent Icons",
        "styleName": "Regular",
        "uniqueFontIdentifier": "AgentIcons-Regular-1.0",
        "fullName": "Agent Icons Regular",
        "version": "Version 1.000",
        "psName": "AgentIcons-Regular",
    })
    fb.setupOS2(sTypoAscender=800, sTypoDescender=-200, usWinAscent=800, usWinDescent=200,
                xAvgCharWidth=ADVANCE)
    fb.setupPost(isFixedPitch=1)
    fb.save(out)
    print(f"wrote {out}: " + ", ".join(f"{n}=U+{cp:X}" for n, (_, cp, _) in GLYPHS.items()))


if __name__ == "__main__":
    main(*sys.argv[1:3])
