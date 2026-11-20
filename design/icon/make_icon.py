"""Renders every Lorelamp icon from the masters in design/icon/.

Run from anywhere: python design/icon/make_icon.py [--web]
Needs Pillow and Node with the dev dependencies of apps/desktop (`npm ci` there; the Tauri CLI makes the
desktop set and rasterises the small SVG with resvg).

The masters: build_icon.py draws the three PNGs, lorelamp-icon-small.svg is drawn by hand.
- lorelamp-icon.png             the app icon, 1024 px: a flame with an open book in it on a rounded indigo
                                tile, corners transparent
- lorelamp-icon-foreground.png  the flame and the book with their glow, inside Android's 66 dp safe circle
- lorelamp-icon-background.png  the indigo field to the edges
- lorelamp-icon-small.svg       the reduced cut for 16-32 px: bigger flame and book, no glow

Every picture of 32 px or less comes from the small SVG, every larger one from the PNGs.
- Desktop (apps/desktop/src-tauri/icons/): `tauri icon` makes the full set from lorelamp-icon.png; only the
  files that already exist there are replaced. The PNGs of 32 px or less are drawn again from the small SVG,
  icon.ico gets 16, 24 and 32 px from the small SVG and 48, 64 and 256 px from the app icon, and icon.icns
  takes its 16 and 32 px entries (is32, il32, ic11 and the masks) from the small SVG's own `tauri icon` set.
- Android (apps/mobile): the adaptive icon (foreground and background bitmaps of 108 dp per density), the
  legacy mipmaps for Android 7 (square and round) and the start page's logo: the small SVG still
  (ui/lorelamp.svg) and with the flame's flicker (ui/lorelamp-lit.svg; the page picks it by
  prefers-reduced-motion, because a picture inside <img> does not always see that setting).
- Web (apps/web/public): favicon.svg (the small SVG), favicon.ico (16, 32, 48 px), the manifest icons
  (the tile as it is at 192 and 512 px, a full-bleed maskable 512) and Apple's touch icon (full bleed, the
  phone rounds it). `--web` makes only these.
All file icons are still; only the start page's logo moves.
"""
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

from PIL import Image, ImageChops, ImageDraw

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
MASTER = HERE / "lorelamp-icon.png"
FOREGROUND = HERE / "lorelamp-icon-foreground.png"
BACKGROUND = HERE / "lorelamp-icon-background.png"
SMALL = HERE / "lorelamp-icon-small.svg"
ICONS = ROOT / "apps" / "desktop" / "src-tauri" / "icons"
WEB = ROOT / "apps" / "web" / "public"
MOBILE_UI = ROOT / "apps" / "mobile" / "ui"
RES = ROOT / "apps" / "mobile" / "src-tauri" / "gen" / "android" / "app" / "src" / "main" / "res"

SMALL_MAX = 32
ICO = [16, 24, 32, 48, 64, 256]
FAVICON = [16, 32, 48]
# The entries of icon.icns that hold 32 px or less: is32 + s8mk (16 px picture and mask), il32 + l8mk
# (32 px) and ic11 (16 px at 2x, a 32 px PNG).
ICNS_SMALL = {"is32", "s8mk", "il32", "l8mk", "ic11"}
DENSITIES = {"mdpi": 1, "hdpi": 1.5, "xhdpi": 2, "xxhdpi": 3, "xxxhdpi": 4}
LEGACY_DP, ADAPTIVE_DP = 48, 108

# The mark's scale in build_icon.py: 1.05 on the app icon, 0.62 on the adaptive foreground. A full-bleed
# square takes the foreground k times as large: Apple's touch icon at the app icon's size, the maskable icon
# a little smaller, so the flame's tip (23 % of the side from the centre on the foreground) stays well inside
# the circle of 80 % that every mask keeps (1.4 * 23 % = 33 % < 40 %).
APPLE = 1.05 / 0.62
MASKABLE = 1.4
# The round legacy icon: a circle just inside the tile, clear of its lit edge.
ROUND = 0.485

