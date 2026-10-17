"""Draws the File Sorter app icon (run once; the results are committed).

    python assets/make_icon.py  ->  assets/icon.ico, assets/icon.png, assets/icon-64.rgba
"""
from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter

HERE = Path(__file__).resolve().parent
S = 1024  # draw large, then scale down for smooth edges


def lerp(a, b, t):
    return tuple(round(x + (y - x) * t) for x, y in zip(a, b))


def draw() -> Image.Image:
    img = Image.new("RGBA", (S, S), (0, 0, 0, 0))

    # Rounded tile with a teal -> blue diagonal gradient.
    grad = Image.new("RGBA", (S, S))
    top, bottom = (45, 212, 191), (37, 99, 235)
    px = grad.load()
    for y in range(S):
        for x in range(S):
            px[x, y] = lerp(top, bottom, (x + y) / (2 * S)) + (255,)
    mask = Image.new("L", (S, S), 0)
    ImageDraw.Draw(mask).rounded_rectangle((40, 40, S - 40, S - 40), radius=220, fill=255)
    img.paste(grad, (0, 0), mask)

    # Soft shadow under the folder.
    shadow = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    sd = ImageDraw.Draw(shadow)
    sd.rounded_rectangle((210, 380, 830, 820), radius=70, fill=(0, 0, 0, 90))
    img.alpha_composite(shadow.filter(ImageFilter.GaussianBlur(28)))

    d = ImageDraw.Draw(img)
    white = (255, 255, 255, 255)
    # Folder: tab + body.
    d.rounded_rectangle((200, 300, 500, 420), radius=50, fill=white)
    d.rounded_rectangle((200, 360, 824, 790), radius=70, fill=white)

    # Arrow dropping into the folder (sorting), in the gradient's blue.
    arrow = (37, 99, 235, 255)
    cx = 512
    d.rounded_rectangle((cx - 48, 430, cx + 48, 640), radius=40, fill=arrow)
    d.polygon([(cx - 150, 590), (cx + 150, 590), (cx, 735)], fill=arrow)
    return img


def main() -> None:
    big = draw()
    sizes = [16, 20, 24, 32, 40, 48, 64, 128, 256]
    big.resize((256, 256), Image.LANCZOS).save(HERE / "icon.ico", sizes=[(s, s) for s in sizes])
    big.resize((256, 256), Image.LANCZOS).save(HERE / "icon.png")
    # Raw RGBA for the window icon (no image decoder needed at runtime).
    (HERE / "icon-64.rgba").write_bytes(big.resize((64, 64), Image.LANCZOS).tobytes())


if __name__ == "__main__":
    main()
