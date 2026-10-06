"""Build the three 1920x480 standby screens (assets/standby/page_N.png) from the
Codex-generated art (art_N.png). The dashboard embeds the pages and shows one at random
when Windows shuts down, restarts or logs off."""
import os
import random
from PIL import Image, ImageDraw, ImageFilter, ImageFont

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
os.chdir(os.path.join(ROOT, "assets", "standby"))
W, H = 1920, 480
FONTS = os.path.join(ROOT, "fonts")


def background():
    # same diagonal night gradient as the dashboard
    c0, c1 = (20, 30, 58), (5, 7, 13)
    bg = Image.new("RGB", (W, H))
    px = bg.load()
    for y in range(H):
        for x in range(W):
            m = (0.4 * y / (H - 1) * 255 + 0.6 * (255 - x / (W - 1) * 255)) / 255
            px[x, y] = tuple(int(c1[k] * m + c0[k] * (1 - m)) for k in range(3))
    d = ImageDraw.Draw(bg)
    random.seed(7)
    for _ in range(70):
        x, y = random.uniform(0, W), random.uniform(0, H)
        r = random.choice([0.8, 1.0, 1.3])
        a = random.uniform(0.25, 0.75)
        c = tuple(int(v * a + 10 * (1 - a)) for v in (223, 230, 255))
        d.ellipse((x - r, y - r, x + r, y + r), fill=c)
    return bg


BG = background()


def compose(art_path, accent, sub, out):
    page = BG.copy()
    art = Image.open(art_path).convert("RGB").resize((480, 480), Image.LANCZOS)
    # soft-edged mask so the art's dark backdrop melts into the gradient
    mask = Image.new("L", (480, 480), 0)
    ImageDraw.Draw(mask).rounded_rectangle((36, 10, 444, 470), radius=120, fill=255)
    mask = mask.filter(ImageFilter.GaussianBlur(28))
    ax = 470
    page.paste(art, (ax, 0), mask)
    d = ImageDraw.Draw(page)
    f_big = ImageFont.truetype(f"{FONTS}/HarmonyOS_Sans_Thin.ttf", 150)
    f_sub = ImageFont.truetype(f"{FONTS}/NotoSansKR-VF.ttf", 30)
    try:
        f_sub.set_variation_by_axes([300])
    except Exception:
        pass
    tx = ax + 520
    x = tx
    for ch in "Stand By":
        d.text((x, 118), ch, font=f_big, fill=(232, 238, 252))
        x += d.textlength(ch, font=f_big) + 6
    d.rounded_rectangle((tx + 6, 318, tx + 70, 322), radius=2, fill=accent)
    d.text((tx + 92, 300), sub, font=f_sub, fill=(138, 151, 184))
    page.save(out)


for i, accent, sub in [
    (1, (157, 140, 255), "잘 자요 · 곧 다시 만나요"),
    (2, (255, 143, 181), "다녀올게요!"),
    (3, (127, 178, 255), "잠시 쉬는 중"),
]:
    compose(f"art_{i}.png", accent, sub, f"page_{i}.png")

