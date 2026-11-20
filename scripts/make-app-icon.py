"""生成 ArtsPhoto 应用图标（分层前景/背景 + AGC 素材）。"""

from PIL import Image, ImageDraw, ImageFilter
import numpy as np
import os
import shutil

FINAL = 1024
SS = 2
N = FINAL * SS
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def lerp(a, b, t):
    return tuple(round(a[i] + (b[i] - a[i]) * t) for i in range(3))


def rounded(size, radius, fill):
    im = Image.new('RGBA', size, (0, 0, 0, 0))
    d = ImageDraw.Draw(im)
    d.rounded_rectangle((0, 0, size[0] - 1, size[1] - 1), radius=radius, fill=fill)
    return im


def photo_card(size, radius, border):
    w, h = size
    sky = Image.new('RGB', (w, h))
    d = ImageDraw.Draw(sky)
    for y in range(h):
        t = y / (h - 1)
        if t < 0.58:
            c = lerp((10, 15, 38), (86, 52, 116), t / 0.58)
        else:
            c = lerp((86, 52, 116), (249, 126, 45), (t - 0.58) / 0.42)
        d.line((0, y, w, y), fill=c)

    sr = int(w * 0.17)
    cx, cy = int(w * 0.63), int(h * 0.37)
    d.ellipse((cx - sr, cy - sr, cx + sr, cy + sr), fill=(255, 210, 104))

    d.polygon([(0, h), (0, int(h * 0.70)), (int(w * 0.17), int(h * 0.52)),
               (int(w * 0.33), int(h * 0.77)), (int(w * 0.50), int(h * 0.48)),
               (int(w * 0.68), int(h * 0.79)), (int(w * 0.84), int(h * 0.60)),
               (w, int(h * 0.74)), (w, h)], fill=(34, 40, 70))
    d.polygon([(0, h), (int(w * 0.10), int(h * 0.88)), (int(w * 0.28), int(h * 0.70)),
               (int(w * 0.48), int(h * 0.94)), (int(w * 0.70), int(h * 0.68)),
               (int(w * 0.90), int(h * 0.90)), (w, h)], fill=(13, 17, 32))

    mask = rounded((w, h), radius, (255, 255, 255, 255))
    card = Image.new('RGBA', (w, h), (0, 0, 0, 0))
    card.paste(sky, (0, 0), mask)
    d = ImageDraw.Draw(card)
    d.rounded_rectangle((0, 0, w - 1, h - 1), radius=radius, outline=border, width=30)
    return card


def make_brush():
    b = Image.new('RGBA', (190, 790), (0, 0, 0, 0))
    d = ImageDraw.Draw(b)
    d.rounded_rectangle((55, 165, 135, 620), radius=28, fill=(245, 250, 255, 255))
    d.rounded_rectangle((55, 165, 135, 620), radius=28, outline=(186, 214, 255, 255), width=8)
    d.polygon([(55, 610), (135, 610), (95, 745)], fill=(34, 211, 238, 255))
    d.polygon([(88, 610), (102, 610), (95, 700)], fill=(14, 116, 144, 255))
    d.ellipse((55, 120, 135, 200), fill=(203, 213, 225, 255))
    d.ellipse((76, 141, 114, 179), fill=(148, 163, 184, 255))
    return b.rotate(34, resample=Image.BICUBIC, expand=True)


def make_background():
    yy, xx = np.mgrid[0:FINAL, 0:FINAL].astype(np.float32)
    cx, cy = FINAL * 0.50, FINAL * 0.40
    dist = np.sqrt((xx - cx) ** 2 + (yy - cy) ** 2) / (FINAL * 0.70)
    t = np.clip(dist, 0, 1)[..., None]
    c0 = np.array([38, 92, 235], dtype=np.float32)
    c1 = np.array([6, 9, 20], dtype=np.float32)
    rgb = (c0 * (1 - t) + c1 * t).astype(np.uint8)
    bg = Image.fromarray(rgb, 'RGB')

    glow = Image.new('RGBA', (FINAL, FINAL), (0, 0, 0, 0))
    gd = ImageDraw.Draw(glow)
    for r, a in [(560, 26), (430, 30), (300, 34)]:
        gd.ellipse((cx - r, cy - r * 0.85, cx + r, cy + r * 0.85), fill=(56, 189, 248, a))
    glow = glow.filter(ImageFilter.GaussianBlur(70))
    bg = Image.alpha_composite(bg.convert('RGBA'), glow).convert('RGB')
    return bg


