"""Generate the launcher icon set (PNG sizes + multi-size ICO) with Pillow.

Usage: python tools/make_icons.py
Writes into src-tauri/icons/. Pure vector-ish drawing so it can be re-run any time.
"""
from pathlib import Path

from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "src-tauri" / "icons"
OUT.mkdir(parents=True, exist_ok=True)

SIZE = 1024
S = 4  # supersample factor for smooth edges
W = SIZE * S


def lerp(a, b, t):
    return tuple(int(a[i] + (b[i] - a[i]) * t) for i in range(3))


def build():
    img = Image.new("RGBA", (W, W), (0, 0, 0, 0))
    draw = ImageDraw.Draw(img)

    # Background: rounded square with a vertical plum gradient.
    top = (74, 48, 104)
    bottom = (30, 20, 46)
    grad = Image.new("RGBA", (W, W), (0, 0, 0, 0))
    gdraw = ImageDraw.Draw(grad)
    for y in range(W):
        gdraw.line([(0, y), (W, y)], fill=lerp(top, bottom, y / W) + (255,))
    mask = Image.new("L", (W, W), 0)
    ImageDraw.Draw(mask).rounded_rectangle([0, 0, W - 1, W - 1], radius=int(W * 0.22), fill=255)
    img.paste(grad, (0, 0), mask)

    # Speech bubble body.
    bx0, by0, bx1, by1 = W * 0.17, W * 0.22, W * 0.83, W * 0.68
    draw.rounded_rectangle([bx0, by0, bx1, by1], radius=int(W * 0.13), fill=(248, 244, 255, 255))
    # Bubble tail.
    draw.polygon(
        [(W * 0.30, by1 - W * 0.02), (W * 0.26, W * 0.82), (W * 0.46, by1 - W * 0.02)],
        fill=(248, 244, 255, 255),
    )

    # Three amber dots.
    amber = (240, 160, 75, 255)
    r = W * 0.055
    cy = (by0 + by1) / 2
    for cx in (W * 0.35, W * 0.50, W * 0.65):
        draw.ellipse([cx - r, cy - r, cx + r, cy + r], fill=amber)

    return img.resize((SIZE, SIZE), Image.LANCZOS)


def main():
    icon = build()
    icon.save(OUT / "icon.png")
    icon.resize((128, 128), Image.LANCZOS).save(OUT / "128x128.png")
    icon.resize((256, 256), Image.LANCZOS).save(OUT / "128x128@2x.png")
    icon.resize((32, 32), Image.LANCZOS).save(OUT / "32x32.png")
    icon.save(
        OUT / "icon.ico",
        sizes=[(256, 256), (128, 128), (64, 64), (48, 48), (32, 32), (16, 16)],
    )
    for p in sorted(OUT.iterdir()):
        print(f"{p.name}: {p.stat().st_size} bytes")


if __name__ == "__main__":
    main()