# The flame's flicker (same loop as the BrandMark in apps/web/src/style.css): transform only, the book
# stays still, and nothing moves with reduced motion.
FLICKER = """<style>
    .flick { transform-box: fill-box; transform-origin: 50% 100%; animation: flick 2.4s ease-in-out infinite; }
    @keyframes flick {
      0%, 100% { transform: scale(1, 1) skewX(0deg); }
      17% { transform: scale(0.985, 1.035) skewX(-1.6deg); }
      31% { transform: scale(1.01, 0.985) skewX(0.8deg); }
      48% { transform: scale(0.99, 1.045) skewX(1.8deg); }
      63% { transform: scale(1.012, 0.99) skewX(-0.6deg); }
      81% { transform: scale(0.992, 1.025) skewX(-1.2deg); }
    }
    @media (prefers-reduced-motion: reduce) { .flick { animation: none; } }
  </style>"""


# ---------- pictures ----------

def tauri_icon(src: Path, out: str, sizes=()):
    """`tauri icon` from apps/desktop: the full desktop set, or only PNGs of the given sizes."""
    args = ["npx", "tauri", "icon", str(src), "-o", out]
    for s in sizes:
        args += ["-p", str(s)]
    subprocess.run(args, cwd=ROOT / "apps" / "desktop", check=True, shell=True, capture_output=True)


def small(sizes) -> dict[int, Image.Image]:
    """{size: RGBA image} of the small SVG, drawn by resvg straight from the vector at each size."""
    with tempfile.TemporaryDirectory() as tmp:
        tauri_icon(SMALL, tmp, sizes)
        return {s: Image.open(Path(tmp) / f"{s}x{s}.png").convert("RGBA").copy() for s in sizes}


def shrink(src: Path, size: int) -> Image.Image:
    """A master scaled down (Pillow resamples RGBA in premultiplied alpha: no dark fringe at the corners)."""
    with Image.open(src) as img:
        return img.convert("RGBA").resize((size, size), Image.LANCZOS)


def bleed(k: float, size: int) -> Image.Image:
    """Full-bleed square: the field to the edges, the mark k times as large as on the adaptive foreground."""
    out = shrink(BACKGROUND, size)
    big = round(size * k)
    off = (big - size) // 2
    out.alpha_composite(shrink(FOREGROUND, big).crop((off, off, off + size, off + size)))
    return out.convert("RGB")


def round_icon(size: int) -> Image.Image:
    """The app icon inside a circle (drawn at 4x and scaled down for a soft edge)."""
    img = shrink(MASTER, size)
    mask = Image.new("L", (size * 4, size * 4), 0)
    c, r = size * 2, size * 4 * ROUND
    ImageDraw.Draw(mask).ellipse((c - r, c - r, c + r, c + r), fill=255)
    img.putalpha(ImageChops.multiply(img.getchannel("A"), mask.resize((size, size), Image.LANCZOS)))
    return img


def width(path: Path) -> int:
    with Image.open(path) as img:
        return img.size[0]


def lit(svg: str) -> str:
    """The small SVG with the flame's flicker."""
    if 'id="flame"' not in svg or "</title>" not in svg:
        raise SystemExit("make_icon: lorelamp-icon-small.svg needs a <title> and the flame's id=\"flame\"")
    svg = svg.replace('id="flame"', 'id="flame" class="flick"', 1)
    return svg.replace("</title>", "</title>\n  " + FLICKER, 1)


# ---------- files ----------

def note(path: Path, extra: str = ""):
    print(f"icon  {path.relative_to(ROOT).as_posix()}{extra}")


def save(img: Image.Image, path: Path):
    path.parent.mkdir(parents=True, exist_ok=True)
    img.save(path, optimize=True)
    note(path)


def save_ico(frames: dict[int, Image.Image], path: Path):
    big = frames[max(frames)]
    big.save(path, sizes=[(s, s) for s in frames], append_images=[frames[s] for s in frames if frames[s] is not big])
    note(path, f"  ({', '.join(str(s) for s in frames)} px)")


def write(path: Path, text: str):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8", newline="\n")
    note(path)


def copy(src: Path, path: Path):
    shutil.copyfile(src, path)
    note(path)


# ---------- targets ----------

