"""Lorelamp app icon - a flame with an open book in it. Draws the three 1024 px masters.

    python design/icon/build_icon.py            -> the three PNGs next to this script (3x supersampling)
    python design/icon/build_icon.py --draft    -> only lorelamp-icon.png, 2x supersampling (fast look)

Needs Python 3 with numpy, opencv-python and scipy (`pip install numpy opencv-python scipy`); CPU only,
about 1.5 minutes. Writes lorelamp-icon.png (the rounded tile, corners transparent) and, for Android's
adaptive icon, lorelamp-icon-foreground.png (the mark and its glow inside the 66 dp safe circle) and
lorelamp-icon-background.png (the field to the edges). make_icon.py turns these files and the reduced
drawing lorelamp-icon-small.svg into every icon the apps use; run it after this script.

The mark is drawn in a 256-unit design space (the same grid as lorelamp-icon-small.svg). The flame gets a
real volume: a height field (soft pillow from a Poisson solve + a rounded edge from the distance field) is
lit from the top left, on top of a colour ramp that radiates from the book - the book is the light source.
"""
import argparse, pathlib
import numpy as np, cv2
from scipy import ndimage as ndi
from scipy.sparse import csr_matrix
from scipy.sparse.linalg import spsolve

HERE = pathlib.Path(__file__).parent


# ---------- colour helpers (all maths in linear light) ----------
def lin(hexs):
    c = np.array([int(hexs[i:i + 2], 16) for i in (1, 3, 5)], np.float32) / 255
    return np.where(c <= 0.04045, c / 12.92, ((c + 0.055) / 1.055) ** 2.4).astype(np.float32)


def to_srgb(a):
    a = np.clip(a, 0, 1)
    return np.where(a <= 0.0031308, a * 12.92, 1.055 * a ** (1 / 2.4) - 0.055)


def ramp(t, stops):
    t = np.clip(t, 0, 1)
    out = np.zeros(t.shape + (3,), np.float32)
    for (t0, c0), (t1, c1) in zip(stops[:-1], stops[1:]):
        m = (t >= t0) & (t <= t1)
        f = ((t[m] - t0) / (t1 - t0))[:, None]
        f = f * f * (3 - 2 * f) * 0.5 + f * 0.5
        out[m] = lin(c0) * (1 - f) + lin(c1) * f
    return out


def smooth(a, lo, hi):
    f = np.clip((a - lo) / (hi - lo), 0, 1)
    return f * f * (3 - 2 * f)


# ---------- shapes (256-unit design space) ----------
def c3(p0, p1, p2, p3, n=64):
    t = np.linspace(0, 1, n)[:, None]
    p0, p1, p2, p3 = map(np.array, (p0, p1, p2, p3))
    return (1 - t) ** 3 * p0 + 3 * (1 - t) ** 2 * t * p1 + 3 * (1 - t) * t ** 2 * p2 + t ** 3 * p3


def q2(p0, p1, p2, n=16):
    t = np.linspace(0, 1, n)[:, None]
    p0, p1, p2 = map(np.array, (p0, p1, p2))
    return (1 - t) ** 2 * p0 + 2 * (1 - t) * t * p1 + t ** 2 * p2


def flame_pts():
    a = np.linspace(0, np.pi, 160)
    belly = np.stack([128 + 58 * np.cos(a), 158 + 58 * np.sin(a)], 1)
    return np.concatenate([
        c3((148, 32), (148, 70), (186, 100), (186, 158)),
        belly,
        c3((70, 158), (70, 124), (92, 104), (90, 72)),
        c3((90, 72), (96, 92), (108, 104), (118, 108)),
        c3((118, 108), (112, 80), (118, 54), (148, 32)),
    ])


def page_pts(wide=0.0, dy=0.0):
    """Left page of the open book; wide > 0 lets the outline overhang at the outer edge (the cover)."""
    w = wide
    pts = np.concatenate([
        c3((126.8, 160), (116, 151), (102, 147), (91 - w, 149)),
        q2((91 - w, 149), (89 - w, 149.5), (89 - w, 152)),
        np.array([[90.5 - w, 179.5]]),
        q2((90.5 - w, 179.5), (90.6 - w, 182), (93 - w, 182)),
        c3((93 - w, 182), (104, 181.5), (116, 185), (126.8, 192)),
    ])
    pts[:, 1] += dy
    return pts


def mirror(p):
    q = p.copy()
    q[:, 0] = 256 - q[:, 0]
    return q


