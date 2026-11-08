"""Render the extension icons and the Chrome Web Store art.

- extension/static/icons/icon-{16,32,48}.png: the mark, full size (toolbar and pages).
- extension/static/icons/icon-128.png and store/assets/icon-128.png: the mark at 96x96 inside 128x128 with
  16 px of transparent padding, as the Web Store asks of the store icon (store/README.md).
- store/assets/promo-small-440x280.png and store/assets/promo-marquee-1400x560.png: the promo tiles. No
  text (the store asks for none, and tiles cannot be localized): a page with text lines turns into a JSON
  record with the mark between them.

The mark is a rounded square in the accent colour with `{}` in white. Drawn large and downsampled so small
sizes stay crisp. Run: uv run --with pillow tools/icons/make_icons.py
"""

from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter, ImageFont

ACCENT = (59, 91, 219, 255)
DEEP = (36, 56, 150)
FONT = "/usr/share/fonts/truetype/dejavu/DejaVuSansMono-Bold.ttf"
ROOT = Path(__file__).resolve().parents[2]
ICONS = ROOT / "extension" / "static" / "icons"
STORE = ROOT / "store" / "assets"


def master(size: int = 512) -> Image.Image:
    img = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    pad = size // 32
    d.rounded_rectangle((pad, pad, size - pad, size - pad), radius=size // 5, fill=ACCENT)
    font = ImageFont.truetype(FONT, int(size * 0.58))
    text = "{}"
    box = d.textbbox((0, 0), text, font=font)
    w, h = box[2] - box[0], box[3] - box[1]
    d.text(((size - w) / 2 - box[0], (size - h) / 2 - box[1]), text, font=font, fill=(255, 255, 255, 255))
    return img


def padded_128(big: Image.Image) -> Image.Image:
    """96x96 artwork centred in 128x128 (16 px transparent padding per side)."""
    out = Image.new("RGBA", (128, 128), (0, 0, 0, 0))
    out.paste(big.resize((96, 96), Image.LANCZOS), (16, 16))
    return out


def promo(width: int, height: int, big: Image.Image) -> Image.Image:
    """A page (text lines) -> the mark -> a JSON record (keys and values), on the accent colour. Full bleed."""
    s = 4  # drawn at 4x, downsampled
    W, H = width * s, height * s
    img = Image.new("RGB", (W, H), ACCENT[:3])
    d = ImageDraw.Draw(img)
    # A soft diagonal gradient towards the deeper blue.
    for y in range(H):
        t = y / H
        c = tuple(int(ACCENT[i] * (1 - t * 0.45) + DEEP[i] * t * 0.45) for i in range(3))
        d.line([(0, y), (W, y)], fill=c)

    unit = H / 280  # layout in 440x280 units, scaled to the tile's height
    card_w, card_h = 150 * unit, 190 * unit
    cy = H / 2
    mark = 74 * unit
    gap = (W - 2 * card_w - mark) / 4
    left_x = gap
    mark_x = left_x + card_w + gap
    right_x = mark_x + mark + gap
    top = cy - card_h / 2

    def card(x: float) -> None:
        shadow = Image.new("RGBA", (W, H), (0, 0, 0, 0))
        ImageDraw.Draw(shadow).rounded_rectangle((x + 6 * unit, top + 8 * unit, x + card_w + 6 * unit, top + card_h + 8 * unit), radius=14 * unit, fill=(10, 20, 70, 90))
        img.paste(shadow.filter(ImageFilter.GaussianBlur(8 * unit)), (0, 0), shadow.filter(ImageFilter.GaussianBlur(8 * unit)))
        d.rounded_rectangle((x, top, x + card_w, top + card_h), radius=14 * unit, fill=(255, 255, 255))

    # The page: a header bar, an image block, and lines of text.
    card(left_x)
    px = left_x + 16 * unit
    d.rounded_rectangle((px, top + 16 * unit, left_x + card_w - 16 * unit, top + 30 * unit), radius=4 * unit, fill=(210, 216, 235))
    d.rounded_rectangle((px, top + 40 * unit, left_x + card_w - 16 * unit, top + 84 * unit), radius=6 * unit, fill=(226, 231, 246))
    for i, frac in enumerate([1.0, 0.92, 0.97, 0.7, 0.88, 0.55]):
        y = top + (98 + i * 14) * unit
        d.rounded_rectangle((px, y, px + (card_w - 32 * unit) * frac, y + 6 * unit), radius=3 * unit, fill=(200, 205, 220))

    # The record: braces, then key/value rows (keys muted, values in colours like the popup's JSON panel).
    card(right_x)
    rx = right_x + 16 * unit
    brace = (59, 91, 219)
    d.rounded_rectangle((rx, top + 16 * unit, rx + 10 * unit, top + 24 * unit), radius=3 * unit, fill=brace)
    values = [(32, 140, 90), (180, 90, 30), (32, 140, 90), (120, 70, 190), (32, 140, 90), (180, 90, 30)]
    for i, col in enumerate(values):
        y = top + (36 + i * 22) * unit
        d.rounded_rectangle((rx + 12 * unit, y, rx + 46 * unit, y + 8 * unit), radius=3 * unit, fill=(170, 180, 205))
        vw = [62, 40, 70, 34, 56, 46][i]
        d.rounded_rectangle((rx + 54 * unit, y, rx + (54 + vw) * unit, y + 8 * unit), radius=3 * unit, fill=col)
    d.rounded_rectangle((rx, top + card_h - 24 * unit, rx + 10 * unit, top + card_h - 16 * unit), radius=3 * unit, fill=brace)

    # The mark in the middle, on a white halo so it reads on the blue.
    halo = mark + 14 * unit
    d.ellipse((mark_x + mark / 2 - halo / 2, cy - halo / 2, mark_x + mark / 2 + halo / 2, cy + halo / 2), fill=(255, 255, 255))
    m = big.resize((int(mark), int(mark)), Image.LANCZOS)
    img.paste(m, (int(mark_x), int(cy - mark / 2)), m)
    return img.resize((width, height), Image.LANCZOS)


def main() -> None:
    ICONS.mkdir(parents=True, exist_ok=True)
    STORE.mkdir(parents=True, exist_ok=True)
    big = master()
    for s in (16, 32, 48):
        big.resize((s, s), Image.LANCZOS).save(ICONS / f"icon-{s}.png", optimize=True)
        print(ICONS / f"icon-{s}.png")
    icon = padded_128(big)
    for path in (ICONS / "icon-128.png", STORE / "icon-128.png"):
        icon.save(path, optimize=True)
        print(path)
    for w, h, name in ((440, 280, "promo-small-440x280.png"), (1400, 560, "promo-marquee-1400x560.png")):
        promo(w, h, big).save(STORE / name, optimize=True)
        print(STORE / name)


if __name__ == "__main__":
    main()
