// 由 scripts/gen_gallery_catalog.py 从 artistic.rs 的 gallery! 宏生成，请勿手工改参数。
// 47 种 Filter Gallery 预设：id / 中文名 / 分组 / 参数范围与默认值与引擎一致。

export interface GalleryParamDef {
  key: string;
  label: string;
  min: number;
  max: number;
  step: number;
  value: number;
  choices: string[];
}

export interface GalleryPresetDef {
  id: string;
  label: string;
  category: string;
  params: GalleryParamDef[];
}

function gp(key: string, label: string, min: number, max: number, step: number, value: number, choices: string[]): GalleryParamDef {
  const p: GalleryParamDef = { key: key, label: label, min: min, max: max, step: step, value: value, choices: choices };
  return p;
}

function gdef(id: string, label: string, category: string, params: GalleryParamDef[]): GalleryPresetDef {
  const p: GalleryPresetDef = { id: id, label: label, category: category, params: params };
  return p;
}

export const GALLERY_CATEGORIES: string[] = ['艺术效果', '画笔描边', '扭曲', '素描', '风格化', '纹理'];

export const GALLERY_PRESETS: GalleryPresetDef[] = [
  gdef('coloredPencil', '彩色铅笔', '艺术效果', [gp('pencilWidth', '笔宽', 1, 24, 1, 4, []), gp('strokePressure', '笔压', 0, 15, 1, 8, []), gp('paperBrightness', '纸亮度', 0, 50, 1, 25, [])]),
  gdef('cutout', '剪贴图', '艺术效果', [gp('numberOfLevels', '色阶数', 2, 8, 0.1, 4, []), gp('edgeSimplicity', '边缘简化', 0, 10, 1, 4, []), gp('edgeFidelity', '边缘逼真', 1, 3, 0.1, 2, [])]),
  gdef('dryBrush', '干画笔', '艺术效果', [gp('brushSize', '画笔大小', 0, 10, 1, 2, []), gp('brushDetail', '画笔细节', 0, 10, 1, 8, []), gp('texture', '纹理', 1, 3, 0.1, 1, [])]),
  gdef('filmGrain', '胶片颗粒', '艺术效果', [gp('grain', '颗粒', 0, 20, 1, 4, []), gp('highlightArea', '高光区域', 0, 20, 1, 0, []), gp('intensity', '强度', 0, 10, 1, 10, [])]),
  gdef('fresco', '壁画', '艺术效果', [gp('brushSize', '画笔大小', 0, 10, 1, 2, []), gp('brushDetail', '画笔细节', 0, 10, 1, 8, []), gp('texture', '纹理', 1, 3, 0.1, 1, [])]),
  gdef('neonGlow', '霓虹灯光', '艺术效果', [gp('glowSize', '光晕大小', -24, 24, 1, 5, []), gp('glowBrightness', '光晕亮度', 0, 50, 1, 15, [])]),
  gdef('paintDaubs', '绘画涂抹', '艺术效果', [gp('brushSize', '画笔大小', 1, 50, 1, 8, []), gp('sharpness', '锐度', 0, 40, 1, 7, []), gp('brushType', '画笔类型', 0, 5, 1, 0, ['simple', 'lightRough', 'darkRough', 'wideSharp', 'wideBlurry', 'sparkle'])]),
  gdef('paletteKnife', '调色刀', '艺术效果', [gp('strokeSize', '笔触大小', 1, 50, 1, 25, []), gp('strokeDetail', '笔触细节', 1, 3, 0.1, 3, []), gp('softness', '柔和度', 0, 10, 1, 0, [])]),
  gdef('plasticWrap', '塑料包装', '艺术效果', [gp('highlightStrength', '高光强度', 0, 20, 1, 15, []), gp('detail', '细节', 1, 15, 1, 9, []), gp('smoothness', '平滑度', 1, 15, 1, 7, [])]),
  gdef('posterEdges', '海报边缘', '艺术效果', [gp('edgeThickness', '边缘厚度', 0, 10, 1, 2, []), gp('edgeIntensity', '边缘强度', 0, 10, 1, 1, []), gp('posterization', '海报化', 0, 6, 0.1, 2, [])]),
  gdef('roughPastels', '粗糙蜡笔', '艺术效果', [gp('strokeLength', '笔画长度', 0, 40, 1, 6, []), gp('strokeDetail', '笔触细节', 1, 20, 1, 4, []), gp('texture', '纹理', 0, 3, 1, 0, ['canvas', 'brick', 'burlap', 'sandstone']), gp('scaling', '缩放', 50, 200, 1, 100, []), gp('relief', '浮雕', 0, 50, 1, 20, []), gp('light', '光照', 0, 7, 1, 0, ['bottom', 'bottomLeft', 'left', 'topLeft', 'top', 'topRight', 'right', 'bottomRight']), gp('invert', '反转', 0, 1, 1, 0, ['关', '开'])]),
  gdef('smudgeStick', '涂抹棒', '艺术效果', [gp('strokeLength', '笔画长度', 0, 10, 1, 2, []), gp('highlightArea', '高光区域', 0, 20, 1, 0, []), gp('intensity', '强度', 0, 10, 1, 10, [])]),
  gdef('sponge', '海绵', '艺术效果', [gp('brushSize', '画笔大小', 0, 10, 1, 2, []), gp('definition', '清晰度', 0, 25, 1, 12, []), gp('smoothness', '平滑度', 1, 15, 1, 5, [])]),
  gdef('underpainting', '底纹效果', '艺术效果', [gp('brushSize', '画笔大小', 0, 40, 1, 6, []), gp('textureCoverage', '纹理覆盖', 0, 40, 1, 16, []), gp('texture', '纹理', 0, 3, 1, 0, ['canvas', 'brick', 'burlap', 'sandstone']), gp('scaling', '缩放', 50, 200, 1, 100, []), gp('relief', '浮雕', 0, 50, 1, 4, []), gp('light', '光照', 0, 7, 1, 0, ['top', 'topRight', 'right', 'bottomRight', 'bottom', 'bottomLeft', 'left', 'topLeft']), gp('invert', '反转', 0, 1, 1, 0, ['关', '开'])]),
  gdef('watercolor', '水彩', '艺术效果', [gp('brushDetail', '画笔细节', 1, 14, 1, 9, []), gp('shadowIntensity', 'shadowIntensity', 0, 10, 1, 1, []), gp('texture', '纹理', 1, 3, 0.1, 1, [])]),
  gdef('accentedEdges', '强化边缘', '画笔描边', [gp('edgeWidth', '边缘宽度', 1, 14, 1, 2, []), gp('edgeBrightness', '边缘亮度', 0, 50, 1, 38, []), gp('smoothness', '平滑度', 1, 15, 1, 5, [])]),
  gdef('angledStrokes', '倾斜笔划', '画笔描边', [gp('directionBalance', '方向平衡', 0, 100, 1, 50, []), gp('strokeLength', '笔画长度', 3, 50, 1, 15, []), gp('sharpness', '锐度', 0, 10, 1, 3, [])]),
  gdef('crosshatch', '交叉线', '画笔描边', [gp('strokeLength', '笔画长度', 3, 50, 1, 9, []), gp('sharpness', '锐度', 0, 20, 1, 6, []), gp('strength', '力度', 1, 3, 0.1, 1, [])]),
  gdef('darkStrokes', '深色线条', '画笔描边', [gp('balance', '平衡', 0, 10, 1, 5, []), gp('blackIntensity', '黑色强度', 0, 10, 1, 6, []), gp('whiteIntensity', '白色强度', 0, 10, 1, 2, [])]),
  gdef('inkOutlines', '油墨概况', '画笔描边', [gp('strokeLength', '笔画长度', 1, 50, 1, 4, []), gp('darkIntensity', '暗部强度', 0, 50, 1, 20, []), gp('lightIntensity', '亮部强度', 0, 50, 1, 10, [])]),
  gdef('spatter', '喷溅', '画笔描边', [gp('sprayRadius', '喷射半径', 0, 25, 1, 10, []), gp('smoothness', '平滑度', 1, 15, 1, 5, [])]),
  gdef('sprayedStrokes', '喷色描边', '画笔描边', [gp('strokeLength', '笔画长度', 0, 20, 1, 12, []), gp('sprayRadius', '喷射半径', 0, 25, 1, 7, []), gp('strokeDirection', '笔触方向', 0, 3, 1, 0, ['rightDiagonal', 'horizontal', 'leftDiagonal', 'vertical'])]),
  gdef('sumiE', '水墨画', '画笔描边', [gp('strokeWidth', '笔画宽度', 3, 15, 1, 10, []), gp('strokePressure', '笔压', 0, 15, 1, 2, []), gp('contrast', '对比度', 0, 40, 1, 16, [])]),
  gdef('diffuseGlow', '扩散亮光', '扭曲', [gp('graininess', '颗粒度', 0, 10, 1, 6, []), gp('glowAmount', '发光量', 0, 20, 1, 10, []), gp('clearAmount', '清晰度量', 0, 20, 1, 15, [])]),
  gdef('glass', '玻璃', '扭曲', [gp('distortion', '扭曲度', 0, 20, 1, 5, []), gp('smoothness', '平滑度', 1, 15, 1, 3, []), gp('texture', '纹理', 0, 3, 1, 0, ['frosted', 'blocks', 'canvas', 'tinyLens']), gp('scaling', '缩放', 50, 200, 1, 100, []), gp('invert', '反转', 0, 1, 1, 0, ['关', '开'])]),
  gdef('oceanRipple', '海洋波纹', '扭曲', [gp('rippleSize', '波纹大小', 1, 15, 1, 9, []), gp('rippleMagnitude', '波纹幅度', 0, 20, 1, 9, [])]),
  gdef('basRelief', '基底凸现', '素描', [gp('detail', '细节', 1, 15, 1, 13, []), gp('smoothness', '平滑度', 1, 15, 1, 3, []), gp('light', '光照', 0, 7, 1, 0, ['bottom', 'bottomLeft', 'left', 'topLeft', 'top', 'topRight', 'right', 'bottomRight'])]),
  gdef('chalkCharcoal', '粉笔和炭笔', '素描', [gp('charcoalArea', '炭笔区', 0, 20, 1, 6, []), gp('chalkArea', '粉笔区', 0, 20, 1, 6, []), gp('strokePressure', '笔压', 0, 5, 0.1, 1, [])]),
  gdef('charcoal', '炭笔', '素描', [gp('charcoalThickness', '炭笔粗细', 1, 7, 0.1, 1, []), gp('detail', '细节', 0, 5, 0.1, 5, []), gp('lightDarkBalance', '明暗平衡', 0, 100, 1, 50, [])]),
  gdef('chrome', '铬黄', '素描', [gp('detail', '细节', 0, 10, 1, 4, []), gp('smoothness', '平滑度', 0, 10, 1, 7, [])]),
  gdef('conteCrayon', '彩色粉笔', '素描', [gp('foregroundLevel', '前景色阶', 1, 15, 1, 11, []), gp('backgroundLevel', '背景色阶', 1, 15, 1, 7, []), gp('texture', '纹理', 0, 3, 1, 0, ['canvas', 'brick', 'burlap', 'sandstone']), gp('scaling', '缩放', 50, 200, 1, 100, []), gp('relief', '浮雕', 0, 50, 1, 4, []), gp('light', '光照', 0, 7, 1, 0, ['top', 'topRight', 'right', 'bottomRight', 'bottom', 'bottomLeft', 'left', 'topLeft']), gp('invert', '反转', 0, 1, 1, 0, ['关', '开'])]),
  gdef('graphicPen', '绘图笔', '素描', [gp('strokeLength', '笔画长度', 1, 15, 1, 15, []), gp('lightDarkBalance', '明暗平衡', 0, 100, 1, 50, []), gp('strokeDirection', '笔触方向', 0, 3, 1, 0, ['rightDiagonal', 'horizontal', 'leftDiagonal', 'vertical'])]),
  gdef('halftonePattern', '半调图案', '素描', [gp('size', 'size', 1, 12, 1, 1, []), gp('contrast', '对比度', 0, 50, 1, 5, []), gp('patternType', '图案类型', 0, 2, 1, 0, ['dot', 'circle', 'line'])]),
  gdef('notePaper', '便条纸', '素描', [gp('imageBalance', '图像平衡', 0, 50, 1, 25, []), gp('graininess', '颗粒度', 0, 20, 1, 10, []), gp('relief', '浮雕', 0, 25, 1, 11, [])]),
  gdef('photocopy', '影印', '素描', [gp('detail', '细节', 1, 24, 1, 7, []), gp('darkness', '暗度', 1, 50, 1, 8, [])]),
  gdef('plaster', '塑料效果', '素描', [gp('imageBalance', '图像平衡', 0, 50, 1, 20, []), gp('smoothness', '平滑度', 1, 15, 1, 2, []), gp('light', '光照', 0, 7, 1, 0, ['top', 'topRight', 'right', 'bottomRight', 'bottom', 'bottomLeft', 'left', 'topLeft'])]),
  gdef('reticulation', '网状', '素描', [gp('density', '密度', 0, 50, 1, 12, []), gp('foregroundLevel', '前景色阶', 0, 50, 1, 40, []), gp('backgroundLevel', '背景色阶', 0, 50, 1, 5, [])]),
  gdef('stamp', '图章', '素描', [gp('lightDarkBalance', '明暗平衡', 0, 50, 1, 25, []), gp('smoothness', '平滑度', 1, 50, 1, 5, [])]),
  gdef('tornEdges', '撕边', '素描', [gp('imageBalance', '图像平衡', 0, 50, 1, 25, []), gp('smoothness', '平滑度', 1, 15, 1, 11, []), gp('contrast', '对比度', 1, 25, 1, 17, [])]),
  gdef('waterPaper', '水彩画纸', '素描', [gp('fiberLength', '纤维长度', 3, 50, 1, 15, []), gp('brightness', '亮度', 0, 100, 1, 60, []), gp('contrast', '对比度', 0, 100, 1, 80, [])]),
  gdef('glowingEdges', '照亮边缘', '风格化', [gp('edgeWidth', '边缘宽度', 1, 14, 1, 2, []), gp('edgeBrightness', '边缘亮度', 0, 20, 1, 6, []), gp('smoothness', '平滑度', 1, 15, 1, 5, [])]),
  gdef('craquelure', '龟裂缝', '纹理', [gp('crackSpacing', '裂缝间距', 2, 100, 1, 15, []), gp('crackDepth', '裂缝深度', 0, 10, 1, 6, []), gp('crackBrightness', '裂缝亮度', 0, 10, 1, 9, [])]),
  gdef('grain', '颗粒', '纹理', [gp('intensity', '强度', 0, 100, 1, 40, []), gp('contrast', '对比度', 0, 100, 1, 50, []), gp('grainType', '颗粒类型', 0, 9, 1, 0, ['regular', 'soft', 'sprinkles', 'clumped', 'contrasty', 'enlarged', 'stippled', 'horizontal', 'vertical', 'speckle'])]),
  gdef('mosaicTiles', '马赛克拼贴', '纹理', [gp('tileSize', '瓷砖大小', 2, 100, 1, 12, []), gp('groutWidth', '间隙宽度', 1, 15, 1, 3, []), gp('lightenGrout', '间隙亮度', 0, 10, 1, 9, [])]),
  gdef('patchwork', '拼缀图', '纹理', [gp('squareSize', '方格大小', 0, 10, 1, 4, []), gp('relief', '浮雕', 0, 25, 1, 8, [])]),
  gdef('stainedGlass', '染色玻璃', '纹理', [gp('cellSize', '单元格大小', 2, 50, 1, 10, []), gp('borderThickness', '边界厚度', 1, 20, 1, 4, []), gp('lightIntensity', '亮部强度', 0, 10, 1, 3, [])]),
  gdef('texturizer', '纹理化', '纹理', [gp('texture', '纹理', 0, 3, 1, 0, ['canvas', 'brick', 'burlap', 'sandstone']), gp('scaling', '缩放', 50, 200, 1, 100, []), gp('relief', '浮雕', 0, 50, 1, 4, []), gp('light', '光照', 0, 7, 1, 0, ['top', 'topRight', 'right', 'bottomRight', 'bottom', 'bottomLeft', 'left', 'topLeft']), gp('invert', '反转', 0, 1, 1, 0, ['关', '开'])]),
];

