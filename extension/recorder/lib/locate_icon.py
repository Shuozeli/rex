"""Find the extension's toolbar icon on a screenshot by template matching.

The icon (icon-16.png or icon-32.png, alpha) is composited over the toolbar background sampled from the
screenshot, then matched by mean absolute difference over the toolbar strip. Prints `x y score` (the icon's
center; score 0 = identical) as one line.

Run: python3 locate_icon.py screen.png icon.png [--top 30 --bottom 100]
"""

import argparse

import numpy as np
from PIL import Image


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("screen")
    ap.add_argument("icon")
    ap.add_argument("--top", type=int, default=30)
    ap.add_argument("--bottom", type=int, default=100)
    ap.add_argument("--size", type=int, default=16, help="icon size on the toolbar, in screen pixels")
    a = ap.parse_args()

    screen = np.asarray(Image.open(a.screen).convert("RGB"), dtype=np.float32)
    strip = screen[a.top : a.bottom]
    icon = Image.open(a.icon).convert("RGBA").resize((a.size, a.size), Image.LANCZOS)
    rgba = np.asarray(icon, dtype=np.float32)
    alpha = rgba[..., 3:4] / 255.0
    # Toolbar background: the most common color of the strip.
    flat = strip.reshape(-1, 3).astype(np.int32)
    keys, counts = np.unique(flat[:, 0] * 65536 + flat[:, 1] * 256 + flat[:, 2], return_counts=True)
    k = int(keys[counts.argmax()])
    bg = np.array([k // 65536, (k // 256) % 256, k % 256], dtype=np.float32)
    tmpl = rgba[..., :3] * alpha + bg * (1 - alpha)

    s = a.size
    windows = np.lib.stride_tricks.sliding_window_view(strip, (s, s, 3))[..., 0, :, :, :]
    diff = np.abs(windows - tmpl).mean(axis=(2, 3, 4))
    y, x = np.unravel_index(diff.argmin(), diff.shape)
    print(int(x + s // 2), int(y + a.top + s // 2), round(float(diff[y, x]), 2))


if __name__ == "__main__":
    main()