def make_foreground():
    fg = Image.new('RGBA', (N, N), (0, 0, 0, 0))

    back = rounded((760, 540), 78, (37, 99, 235, 235))
    mid = rounded((740, 540), 78, (14, 165, 233, 235))
    front = photo_card((720, 520), 64, (241, 245, 249, 255))

    back = back.rotate(-13, resample=Image.BICUBIC, expand=True)
    mid = mid.rotate(9, resample=Image.BICUBIC, expand=True)
    front = front.rotate(-4, resample=Image.BICUBIC, expand=True)

    fg.alpha_composite(back, (420, 610))
    fg.alpha_composite(mid, (500, 730))
    fg.alpha_composite(front, (470, 900))
    fg.alpha_composite(make_brush(), (1230, 1040))

    bbox = fg.getbbox()
    content = fg.crop(bbox)
    target = int(FINAL * 0.68)
    scale = target / max(content.size)
    content = content.resize((max(1, round(content.size[0] * scale)), max(1, round(content.size[1] * scale))),
                             Image.LANCZOS)
    out = Image.new('RGBA', (FINAL, FINAL), (0, 0, 0, 0))
    out.alpha_composite(content, ((FINAL - content.size[0]) // 2, (FINAL - content.size[1]) // 2 - 6))
    return out


def main():
    bg = make_background()
    fg = make_foreground()
    icon = Image.alpha_composite(bg.convert('RGBA'), fg).convert('RGB')

    media = os.path.join(ROOT, 'AppScope', 'resources', 'base', 'media')
    entry_media = os.path.join(ROOT, 'entry', 'src', 'main', 'resources', 'base', 'media')
    launch = os.path.join(ROOT, 'docs', 'launch')
    os.makedirs(launch, exist_ok=True)

    bg.save(os.path.join(media, 'background.png'), optimize=True)
    fg.save(os.path.join(media, 'foreground.png'), optimize=True)
    icon.save(os.path.join(media, 'app_icon.png'), optimize=True)

    shutil.copyfile(os.path.join(media, 'background.png'), os.path.join(entry_media, 'background.png'))
    shutil.copyfile(os.path.join(media, 'foreground.png'), os.path.join(entry_media, 'foreground.png'))

    icon.save(os.path.join(launch, 'app-icon-1024.png'), optimize=True)
    icon.resize((216, 216), Image.LANCZOS).save(os.path.join(launch, 'app-icon-216.png'), optimize=True)

    preview = os.path.join(ROOT, 'verify-artifacts', 'icon-preview')
    os.makedirs(preview, exist_ok=True)
    icon.save(os.path.join(preview, 'icon-1024.png'))
    icon.resize((216, 216), Image.LANCZOS).save(os.path.join(preview, 'icon-216.png'))
    fg.save(os.path.join(preview, 'foreground.png'))
    bg.save(os.path.join(preview, 'background.png'))

    alpha = fg.getchannel('A')
    bbox = alpha.getbbox()
    print('icon size', icon.size, 'foreground bbox', bbox,
          'ratio %.1fx%.1f%%' % ((bbox[2] - bbox[0]) / FINAL * 100, (bbox[3] - bbox[1]) / FINAL * 100))
    print('background alpha min/max', bg.getchannel('A').getextrema() if bg.mode == 'RGBA' else (255, 255))


if __name__ == '__main__':
    main()