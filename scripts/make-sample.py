from PIL import Image, ImageDraw, ImageFilter

W, H = 2400, 1500
S = 2
CW, CH = W * S, H * S


def lerp(a, b, t):
    return tuple(round(a[i] + (b[i] - a[i]) * t) for i in range(3))


sky_top = (12, 16, 40)
sky_mid = (58, 34, 82)
sky_bottom = (244, 140, 92)
horizon = int(CH * 0.62)

img = Image.new('RGB', (CW, CH))
px = img.load()
for y in range(CH):
    if y < horizon:
        t = y / horizon
        if t < 0.55:
            c = lerp(sky_top, sky_mid, t / 0.55)
        else:
            c = lerp(sky_mid, sky_bottom, (t - 0.55) / 0.45)
    else:
        t = (y - horizon) / (CH - horizon)
        c = lerp((32, 26, 46), (8, 10, 18), t)
    for x in range(CW):
        px[x, y] = c

draw = ImageDraw.Draw(img)

sun_r = int(CW * 0.055)
sun_cx, sun_cy = int(CW * 0.70), int(horizon * 0.68)
for i in range(10, 0, -1):
    r = int(sun_r * (1 + i * 0.16))
    alpha = int(16 + (10 - i) * 3)
    glow = Image.new('RGBA', (r * 2, r * 2), (0, 0, 0, 0))
    gd = ImageDraw.Draw(glow)
    gd.ellipse((0, 0, r * 2, r * 2), fill=(255, 205, 130, alpha))
    img.paste(glow, (sun_cx - r, sun_cy - r), glow)
draw.ellipse((sun_cx - sun_r, sun_cy - sun_r, sun_cx + sun_r, sun_cy + sun_r), fill=(255, 226, 168))

far = [(0, horizon)]
far += [(int(CW * 0.08), int(horizon * 0.72)), (int(CW * 0.16), int(horizon * 0.86)),
        (int(CW * 0.27), int(horizon * 0.60)), (int(CW * 0.36), int(horizon * 0.90)),
        (int(CW * 0.47), int(horizon * 0.66)), (int(CW * 0.55), int(horizon * 0.88)),
        (int(CW * 0.66), int(horizon * 0.70)), (int(CW * 0.78), int(horizon * 0.92)),
        (int(CW * 0.88), int(horizon * 0.74)), (CW, int(horizon * 0.86)), (CW, horizon)]
draw.polygon(far, fill=(46, 44, 78))

near = [(0, horizon), (0, int(horizon * 0.94))]
near += [(int(CW * 0.10), int(horizon * 0.78)), (int(CW * 0.22), int(horizon * 0.98)),
         (int(CW * 0.34), int(horizon * 0.74)), (int(CW * 0.44), int(horizon * 0.96)),
         (int(CW * 0.60), int(horizon * 0.80)), (int(CW * 0.72), int(horizon * 0.99)),
         (int(CW * 0.84), int(horizon * 0.76)), (int(CW * 0.93), int(horizon * 0.95)), (CW, int(horizon * 0.82)), (CW, horizon)]
draw.polygon(near, fill=(24, 24, 44))

water = img.crop((0, horizon, CW, CH)).transpose(Image.FLIP_TOP_BOTTOM)
water = water.filter(ImageFilter.GaussianBlur(radius=8 * S))
img.paste(water, (0, horizon))
water_draw = ImageDraw.Draw(img)
refl_cx = sun_cx
refl = Image.new('RGBA', (CW, CH - horizon), (0, 0, 0, 0))
rd = ImageDraw.Draw(refl)
for i in range(20, 0, -1):
    t = i / 20
    w = int(sun_r * (1.0 + t * 0.55))
    h = max(S, int(sun_r * 0.14))
    cy = int((CH - horizon) * t * 0.92)
    rd.ellipse((refl_cx - w, cy - h, refl_cx + w, cy + h), fill=(255, 208, 152, int(10 + 42 * (1 - t))))
refl = refl.filter(ImageFilter.GaussianBlur(radius=7 * S))
img.paste(refl, (0, horizon), refl)


draw = ImageDraw.Draw(img)
draw.rectangle((0, CH - int(CH * 0.035), CW, CH), fill=(6, 7, 12))

out = img.resize((W, H), Image.LANCZOS)
out.save('entry/src/main/resources/rawfile/sample.jpg', quality=92, optimize=True)
print('written', out.size)