#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT_DIR="${1:-$ROOT/docs/assets}"
mkdir -p "$OUT_DIR"
OUT_PNG="$OUT_DIR/court-jester-scorecard.png"

python3 - "$OUT_PNG" "$ROOT/cj-hero.png" <<'PY'
from pathlib import Path
import sys
import textwrap
from PIL import Image, ImageDraw, ImageFont, ImageFilter

out_path = Path(sys.argv[1])
hero_path = Path(sys.argv[2])
W, H = 1500, 2100

fonts = {
    "impact": "/System/Library/Fonts/Supplemental/Impact.ttf",
    "arial_bold": "/System/Library/Fonts/Supplemental/Arial Bold.ttf",
    "georgia": "/System/Library/Fonts/Supplemental/Georgia.ttf",
    "georgia_bold": "/System/Library/Fonts/Supplemental/Georgia Bold.ttf",
}

def font(name, size):
    try:
        return ImageFont.truetype(fonts[name], size)
    except Exception:
        return ImageFont.load_default()

def cover(im, size):
    target_w, target_h = size
    scale = max(target_w / im.width, target_h / im.height)
    resized = im.resize((round(im.width * scale), round(im.height * scale)), Image.Resampling.LANCZOS)
    left = (resized.width - target_w) // 2
    top = (resized.height - target_h) // 2
    return resized.crop((left, top, left + target_w, top + target_h))

def rect(draw, xy, outline, width=3, fill=None):
    draw.rectangle(xy, fill=fill, outline=outline, width=width)

def text(draw, xy, value, fnt, fill, anchor=None):
    draw.text(xy, value, font=fnt, fill=fill, anchor=anchor)

def center_text(draw, cx, y, value, fnt, fill):
    draw.text((cx, y), value, font=fnt, fill=fill, anchor="ma")

def wrap_center(draw, cx, y, value, fnt, fill, width_chars, line_gap=8):
    lines = textwrap.wrap(value, width=width_chars)
    for idx, line in enumerate(lines):
        draw.text((cx, y + idx * (fnt.size + line_gap)), line, font=fnt, fill=fill, anchor="ma")

def wrap_left(draw, x, y, value, fnt, fill, width_chars, line_gap=8):
    lines = textwrap.wrap(value, width=width_chars)
    for idx, line in enumerate(lines):
        draw.text((x, y + idx * (fnt.size + line_gap)), line, font=fnt, fill=fill)

bg = Image.new("RGB", (W, H), "#07090d")
if hero_path.exists():
    hero = Image.open(hero_path).convert("RGB")
    blurred = cover(hero, (W, H)).filter(ImageFilter.GaussianBlur(24))
    wash = Image.new("RGB", (W, H), "#07090d")
    bg = Image.blend(blurred, wash, 0.72)

img = bg.convert("RGBA")
d = ImageDraw.Draw(img)

ink = "#fbf6e8"
dim = "#c5bfae"
gold = "#e1bf63"
green = "#93eba4"
blue = "#82dcff"
panel = "#0f141c"
panel2 = "#131a14"

rect(d, (34, 34, W - 34, H - 34), gold, 8)
rect(d, (82, 82, W - 82, H - 82), "#27303a", 3, fill="#0d1118dd")
rect(d, (120, 104, W - 120, 210), gold, 4, fill="#172017")
text(d, (160, 132), "COURT JESTER", font("impact", 58), ink)
text(d, (W - 160, 150), "VERIFIER // LOOP BREAKER // PUBLIC ALPHA", font("arial_bold", 24), gold, anchor="ra")

if hero_path.exists():
    hero_art = cover(Image.open(hero_path).convert("RGB"), (1180, 620))
    overlay = Image.new("RGBA", hero_art.size, "#06080bc8")
    hero_art = Image.alpha_composite(hero_art.convert("RGBA"), overlay)
    img.alpha_composite(hero_art, (160, 270))
rect(d, (160, 270, 1340, 890), gold, 4)

center_text(d, W / 2, 984, "TCG SCORECARD", font("georgia_bold", 42), ink)

cards = [
    (160, 1060, 500, 1370, green, "UTILITY LIFT", "+9.4 pp", "vs baseline on the primary causal matrix"),
    (580, 1060, 920, 1370, gold, "FALSE-POSITIVE", "270/270", "known-good and upstream replay stayed clean"),
    (1000, 1060, 1340, 1370, blue, "ROBUSTNESS x2", "156/156", "still best after giving controls extra budget"),
]
for x1, y1, x2, y2, color, label, value, note in cards:
    rect(d, (x1, y1, x2, y2), color, 4, fill=panel2 if color == green else panel)
    center_text(d, (x1 + x2) / 2, y1 + 42, label, font("arial_bold", 27), color)
    center_text(d, (x1 + x2) / 2, y1 + 142, value, font("impact", 70), ink)
    wrap_center(d, (x1 + x2) / 2, y1 + 230, note, font("georgia", 21), dim, 28, 6)

rect(d, (160, 1450, 770, 1815), gold, 3, fill="#111820")
text(d, (196, 1504), "MATCHED CONTROLS", font("arial_bold", 30), ink)
rows = [
    ("Baseline", "208/234", dim),
    ("Public repair x1", "205/234", dim),
    ("Blind retry x1", "216/234", dim),
    ("Verify-only x1", "230/234", green),
    ("Proving ground", "25/36", green),
]
for i, (label, value, color) in enumerate(rows):
    y = 1575 + i * 50
    text(d, (200, y), label, font("georgia", 27), dim)
    text(d, (700, y), value, font("georgia_bold", 29), color, anchor="ra")

rect(d, (820, 1450, 1340, 1815), "#6a765f", 3, fill="#10161d")
text(d, (856, 1504), "COUNTEREXAMPLE ENGINE", font("arial_bold", 28), "#fff3d9")
text(d, (856, 1574), "Turns plausible patches", font("georgia_bold", 38), ink)
text(d, (856, 1624), "into concrete failing repros", font("georgia_bold", 38), ink)
text(d, (856, 1718), "Strongest today on Python/TypeScript", font("georgia", 27), dim)
text(d, (856, 1756), "library + utility code", font("georgia", 27), dim)

rect(d, (160, 1885, 1340, 1990), gold, 3, fill="#151b24")
text(d, (190, 1930), "ALPHA NOTE", font("arial_bold", 25), gold)
wrap_left(d, 400, 1919, "Experimental verifier for agent repair loops // benchmarked, not universal", font("georgia", 27), ink, 52, 6)
text(d, (1310, 1950), "court-jester", font("arial_bold", 22), green, anchor="ra")

out_path.parent.mkdir(parents=True, exist_ok=True)
img.convert("RGB").save(out_path, quality=95)
print(f"Rendered {out_path}")
PY
