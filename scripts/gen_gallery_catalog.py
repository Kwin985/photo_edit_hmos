"""从 Rust `artistic.rs` 的 gallery! 宏生成 ArkTS 画廊目录。

用法：python scripts/gen_gallery_catalog.py
源数据只有一份（artistic.rs 的 params_doc），本脚本保证 UI 目录与引擎的 47 个预设、
参数范围、默认值、选项列表零漂移。生成物：entry/src/main/ets/engine/GalleryCatalog.ets。
"""

import re
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SRC = ROOT / "native" / "crates" / "algo" / "src" / "artistic.rs"
OUT = ROOT / "entry" / "src" / "main" / "ets" / "engine" / "GalleryCatalog.ets"

PRESET_LABELS = {
    "Colored Pencil": "彩色铅笔", "Cutout": "剪贴图", "Dry Brush": "干画笔",
    "Film Grain": "胶片颗粒", "Fresco": "壁画", "Neon Glow": "霓虹灯光",
    "Paint Daubs": "绘画涂抹", "Palette Knife": "调色刀", "Plastic Wrap": "塑料包装",
    "Poster Edges": "海报边缘", "Rough Pastels": "粗糙蜡笔", "Smudge Stick": "涂抹棒",
    "Sponge": "海绵", "Underpainting": "底纹效果", "Watercolor": "水彩",
    "Accented Edges": "强化边缘", "Angled Strokes": "倾斜笔划", "Crosshatch": "交叉线",
    "Dark Strokes": "深色线条", "Ink Outlines": "油墨概况", "Spatter": "喷溅",
    "Sprayed Strokes": "喷色描边", "Sumi-e": "水墨画", "Diffuse Glow": "扩散亮光",
    "Glass": "玻璃", "Ocean Ripple": "海洋波纹", "Bas Relief": "基底凸现",
    "Chalk & Charcoal": "粉笔和炭笔", "Charcoal": "炭笔", "Chrome": "铬黄",
    "Conté Crayon": "彩色粉笔", "Graphic Pen": "绘图笔", "Halftone Pattern": "半调图案",
    "Note Paper": "便条纸", "Photocopy": "影印", "Plaster": "塑料效果",
    "Reticulation": "网状", "Stamp": "图章", "Torn Edges": "撕边",
    "Water Paper": "水彩画纸", "Glowing Edges": "照亮边缘", "Craquelure": "龟裂缝",
    "Grain": "颗粒", "Mosaic Tiles": "马赛克拼贴", "Patchwork": "拼缀图",
    "Stained Glass": "染色玻璃", "Texturizer": "纹理化",
}

CATEGORY_LABELS = {
    "Artistic": "艺术效果", "Brush Strokes": "画笔描边", "Distort": "扭曲",
    "Sketch": "素描", "Stylize": "风格化", "Texture": "纹理",
    "Artistic, Brush Strokes": "艺术效果",  # 不会被宏用到，仅防御
}

