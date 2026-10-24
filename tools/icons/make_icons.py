"""Render the extension icons (extension/static/icons/icon-{16,32,48,128}.png).

The mark is a rounded square in the accent colour with `{}` in white. Drawn at 512 px and downsampled
so small sizes stay crisp. Run: uv run --with pillow tools/icons/make_icons.py
"""

from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

ACCENT = (59, 91, 219, 255)
FONT = "/usr/share/fonts/truetype/dejavu/DejaVuSansMono-Bold.ttf"
OUT = Path(__file__).resolve().parents[2] / "extension" / "static" / "icons"


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


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    big = master()
    for s in (16, 32, 48, 128):
        big.resize((s, s), Image.LANCZOS).save(OUT / f"icon-{s}.png", optimize=True)
        print(OUT / f"icon-{s}.png")


if __name__ == "__main__":
    main()