def page_top_y():
    """y of the left page's top edge as a function of x (the edge curves down into the gutter)."""
    t = c3((126.8, 160), (116, 151), (102, 147), (91, 149), n=200)
    order = np.argsort(t[:, 0])
    return lambda x: np.interp(x, t[order, 0], t[order, 1])


def text_polys(seed, right=False):
    """Words of a few lines of text on the left page (mirrored for the right one): bars of varied length
    with a paragraph break and an indent, following the curve of the page."""
    rng = np.random.default_rng(seed)
    top = page_top_y()
    polys = []
    x0, x1 = 95.0, 122.0                     # outer margin .. gutter margin (design units)
    for k in range(9):
        off = 6.6 + k * 2.3
        start = x0 + (2.4 if k == 5 else 0.0)
        end = x1 - (rng.uniform(8, 13) if k == 4 else rng.uniform(0, 1.0))
        x = start
        while x < end - 1.5:
            w = min(rng.uniform(2.6, 7.5), end - x)
            xs = np.linspace(x, x + w, 8)
            yt = top(xs) + off
            yb = yt + 0.62
            poly = np.concatenate([np.stack([xs, yt], 1), np.stack([xs[::-1], yb[::-1]], 1)])
            polys.append(mirror(poly) if right else poly)
            x += w + rng.uniform(0.55, 0.85)
    return polys


class Canvas:
    def __init__(self, size, scale=1.0, dy=0.0):
        self.S, self.scale, self.dy = size, scale, dy
        self.u = size / 256 * scale  # pixels per design unit of the mark

    def mask(self, polys, mark=True):
        m = np.zeros((self.S, self.S), np.uint8)
        out = []
        for p in polys:
            p = p.copy()
            if mark:
                p = (p - 128) * self.scale + 128
                p[:, 1] += self.dy
            out.append(np.round(p * self.S / 256 * 16).astype(np.int32))
        cv2.fillPoly(m, out, 255, lineType=cv2.LINE_8, shift=4)
        return m > 127


def sdf(m):
    """Signed distance in pixels, negative inside."""
    return (ndi.distance_transform_edt(~m) - ndi.distance_transform_edt(m)).astype(np.float32)


def soften(m, r_convex, r_concave):
    """Round convex corners (tips) by r_convex and concave ones (notch) by r_concave, in pixels."""
    d = sdf(m)
    core = d <= -r_convex
    d2 = ndi.distance_transform_edt(~core)
    m2 = d2 <= r_convex
    grown = ndi.distance_transform_edt(~m2) <= r_concave
    return ndi.distance_transform_edt(grown) > r_concave


def aa(d):
    return np.clip(0.5 - d, 0, 1).astype(np.float32)


def blur(a, sigma):
    return cv2.GaussianBlur(a, (0, 0), sigma, borderType=cv2.BORDER_REPLICATE)


def pillow(m, n=420):
    """Inflated-cushion height (0..1) of a mask: sqrt of the Poisson solution, solved small, scaled up."""
    S = m.shape[0]
    small = cv2.resize(m.astype(np.uint8), (n, n), interpolation=cv2.INTER_AREA) > 0
    idx = -np.ones((n, n), np.int64)
    ys, xs = np.nonzero(small)
    idx[ys, xs] = np.arange(len(ys))
    rows, cols, vals = [], [], []
    for k, (y, x) in enumerate(zip(ys, xs)):
        rows.append(k); cols.append(k); vals.append(4.0)
        for yy, xx in ((y - 1, x), (y + 1, x), (y, x - 1), (y, x + 1)):
            j = idx[yy, xx]
            if j >= 0:
                rows.append(k); cols.append(j); vals.append(-1.0)
    A = csr_matrix((vals, (rows, cols)), shape=(len(ys), len(ys)))
    u = spsolve(A, np.ones(len(ys)))
    h = np.zeros((n, n), np.float32)
    h[ys, xs] = np.sqrt(u / u.max())
    h = cv2.resize(h, (S, S), interpolation=cv2.INTER_CUBIC)
    return np.clip(blur(h, S / n * 1.5), 0, 1)


def over(dst, col, a):
    dst *= (1 - a)[..., None]
    dst += col * a[..., None]