PARAM_LABELS = {
    "pencilWidth": "笔宽", "strokePressure": "笔压", "paperBrightness": "纸亮度",
    "numberOfLevels": "色阶数", "edgeSimplicity": "边缘简化", "edgeFidelity": "边缘逼真",
    "brushSize": "画笔大小", "brushDetail": "画笔细节", "texture": "纹理",
    "grain": "颗粒", "highlightArea": "高光区域", "intensity": "强度",
    "glowSize": "光晕大小", "glowBrightness": "光晕亮度", "glowColor": "光晕颜色",
    "brushType": "画笔类型", "sharpness": "锐度", "strokeSize": "笔触大小",
    "strokeDetail": "笔触细节", "softness": "柔和度", "highlightStrength": "高光强度",
    "detail": "细节", "smoothness": "平滑度", "edgeThickness": "边缘厚度",
    "edgeIntensity": "边缘强度", "posterization": "海报化", "strokeLength": "笔画长度",
    "scaling": "缩放", "relief": "浮雕", "light": "光照", "invert": "反转",
    "definition": "清晰度", "textureCoverage": "纹理覆盖", "edgeWidth": "边缘宽度",
    "edgeBrightness": "边缘亮度", "directionBalance": "方向平衡", "strength": "力度",
    "balance": "平衡", "blackIntensity": "黑色强度", "whiteIntensity": "白色强度",
    "darkIntensity": "暗部强度", "lightIntensity": "亮部强度", "sprayRadius": "喷射半径",
    "strokeDirection": "笔触方向", "strokeWidth": "笔画宽度", "contrast": "对比度",
    "graininess": "颗粒度", "glowAmount": "发光量", "clearAmount": "清晰度量",
    "distortion": "扭曲度", "rippleSize": "波纹大小", "rippleMagnitude": "波纹幅度",
    "lightDarkBalance": "明暗平衡", "charcoalArea": "炭笔区", "chalkArea": "粉笔区",
    "charcoalThickness": "炭笔粗细", "foregroundLevel": "前景色阶",
    "backgroundLevel": "背景色阶", "patternType": "图案类型", "imageBalance": "图像平衡",
    "darkness": "暗度", "density": "密度", "fiberLength": "纤维长度", "brightness": "亮度",
    "crackSpacing": "裂缝间距", "crackDepth": "裂缝深度", "crackBrightness": "裂缝亮度",
    "grainType": "颗粒类型", "tileSize": "瓷砖大小", "groutWidth": "间隙宽度",
    "lightenGrout": "间隙亮度", "squareSize": "方格大小", "cellSize": "单元格大小",
    "borderThickness": "边界厚度", "kind": "类型", "upper": "上侧",
}

ENTRY = re.compile(
    r'^\s*(\w+)\s*=\s*"([^"]+)",\s*"([^"]+)",\s*"([^"]+)",\s*r#"(.*)"#;', re.M
)


def parse_params(doc: str):
    out = []
    inner = doc.strip().strip("{}")
    for part in inner.split(","):
        key, sep, value = part.partition(":")
        if not sep:
            continue
        key = key.strip().strip('"')
        value = value.strip()
        if value.startswith('"'):
            choices = value.strip('"').split("|")
            out.append((key, 0, len(choices) - 1, 1, 0, choices))
        elif value == "bool":
            out.append((key, 0, 1, 1, 0, ["关", "开"]))
        elif "=" in value:
            rng, default = value.split("=", 1)
            lo, hi = rng.split("..", 1)
            lo, hi, default = float(lo), float(hi), float(default)
            step = 1 if hi - lo >= 10 else 0.1
            def fmt(v):
                return str(int(v)) if float(v).is_integer() else str(v)
            out.append((key, fmt(lo), fmt(hi), str(step), fmt(default), []))
        # "json" 颜色项使用 GalleryEffect 的默认颜色，不作为 UI 参数。
    return out


def ts_array(items):
    return "[" + ", ".join("'" + s.replace("'", "\\'") + "'" for s in items) + "]"


