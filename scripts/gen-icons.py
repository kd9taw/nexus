#!/usr/bin/env python3
"""Generate Tempo's app icon set into src-tauri/icons/.

Draws a simple, on-theme mark — a dark rounded square with a stylized
spectrum/waterfall "equalizer" in an amber→green sweep (signal + tempo) — and
emits every size tauri.conf.json references (PNG window icons, a multi-size
Windows .ico, and a macOS .icns). Re-run after changing the design:

    python3 scripts/gen-icons.py

Requires Pillow (PIL). On Windows the build script falls back to
`cargo tauri icon` if this can't run.
"""
import os
from PIL import Image, ImageDraw

OUT = os.path.join(os.path.dirname(__file__), "..", "src-tauri", "icons")
OUT = os.path.normpath(OUT)
os.makedirs(OUT, exist_ok=True)

BG = (14, 21, 31, 255)        # deep slate (matches the dark theme)
BARS = [0.34, 0.58, 0.88, 0.50, 0.40]   # relative heights of the 5 bars


def lerp(a, b, t):
    return tuple(int(a[i] * (1 - t) + b[i] * t) for i in range(len(a)))


def render(size: int) -> Image.Image:
    # Supersample for clean edges, then downscale.
    ss = 4
    s = size * ss
    img = Image.new("RGBA", (s, s), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    d.rounded_rectangle([0, 0, s - 1, s - 1], radius=int(s * 0.22), fill=BG)

    amber, green = (255, 176, 0, 255), (60, 200, 140, 255)
    n = len(BARS)
    margin = s * 0.21
    avail = s - 2 * margin
    bw = avail / (n * 2 - 1)          # bar + equal gap
    base_y = s * 0.76
    for i, h in enumerate(BARS):
        x0 = margin + i * 2 * bw
        bh = avail * h
        col = lerp(amber, green, i / (n - 1))
        d.rounded_rectangle(
            [x0, base_y - bh, x0 + bw, base_y],
            radius=max(2, int(bw * 0.42)),
            fill=col,
        )
    return img.resize((size, size), Image.LANCZOS)


def main():
    master = render(1024)
    master.save(os.path.join(OUT, "icon-source.png"))
    for px, name in [(32, "32x32.png"), (128, "128x128.png"), (256, "128x128@2x.png")]:
        render(px).save(os.path.join(OUT, name))

    ico_sizes = [16, 24, 32, 48, 64, 128, 256]
    render(256).save(
        os.path.join(OUT, "icon.ico"),
        format="ICO",
        sizes=[(p, p) for p in ico_sizes],
    )

    try:
        render(1024).save(os.path.join(OUT, "icon.icns"), format="ICNS")
    except Exception as e:  # noqa: BLE001 - icns is best-effort (macOS only)
        print(f"  note: could not write icon.icns ({e}); macOS bundle only.")

    print(f"icons written to {OUT}")


if __name__ == "__main__":
    main()
