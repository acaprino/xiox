"""Cut the first X out of the Xiox logo and write the Tauri icon set.

Usage: python make_icon.py <xiox-logo.png> <icons-dir>

The logo (2172x724 RGBA: block letters X I O X, an underline bar below) is
cropped to the first letter only:
  * rows 141..534 are the letter row; rows 559..608 are the underline bar,
    separated by a fully transparent band, so the bar falls outside the crop;
  * the first X ends where the I begins: the two outlines meet at one point
    (column 672 holds only the X's tips, the I outline starts at 674), so the
    crop stops at column 672.
The X is centred on a transparent square canvas (1.12 x its longest side) and
the PNGs and the multi-size .ico are written into <icons-dir>.
"""
import sys
from pathlib import Path

from PIL import Image

X_LEFT, X_RIGHT = 51, 672      # inclusive columns of the first X
ROW_TOP, ROW_BOTTOM = 141, 534  # inclusive rows of the letter row
MARGIN = 1.12
ICO_SIZES = [(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)]


def first_x(logo: Image.Image) -> Image.Image:
    box = (X_LEFT, ROW_TOP, X_RIGHT + 1, ROW_BOTTOM + 1)
    x = logo.convert("RGBA").crop(box)
    bbox = x.getchannel("A").getbbox()
    if bbox is None:
        raise SystemExit("the crop of the first X is empty: wrong logo?")
    return x.crop(bbox)


def square(x: Image.Image) -> Image.Image:
    side = round(max(x.size) * MARGIN)
    canvas = Image.new("RGBA", (side, side), (0, 0, 0, 0))
    canvas.alpha_composite(x, ((side - x.width) // 2, (side - x.height) // 2))
    return canvas


def main() -> None:
    logo_path, out = Path(sys.argv[1]), Path(sys.argv[2])
    out.mkdir(parents=True, exist_ok=True)
    big = square(first_x(Image.open(logo_path)))
    for name, size in [("icon.png", 512), ("32x32.png", 32), ("128x128.png", 128), ("128x128@2x.png", 256)]:
        big.resize((size, size), Image.LANCZOS).save(out / name)
    big.resize((256, 256), Image.LANCZOS).save(out / "icon.ico", sizes=ICO_SIZES)


if __name__ == "__main__":
    main()