def main():
    text = SRC.read_text(encoding="utf-8")
    entries = ENTRY.findall(text)
    lines = []
    lines.append("// 由 scripts/gen_gallery_catalog.py 从 artistic.rs 的 gallery! 宏生成，请勿手工改参数。")
    lines.append("// 47 种 Filter Gallery 预设：id / 中文名 / 分组 / 参数范围与默认值与引擎一致。")
    lines.append("")
    lines.append("export interface GalleryParamDef {")
    lines.append("  key: string;")
    lines.append("  label: string;")
    lines.append("  min: number;")
    lines.append("  max: number;")
    lines.append("  step: number;")
    lines.append("  value: number;")
    lines.append("  choices: string[];")
    lines.append("}")
    lines.append("")
    lines.append("export interface GalleryPresetDef {")
    lines.append("  id: string;")
    lines.append("  label: string;")
    lines.append("  category: string;")
    lines.append("  params: GalleryParamDef[];")
    lines.append("}")
    lines.append("")
    lines.append("function gp(key: string, label: string, min: number, max: number, step: number, value: number, choices: string[]): GalleryParamDef {")
    lines.append("  const p: GalleryParamDef = { key: key, label: label, min: min, max: max, step: step, value: value, choices: choices };")
    lines.append("  return p;")
    lines.append("}")
    lines.append("")
    lines.append("function gdef(id: string, label: string, category: string, params: GalleryParamDef[]): GalleryPresetDef {")
    lines.append("  const p: GalleryPresetDef = { id: id, label: label, category: category, params: params };")
    lines.append("  return p;")
    lines.append("}")
    lines.append("")
    lines.append("export const GALLERY_CATEGORIES: string[] = ['艺术效果', '画笔描边', '扭曲', '素描', '风格化', '纹理'];")
    lines.append("")
    lines.append("export const GALLERY_PRESETS: GalleryPresetDef[] = [")
    for variant, key, name, category, doc in entries:
        label = PRESET_LABELS.get(name, name)
        cat = CATEGORY_LABELS.get(category, category)
        params = parse_params(doc)
        parts = []
        for pkey, lo, hi, step, default, choices in params:
            label_key = PARAM_LABELS.get(pkey, pkey)
            parts.append(
                f"gp('{pkey}', '{label_key}', {lo}, {hi}, {step}, {default}, {ts_array(choices)})"
            )
        lines.append(f"  gdef('{key}', '{label}', '{cat}', [{', '.join(parts)}]),")
    lines.append("];")
    lines.append("")
    lines.append("/** 按 id 找预设；找不到回退到第一项。 */")
    lines.append("export function findGalleryPreset(id: string): GalleryPresetDef {")
    lines.append("  for (const p of GALLERY_PRESETS) {")
    lines.append("    if (p.id === id) { return p; }")
    lines.append("  }")
    lines.append("  return GALLERY_PRESETS[0];")
    lines.append("}")
    lines.append("")
    lines.append("/** 某个分组下的预设列表。 */")
    lines.append("export function galleryPresetsInCategory(category: string): GalleryPresetDef[] {")
    lines.append("  const out: GalleryPresetDef[] = [];")
    lines.append("  for (const p of GALLERY_PRESETS) {")
    lines.append("    if (p.category === category) { out.push(p); }")
    lines.append("  }")
    lines.append("  return out;")
    lines.append("}")
    lines.append("")
    lines.append("/** 生成带一层画廊特效的 filterGallery 参数 JSON（颜色走 GalleryEffect 默认）。 */")
    lines.append("export function buildGalleryArgs(preset: GalleryPresetDef, values: Map<string, number>): string {")
    lines.append("  const effectValues: Record<string, number> = {};")
    lines.append("  for (const param of preset.params) {")
    lines.append("    const stored: number | undefined = values.get(param.key);")
    lines.append("    const raw: number = stored === undefined ? param.value : stored;")
    lines.append("    effectValues[param.key] = Math.min(param.max, Math.max(param.min, raw));")
    lines.append("  }")
    lines.append("  const effect: Record<string, Object> = {")
    lines.append("    'filter': preset.id, 'values': effectValues,")
    lines.append("    'foreground': [0, 0, 0, 1], 'background': [1, 1, 1, 1], 'color': [0, 0.62, 1, 1]")
    lines.append("  };")
    lines.append("  const payload: Record<string, Object> = { 'filter': 'filterGallery', 'effects': [effect] };")
    lines.append("  return JSON.stringify(payload);")
    lines.append("}")
    OUT.write_text("\n".join(lines) + "\n", encoding="utf-8")
    print(f"generated {len(entries)} presets -> {OUT}")


if __name__ == "__main__":
    main()