# ---------- the picture ----------
def render(size, layer="full", scale=1.05, dy=0.0):
    """layer: full | foreground (mark + its glow, transparent) | background (square field only)."""
    cv = Canvas(size, scale, dy)
    S, u = size, cv.u
    yy, xx = np.mgrid[0:S, 0:S].astype(np.float32)
    X = ((xx + 0.5) / S * 256 - 128) / scale + 128          # design coordinates of the mark
    Y = ((yy + 0.5) / S * 256 - 128 - dy) / scale + 128
    TY = (yy + 0.5) / S

    # --- field ---
    if layer == "background":
        tile_a = np.ones((S, S), np.float32)
        tile_d = np.full((S, S), -1e6, np.float32)
    else:
        rad = 58.0
        qx, qy = np.abs((xx + 0.5) / S * 256 - 128) - (128 - rad), np.abs((yy + 0.5) / S * 256 - 128) - (128 - rad)
        tile_d = (np.hypot(np.maximum(qx, 0), np.maximum(qy, 0)) + np.minimum(np.maximum(qx, qy), 0) - rad) * S / 256
        tile_a = aa(tile_d)
    img = ramp(TY, [(0, "#2d2856"), (0.55, "#1d1a3a"), (1, "#131027")])
    # soft light from above on the field + the faintest lifted edge, as on a pressed tile
    img *= (1 + 0.10 * smooth(1 - TY, 0.55, 1.0))[..., None]
    if layer != "background":
        edge = np.exp(-((tile_d + 1.6 * S / 256) / (1.3 * S / 256)) ** 2)
        gy = np.gradient(tile_d, axis=0)
        img += (edge * np.clip(-gy, 0, 1) * 0.055)[..., None] * lin("#b9b4ff")
        img *= (1 - edge * np.clip(gy, 0, 1) * 0.35)[..., None]

    if layer == "background":
        return img, tile_a

    # --- flame shape ---
    fm = soften(cv.mask([flame_pts()]), 0.6 * u, 1.4 * u)
    fd = sdf(fm)
    fa = aa(fd)
    inside = np.clip(-fd, 0, None) / u                       # depth inside the flame, design units

    # volume: broad pillow + rounded edge
    R = 9.0
    e = np.clip(inside / R, 0, 1)
    bevel = np.sqrt(1 - (1 - e) ** 2)
    H = (pillow(fm) * 8.0 + blur(bevel, 0.8 * u) * 4.0) * u
    H = blur(H, 2.0 * u)
    gx, gy = np.gradient(H, axis=1), np.gradient(H, axis=0)
    nz = 1 / np.sqrt(gx * gx + gy * gy + 1)
    L = np.array([-0.46, -0.66, 0.59], np.float32); L /= np.linalg.norm(L)
    rel = (-gx * L[0] - gy * L[1] + L[2]) * nz - L[2]        # lighting relative to a flat face

    # colour: light radiates from the open book
    r = np.hypot((X - 128) * 1.12, (Y - 172) * 0.86) / 136
    r += 0.10 * (1 - np.clip(inside / 16, 0, 1)) ** 2          # the body deepens toward its edge
    col = ramp(r, [(0, "#fff3b8"), (0.16, "#ffdc7c"), (0.36, "#ffb347"), (0.60, "#fa8432"),
                   (0.82, "#ea5a29"), (1, "#d2401f")])
    rel = rel * smooth(np.hypot(X - 118, Y - 108), 3, 26)   # no hot spot in the fold between the tongues
    lit, shade = np.clip(rel, 0, None), np.clip(-rel, 0, None)
    col = col * (1 + 0.42 * lit - 0.22 * shade)[..., None]
    col += (smooth(lit, 0.10, 0.42) * 0.13)[..., None] * lin("#fff0c8")
    over(col, lin("#cf3a1c"), np.clip(shade * 0.40, 0, 0.5))

    # --- book ---
    lp, rp = page_pts(), mirror(page_pts())
    pages_m = cv.mask([lp, rp])
    pages_d = sdf(pages_m); pages_a = aa(pages_d)
    # the page stack: a few sheets below the top pages, each a hair lower and wider -> visible thickness
    sheets = [aa(sdf(cv.mask([page_pts(w, dy=d), mirror(page_pts(w, dy=d))])))
              for d, w in ((0.85, 0.12), (1.65, 0.24), (2.45, 0.36), (3.2, 0.46))]
    block_a = sheets[-1]
    spine = np.array([[126.3, 161.2], [129.7, 161.2], [129.7, 197.2], [128, 198.2], [126.3, 197.2]])
    cover_a = aa(sdf(cv.mask([page_pts(0.2, dy=5.0), mirror(page_pts(0.2, dy=5.0)), spine])))
    text_a = aa(sdf(cv.mask(text_polys(11) + text_polys(29, right=True)))) * pages_a

    # --- glow of the mark onto the field ---
    mark_rgb = col * fa[..., None]
    glow = blur(mark_rgb, 17 * u) * 0.30 + blur(mark_rgb, 5 * u) * 0.26
    glow *= lin("#ffad5c")[None, None]
    halo_a = np.clip(blur(fa, 17 * u) * 0.55 + blur(fa, 5 * u) * 0.35, 0, 1)

    if layer == "foreground":
        img = np.zeros((S, S, 3), np.float32)
        alpha = np.clip(halo_a * (1 - fa) * 0.85, 0, 1)
        img += lin("#ff8a3c")[None, None]
    else:
        img *= (1 - 0.22 * halo_a * (1 - fa))[..., None]
        img += glow * (1 - fa)[..., None]
        alpha = tile_a

    # flame on top, a whisper of occlusion where it meets the field
    over(img, col, fa)
    alpha = np.maximum(alpha, fa) if layer == "foreground" else alpha

    # light the book throws onto the flame around it
    bg = blur(pages_a, 10 * u) * 0.34 + blur(pages_a, 3.5 * u) * 0.16
    img += (bg * fa * (1 - cover_a))[..., None] * lin("#fff1c4")

    # soft contact shade under the book so it sits in the flame instead of floating on it
    sh = blur(np.roll(cover_a, int(2.2 * u), axis=0), 2.6 * u)
    over(img, lin("#c2441c"), sh * fa * (1 - cover_a) * 0.42)

    cov = ramp((Y - 152) / 50, [(0, "#c5501f"), (1, "#8e3214")])
    over(img, cov, cover_a)
    # page stack: lowest sheet darkest, each sheet above a little lighter -> fine page edges
    for sa, tone in zip(reversed(sheets), ("#cfb07a", "#dfc597", "#ecdab6", "#f6ead0")):
        over(img, lin(tone), sa)
    # pages: the brightest thing in the picture; they bend into the gutter (soft crease shadow)
    gut = smooth(np.abs(X - 128), 3, 34)
    pg = lin("#f3ddb3") * (1 - gut[..., None]) + lin("#fffdf6") * gut[..., None]
    pg = pg * (1 + 0.05 * smooth(-(Y - 170), -12, 22))[..., None]
    crease = np.exp(-np.abs(X - 128) / 2.6)
    pg = pg * (1 - 0.20 * crease)[..., None]
    over(img, pg, pages_a)
    # text: soft warm ink, light enough that the pages still glow
    over(img, lin("#a88a5e"), text_a * 0.32)
    # bloom: the pages shine past their own edge
    bloom = (blur(pages_a, 2.4 * u) * 0.22 + blur(pages_a, 7 * u) * 0.10) * (1 - np.maximum(pages_a, cover_a))
    img += (bloom * (fa if layer == "foreground" else 1))[..., None] * lin("#fff4d2")

    return img, alpha