/** 按 id 找预设；找不到回退到第一项。 */
export function findGalleryPreset(id: string): GalleryPresetDef {
  for (const p of GALLERY_PRESETS) {
    if (p.id === id) { return p; }
  }
  return GALLERY_PRESETS[0];
}

/** 某个分组下的预设列表。 */
export function galleryPresetsInCategory(category: string): GalleryPresetDef[] {
  const out: GalleryPresetDef[] = [];
  for (const p of GALLERY_PRESETS) {
    if (p.category === category) { out.push(p); }
  }
  return out;
}

/** 生成带一层画廊特效的 filterGallery 参数 JSON（颜色走 GalleryEffect 默认）。 */
export function buildGalleryArgs(preset: GalleryPresetDef, values: Map<string, number>): string {
  const effectValues: Record<string, number> = {};
  for (const param of preset.params) {
    const stored: number | undefined = values.get(param.key);
    const raw: number = stored === undefined ? param.value : stored;
    effectValues[param.key] = Math.min(param.max, Math.max(param.min, raw));
  }
  const effect: Record<string, Object> = {
    'filter': preset.id, 'values': effectValues,
    'foreground': [0, 0, 0, 1], 'background': [1, 1, 1, 1], 'color': [0, 0.62, 1, 1]
  };
  const payload: Record<string, Object> = { 'filter': 'filterGallery', 'effects': [effect] };
  return JSON.stringify(payload);
}
