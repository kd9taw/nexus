#!/usr/bin/env python3
"""Generate Tempo's hero banner and social/OG card into docs/img/.

On-theme with the app icon and dark UI: a deep-slate field with an amber->green
spectrum/waterfall motif (the same "signal + tempo" idea as the icon), the
Tempo wordmark, tagline, and the two-tier line. Re-run after wording changes:

    python3 scripts/gen-banner.py

Outputs:
    docs/img/banner.png        1280x640  (README hero)
    docs/img/social-card.png   1200x630  (GitHub social preview / OG image)

Requires Pillow (PIL).
"""
import math
import os
from PIL import Image, ImageDraw, ImageFont

OUT = os.path.normpath(os.path.join(os.path.dirname(__file__), "..", "docs", "img"))
os.makedirs(OUT, exist_ok=True)

FONTS = "/usr/share/fonts/truetype/dejavu"
BOLD = os.path.join(FONTS, "DejaVuSans-Bold.ttf")
REG = os.path.join(FONTS, "DejaVuSans.ttf")
MONO = os.path.join(FONTS, "DejaVuSansMono.ttf")
MONO_B = os.path.join(FONTS, "DejaVuSansMono-Bold.ttf")

BG_TOP = (17, 24, 36)
BG_BOT = (8, 12, 20)
AMBER = (255, 176, 0)
GREEN = (60, 200, 140)
WHITE = (238, 243, 249)
MUTED = (150, 165, 184)


def lerp(a, b, t):
    return tuple(int(a[i] * (1 - t) + b[i] * t) for i in range(len(a)))


def spectrum_height(i, n):
    """Deterministic, organic-looking bar height in [0.06, 1.0]."""
    x = i / n
    v = (
        0.45
        + 0.32 * math.sin(i * 0.55)
        + 0.18 * math.sin(i * 0.17 + 1.3)
        + 0.12 * math.sin(i * 1.7 + 0.4)
    )
    # gentle envelope so the middle is taller than the edges
    env = 0.55 + 0.45 * math.sin(math.pi * x)
    return max(0.06, min(1.0, v * env))


def gradient_bg(w, h):
    img = Image.new("RGB", (w, h))
    px = img.load()
    for y in range(h):
        t = y / (h - 1)
        row = lerp(BG_TOP, BG_BOT, t)
        for x in range(w):
            px[x, y] = row
    return img.convert("RGBA")


def draw_spectrum(w, h, base_y, band_h, alpha_scale=1.0):
    """A transparent layer with an amber->green FFT-style bar spectrum."""
    layer = Image.new("RGBA", (w, h), (0, 0, 0, 0))
    d = ImageDraw.Draw(layer)
    n = 64
    margin = int(w * 0.06)
    avail = w - 2 * margin
    bw = avail / (n * 2 - 1)
    for i in range(n):
        x0 = margin + i * 2 * bw
        bh = band_h * spectrum_height(i, n)
        col = lerp(AMBER, GREEN, i / (n - 1))
        a = int(190 * alpha_scale * (0.45 + 0.55 * (bh / band_h)))
        d.rounded_rectangle(
            [x0, base_y - bh, x0 + bw, base_y],
            radius=max(1, int(bw * 0.4)),
            fill=col + (a,),
        )
    return layer


def grid_lines(w, h, alpha=18):
    layer = Image.new("RGBA", (w, h), (0, 0, 0, 0))
    d = ImageDraw.Draw(layer)
    for x in range(0, w, 48):
        d.line([(x, 0), (x, h)], fill=(120, 150, 180, alpha))
    for y in range(0, h, 48):
        d.line([(0, y), (w, y)], fill=(120, 150, 180, alpha))
    return layer


def hgrad_text(draw, xy, text, font, c0, c1, base_img):
    """Draw text filled with a horizontal c0->c1 gradient (for the accent rule)."""
    bbox = draw.textbbox(xy, text, font=font)
    tw = bbox[2] - bbox[0]
    th = bbox[3] - bbox[1]
    if tw <= 0 or th <= 0:
        return
    grad = Image.new("RGBA", (tw, th), (0, 0, 0, 0))
    gpx = grad.load()
    for x in range(tw):
        c = lerp(c0, c1, x / max(1, tw - 1))
        for y in range(th):
            gpx[x, y] = c + (255,)
    mask = Image.new("L", (tw, th), 0)
    md = ImageDraw.Draw(mask)
    md.text((-bbox[0] + xy[0] - xy[0], -bbox[1] + xy[1] - xy[1]), text, font=font, fill=255)
    base_img.paste(grad, (xy[0], xy[1]), mask)