def save(img, alpha, out, final):
    S = img.shape[0]
    pm = np.dstack([img * alpha[..., None], alpha])
    pm = cv2.resize(pm, (final, final), interpolation=cv2.INTER_AREA) if S != final else pm
    a = pm[..., 3]
    rgb = to_srgb(pm[..., :3] / np.maximum(a, 1e-6)[..., None])
    rng = np.random.default_rng(7)
    rgb = rgb * 255 + (rng.random(rgb.shape) - rng.random(rgb.shape)) * 0.5   # dither against banding
    px = np.dstack([np.clip(rgb, 0, 255), a * 255]).round().astype(np.uint8)
    cv2.imwrite(str(out), cv2.cvtColor(px, cv2.COLOR_RGBA2BGRA))
    print(out)


if __name__ == "__main__":
    ap = argparse.ArgumentParser()
    ap.add_argument("--draft", action="store_true")
    ap.add_argument("--out", default=str(HERE))
    a = ap.parse_args()
    out = pathlib.Path(a.out); out.mkdir(parents=True, exist_ok=True)
    ss = 2 if a.draft else 3
    save(*render(1024 * ss), out / "lorelamp-icon.png", 1024)
    if not a.draft:
        # Android adaptive icon: 108dp canvas, the mark has to live inside the inner 66dp circle
        save(*render(1024 * ss, "foreground", scale=0.62, dy=1), out / "lorelamp-icon-foreground.png", 1024)
        save(*render(1024 * ss, "background"), out / "lorelamp-icon-background.png", 1024)