def icns_read(path: Path) -> dict[str, bytes]:
    """{type: data} of an .icns file (a header, then entries of type, length and data)."""
    raw = path.read_bytes()
    out, i = {}, 8
    while i < len(raw):
        size = int.from_bytes(raw[i + 4:i + 8], "big")
        out[raw[i:i + 4].decode("ascii")] = raw[i + 8:i + size]
        i += size
    return out


def icns_write(entries: dict[str, bytes], path: Path):
    """An .icns file with its entries in a fixed order (`tauri icon` writes them in a random one)."""
    body = b"".join(t.encode("ascii") + (len(d) + 8).to_bytes(4, "big") + d for t, d in sorted(entries.items()))
    path.write_bytes(b"icns" + (len(body) + 8).to_bytes(4, "big") + body)


def desktop():
    with tempfile.TemporaryDirectory() as tmp:
        tauri_icon(MASTER, tmp)
        for old in sorted(ICONS.iterdir()):
            new = Path(tmp) / old.name
            if new.is_file() and old.suffix != ".ico":
                shutil.copyfile(new, old)
                note(old)
    # icon.icns: its 16 and 32 px pictures from the small SVG, the larger ones from the app icon.
    with tempfile.TemporaryDirectory() as tmp:
        tauri_icon(SMALL, tmp)
        drawn_small = icns_read(Path(tmp) / "icon.icns")
    entries = icns_read(ICONS / "icon.icns")
    entries.update({t: drawn_small[t] for t in ICNS_SMALL})
    icns_write(entries, ICONS / "icon.icns")
    note(ICONS / "icon.icns", f"  ({', '.join(sorted(ICNS_SMALL))} from the small SVG)")
    tiny = {p: s for p in sorted(ICONS.glob("*.png")) if (s := width(p)) <= SMALL_MAX}
    drawn = small(sorted({*tiny.values(), *(s for s in ICO if s <= SMALL_MAX)}))
    for path, size in tiny.items():
        save(drawn[size], path)
    save_ico({s: drawn[s] if s <= SMALL_MAX else shrink(MASTER, s) for s in ICO}, ICONS / "icon.ico")


def android():
    for dpi, d in DENSITIES.items():
        folder = RES / f"mipmap-{dpi}"
        save(shrink(FOREGROUND, round(ADAPTIVE_DP * d)), folder / "ic_launcher_foreground.png")
        save(shrink(BACKGROUND, round(ADAPTIVE_DP * d)).convert("RGB"), folder / "ic_launcher_background.png")
        save(shrink(MASTER, round(LEGACY_DP * d)), folder / "ic_launcher.png")
        save(round_icon(round(LEGACY_DP * d)), folder / "ic_launcher_round.png")
    write(RES / "mipmap-anydpi-v26" / "ic_launcher.xml",
          '<?xml version="1.0" encoding="utf-8"?>\n'
          "<!-- Made by design/icon/make_icon.py from design/icon/lorelamp-icon-*.png. -->\n"
          '<adaptive-icon xmlns:android="http://schemas.android.com/apk/res/android">\n'
          '    <background android:drawable="@mipmap/ic_launcher_background" />\n'
          '    <foreground android:drawable="@mipmap/ic_launcher_foreground" />\n'
          "</adaptive-icon>\n")
    copy(SMALL, MOBILE_UI / "lorelamp.svg")
    write(MOBILE_UI / "lorelamp-lit.svg", lit(SMALL.read_text(encoding="utf-8")))


def web():
    copy(SMALL, WEB / "favicon.svg")
    drawn = small([s for s in FAVICON if s <= SMALL_MAX])
    save_ico({s: drawn[s] if s <= SMALL_MAX else shrink(MASTER, s) for s in FAVICON}, WEB / "favicon.ico")
    icons = WEB / "icons"
    save(shrink(MASTER, 192), icons / "icon-192.png")
    save(shrink(MASTER, 512), icons / "icon-512.png")
    save(bleed(MASKABLE, 512), icons / "icon-maskable-512.png")
    save(bleed(APPLE, 180), icons / "apple-touch-icon.png")


def main():
    if "--web" not in sys.argv:
        desktop()
        android()
    web()


if __name__ == "__main__":
    main()
