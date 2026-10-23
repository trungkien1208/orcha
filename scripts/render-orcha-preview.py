#!/usr/bin/env python3
"""Render README screenshots from the orcha_preview test (fake demo data only).

    ORCHA_PREVIEW_OUT=/tmp/dark.json ORCHA_PREVIEW_THEME=orcha-dark \
        cargo test --bin herdr client::shell::tests::orcha_layout::orcha_preview -- --ignored --exact
    python3 scripts/render-orcha-preview.py /tmp/dark.json docs/orcha/images/dark.png

Themes: orcha (light), orcha-dark. ORCHA_PREVIEW_POPUP=1 adds CONTINUE ON,
ORCHA_PREVIEW_OPEN_WITH=1 adds OPEN WITH. Needs Pillow, fontTools and
JetBrains Mono in ~/Library/Fonts.
"""
import json, sys
from PIL import Image, ImageDraw, ImageFont
from fontTools.ttLib import TTFont

src, out = sys.argv[1], sys.argv[2]
j = json.load(open(src))
W, H = j['width'], j['height']
CW, CH, SIZE = 18, 40, 30
FD = '/Users/hibari/Library/Fonts/'
primary = {False: FD + 'JetBrainsMono-Regular.ttf', True: FD + 'JetBrainsMono-Bold.ttf'}
fallbacks = ['/System/Library/Fonts/Menlo.ttc', '/System/Library/Fonts/Apple Symbols.ttf',
             FD + 'Arial Unicode.ttf', '/System/Library/Fonts/Supplemental/Arial Unicode.ttf']
cmaps = {}
def has(path, ch):
    if path not in cmaps:
        try:
            f = TTFont(path, fontNumber=0, lazy=True)
            cmaps[path] = set(f.getBestCmap().keys())
        except Exception:
            cmaps[path] = set()
    return ord(ch) in cmaps[path]
fonts = {}
def font(path, size=SIZE):
    key = (path, size)
    if key not in fonts:
        fonts[key] = ImageFont.truetype(path, size, index=0)
    return fonts[key]
def pick(ch, bold):
    if has(primary[bold], ch):
        return font(primary[bold])
    for path in fallbacks:
        try:
            if has(path, ch):
                return font(path, int(SIZE * 0.9))
        except OSError:
            pass
    return font(primary[bold])

def col(v, d):
    return '#%06x' % (v & 0xFFFFFF) if v >= 0x2000000 else d

cells = j['cells']
bg0 = col(cells[0][2], '#1a1916')
img = Image.new('RGB', (W * CW, H * CH), bg0)
d = ImageDraw.Draw(img)
BLOCKS = {'▀': (0, 0, 1, .5), '▄': (0, .5, 1, 1), '▌': (0, 0, .5, 1), '▐': (.5, 0, 1, 1),
          '█': (0, 0, 1, 1), '▏': (0, 0, .125, 1), '▕': (.875, 0, 1, 1),
          '▖': (0, .5, .5, 1), '▗': (.5, .5, 1, 1), '▘': (0, 0, .5, .5), '▝': (.5, 0, 1, .5), '▁': (0, .875, 1, 1), '▔': (0, 0, 1, .125)}
for y in range(H):
    for x in range(W):
        s, fg, bg, m = cells[y * W + x]
        X, Y = x * CW, y * CH
        bgc = col(bg, bg0)
        fgc = col(fg, '#ece6d6')
        if m & 0x40:  # reversed
            bgc, fgc = fgc, bgc
        d.rectangle([X, Y, X + CW - 1, Y + CH - 1], fill=bgc)
        if not s.strip():
            continue
        if s in ('\U0001FB7C', '\U0001FB7F'):  # corner: side eighth + bottom eighth
            left = s == '\U0001FB7C'
            d.rectangle([X if left else X + CW * .875, Y, X + CW * .125 if left else X + CW - 1, Y + CH - 1], fill=fgc)
            d.rectangle([X, Y + CH * .875, X + CW - 1, Y + CH - 1], fill=fgc)
            continue
        if s in BLOCKS:  # draw block elements exactly so edges meet
            a, b, c, e = BLOCKS[s]
            d.rectangle([X + a * CW, Y + b * CH, X + c * CW - 1, Y + e * CH - 1], fill=fgc)
            continue
        f = pick(s[0], bool(m & 1))
        d.text((X + CW / 2, Y + CH / 2), s, fill=fgc, font=f, anchor='mm')
img.save(out, optimize=True)
print(out, img.size)