def render(width, height, with_url):
    ss = 2
    w, h = width * ss, height * ss
    img = gradient_bg(w, h)
    img.alpha_composite(grid_lines(w, h))

    # Spectrum as a "floor" band across the very bottom (text lives above it).
    band_h = int(h * 0.20)
    base_y = int(h * 0.985)
    img.alpha_composite(draw_spectrum(w, h, base_y, band_h, alpha_scale=1.0))
    # A soft scrim over the upper area so text stays readable.
    scrim = Image.new("RGBA", (w, h), (0, 0, 0, 0))
    sd = ImageDraw.Draw(scrim)
    sd.rectangle([0, 0, w, int(h * 0.70)], fill=(8, 12, 20, 80))
    img.alpha_composite(scrim)

    d = ImageDraw.Draw(img)

    word = ImageFont.truetype(BOLD, int(h * 0.20))
    tag = ImageFont.truetype(REG, int(h * 0.052))
    tier = ImageFont.truetype(MONO_B, int(h * 0.040))
    foot = ImageFont.truetype(MONO, int(h * 0.034))

    cx = w // 2
    top = int(h * 0.16)

    # Wordmark "Tempo" centered.
    wb = d.textbbox((0, 0), "Tempo", font=word)
    ww = wb[2] - wb[0]
    wx = cx - ww // 2
    d.text((wx, top), "Tempo", font=word, fill=WHITE)

    # Amber->green accent rule under the wordmark.
    ry = top + (wb[3] - wb[1]) + int(h * 0.045)
    rule_w = int(ww * 0.92)
    rx = cx - rule_w // 2
    rule = Image.new("RGBA", (rule_w, max(3, int(h * 0.012))), (0, 0, 0, 0))
    rpx = rule.load()
    for x in range(rule.width):
        c = lerp(AMBER, GREEN, x / max(1, rule.width - 1))
        for y in range(rule.height):
            rpx[x, y] = c + (255,)
    img.alpha_composite(rule, (rx, ry))

    # Tagline.
    tagline = "Modern, chat-first off-grid ham radio text"
    tb = d.textbbox((0, 0), tagline, font=tag)
    d.text((cx - (tb[2] - tb[0]) // 2, ry + int(h * 0.04)), tagline, font=tag, fill=MUTED)

    # Two-tier line, mono, with colored tier labels.
    ty = ry + int(h * 0.135)
    fast = "FAST  FT1 · 4s coherent"
    sep = "      "
    robust = "ROBUST  DX1 · 15s fading-immune"
    fb = d.textbbox((0, 0), fast, font=tier)
    sb = d.textbbox((0, 0), sep, font=tier)
    rb = d.textbbox((0, 0), robust, font=tier)
    total = (fb[2] - fb[0]) + (sb[2] - sb[0]) + (rb[2] - rb[0])
    sx = cx - total // 2
    d.text((sx, ty), fast, font=tier, fill=AMBER)
    sx += (fb[2] - fb[0]) + (sb[2] - sb[0])
    d.text((sx, ty), robust, font=tier, fill=GREEN)

    # Footer line — sits above the spectrum floor.
    footer = "HF · VHF/UHF  weak-signal text   ·   GPLv3"
    if with_url:
        footer = "github.com/kd9taw/nexus   ·   HF · VHF/UHF weak-signal text · GPLv3"
    fbb = d.textbbox((0, 0), footer, font=foot)
    d.text((cx - (fbb[2] - fbb[0]) // 2, int(h * 0.73)), footer, font=foot, fill=(126, 144, 164))

    return img.resize((width, height), Image.LANCZOS).convert("RGB")


def main():
    render(1280, 640, with_url=False).save(os.path.join(OUT, "banner.png"))
    render(1200, 630, with_url=True).save(os.path.join(OUT, "social-card.png"))
    print(f"banner + social card written to {OUT}")


if __name__ == "__main__":
    main()
