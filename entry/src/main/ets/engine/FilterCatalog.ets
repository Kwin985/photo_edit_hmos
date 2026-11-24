/**
 * 滤镜目录（1.3.2）：与 Rust `photocraft_algo::FilterParams` 的 61 个 serde tag 一一对应。
 *
 * 每个条目给出中文名、所属分组、少量可调数值参数，以及枚举/颜色/结构参数的 sane 默认值
 * （`fixed`）。UI 只渲染数值滑杆；非数值项固定为引擎已验证的默认值，避免"点了报错"的假按钮。
 *
 * 参数字段采用 serde 的 camelCase 命名（FilterParams 上 `rename_all_fields = "camelCase"`）。
 */

export interface FilterParamDef {
  key: string;
  label: string;
  min: number;
  max: number;
  step: number;
  value: number;
}

export interface FilterDef {
  id: string;
  label: string;
  group: string;
  params: FilterParamDef[];
  fixed: Record<string, Object>;
}

function p(key: string, label: string, min: number, max: number, step: number, value: number): FilterParamDef {
  const def: FilterParamDef = { key: key, label: label, min: min, max: max, step: step, value: value };
  return def;
}

function f(id: string, label: string, group: string, params: FilterParamDef[], fixed: Record<string, Object>): FilterDef {
  const def: FilterDef = { id: id, label: label, group: group, params: params, fixed: fixed };
  return def;
}

// ArkTS 严格模式：对象字面量必须有显式类型，故嵌套结构先声明接口再构造。
interface IrisPinDef {
  x: number;
  y: number;
  radiusX: number;
  radiusY: number;
  angle: number;
  roundness: number;
  feather: number;
  blur: number;
}

interface FieldPinDef {
  x: number;
  y: number;
  blur: number;
}

interface SpinPinDef {
  x: number;
  y: number;
  radiusX: number;
  radiusY: number;
  angle: number;
  blurAngle: number;
}

interface BlurPathDef {
  points: number[][];
  speed: number;
  taper: number;
}

interface LightDef {
  kind: string;
  x: number;
  y: number;
  z: number;
  targetX: number;
  targetY: number;
  angle: number;
  elevation: number;
  color: number[];
  intensity: number;
  cone: number;
  hotspot: number;
  radius: number;
}

interface GalleryEffectDef {
  filter: string;
  values: Record<string, number>;
  foreground: number[];
  background: number[];
  color: number[];
}

function irisPin(): IrisPinDef {
  const v: IrisPinDef = { x: 0.5, y: 0.5, radiusX: 0.35, radiusY: 0.25, angle: 0, roundness: 0, feather: 0.5, blur: 15 };
  return v;
}

function fieldPin(): FieldPinDef {
  const v: FieldPinDef = { x: 0.5, y: 0.5, blur: 15 };
  return v;
}

function spinPin(): SpinPinDef {
  const v: SpinPinDef = { x: 0.5, y: 0.5, radiusX: 0.3, radiusY: 0.3, angle: 0, blurAngle: 15 };
  return v;
}

function blurPath(): BlurPathDef {
  const v: BlurPathDef = { points: [[0.2, 0.5], [0.8, 0.5]], speed: 50, taper: 0 };
  return v;
}

function spotLight(): LightDef {
  const v: LightDef = {
    kind: 'spot', x: 0.25, y: 0.2, z: 0.6, targetX: 0.5, targetY: 0.55,
    angle: 135, elevation: 45, color: [1, 1, 1], intensity: 75, cone: 45, hotspot: 50, radius: 0.8
  };
  return v;
}

function watercolorEffect(): GalleryEffectDef {
  const values: Record<string, number> = { 'brushDetail': 9, 'shadowIntensity': 1, 'texture': 1 };
  const v: GalleryEffectDef = {
    filter: 'watercolor', values: values,
    foreground: [0, 0, 0, 1], background: [1, 1, 1, 1], color: [0, 1, 1, 1]
  };
  return v;
}

export const FILTER_GROUPS: string[] = [
  '模糊锐化', '杂色修复', '扭曲变形', '像素风格', '艺术渲染', '其他滤镜'
];

export const FILTERS: FilterDef[] = [
  // ── 模糊 / 锐化 ────────────────────────────────────────────────────────
  f('gaussianBlur', '高斯模糊', '模糊锐化', [p('radius', '半径', 0.1, 100, 0.1, 4)], {}),
  f('boxBlur', '方框模糊', '模糊锐化', [p('radius', '半径', 1, 100, 1, 5)], {}),
  f('motionBlur', '动感模糊', '模糊锐化', [p('angle', '角度', -180, 180, 1, 0), p('distance', '距离', 1, 200, 1, 20)], {}),
  f('radialBlur', '径向模糊', '模糊锐化', [p('amount', '数量', 0, 100, 1, 30), p('centerX', '中心X', 0, 1, 0.01, 0.5), p('centerY', '中心Y', 0, 1, 0.01, 0.5)], { 'method': 'spin', 'quality': 'good' }),
  f('surfaceBlur', '表面模糊', '模糊锐化', [p('radius', '半径', 1, 30, 1, 8), p('threshold', '阈值', 0, 255, 1, 32)], {}),
  f('unsharpMask', 'USM 锐化', '模糊锐化', [p('amount', '数量', 0, 500, 1, 100), p('radius', '半径', 0.1, 20, 0.1, 2), p('threshold', '阈值', 0, 255, 1, 3)], {}),
  f('smartSharpen', '智能锐化', '模糊锐化', [p('amount', '数量', 1, 300, 1, 100), p('radius', '半径', 0.1, 20, 0.1, 2), p('reduceNoise', '减少杂色', 0, 100, 1, 10)], {}),
  f('highPass', '高通滤波', '模糊锐化', [p('radius', '半径', 0.1, 50, 0.1, 4)], {}),
  f('smartBlur', '智能模糊', '模糊锐化', [p('radius', '半径', 0.1, 100, 0.1, 5), p('threshold', '阈值', 0.1, 100, 0.1, 20)], { 'quality': 'medium', 'mode': 'normal' }),
  f('lensBlur', '镜头模糊', '模糊锐化', [p('radius', '半径', 0, 100, 1, 15), p('blades', '叶片', 3, 8, 1, 6), p('curvature', '曲率', 0, 100, 1, 50), p('brightness', '亮度', 0, 100, 1, 30)], { 'rotation': 0, 'depth': 'none', 'focalDistance': 128, 'invertDepth': false, 'threshold': 255, 'noise': 0, 'distribution': 'uniform', 'monochromatic': true, 'seed': 12345 }),
  f('shapeBlur', '形状模糊', '模糊锐化', [p('radius', '半径', 1, 100, 1, 10)], { 'shape': 'circle' }),
  f('tiltShift', '移轴模糊', '模糊锐化', [p('blur', '模糊', 0, 100, 1, 20), p('centerX', '中心X', 0, 1, 0.01, 0.5), p('centerY', '中心Y', 0, 1, 0.01, 0.5), p('focus', '清晰区', 0, 1, 0.01, 0.25), p('transition', '过渡', 0, 1, 0.01, 0.2)], { 'angle': 0 }),
  f('irisBlur', '光圈模糊', '模糊锐化', [], { 'pins': [irisPin()] }),
  f('fieldBlur', '场景模糊', '模糊锐化', [], { 'pins': [fieldPin()] }),
  f('spinBlur', '旋转模糊', '模糊锐化', [], { 'pins': [spinPin()] }),
  f('pathBlur', '路径模糊', '模糊锐化', [], { 'paths': [blurPath()] }),

  // ── 杂色 / 修复 ────────────────────────────────────────────────────────
  f('addNoise', '添加杂色', '杂色修复', [p('amount', '数量', 0, 100, 1, 12)], { 'distribution': 'gaussian', 'monochromatic': true, 'seed': 12345 }),
  f('median', '中间值', '杂色修复', [p('radius', '半径', 1, 20, 1, 3)], {}),
  f('dustAndScratches', '去斑', '杂色修复', [p('radius', '半径', 1, 20, 1, 4), p('threshold', '阈值', 0, 255, 1, 20)], {}),
  f('reduceNoise', '减少杂色', '杂色修复', [p('strength', '强度', 0, 10, 0.1, 5), p('preserveDetails', '保留细节', 0, 100, 1, 50), p('reduceColorNoise', '减少彩噪', 0, 100, 1, 50), p('sharpenDetails', '锐化细节', 0, 100, 1, 20)], { 'removeJpegArtifact': false }),
  f('minimum', '最小值', '杂色修复', [p('radius', '半径', 1, 20, 1, 2)], { 'preserve': 'squareness' }),
  f('maximum', '最大值', '杂色修复', [p('radius', '半径', 1, 20, 1, 2)], { 'preserve': 'squareness' }),
  f('deInterlace', '去隔行', '杂色修复', [], { 'eliminateEven': false, 'interpolate': true }),
  f('ntscColors', 'NTSC 颜色', '杂色修复', [], {}),

  // ── 扭曲 / 变形 ────────────────────────────────────────────────────────
  f('twirl', '旋转扭曲', '扭曲变形', [p('angle', '角度', -999, 999, 1, 30)], {}),
  f('pinch', '挤捏', '扭曲变形', [p('amount', '数量', -100, 100, 1, 50)], {}),
  f('spherize', '球面化', '扭曲变形', [p('amount', '数量', -100, 100, 1, 50)], { 'mode': 'normal' }),
  f('wave', '波浪', '扭曲变形', [p('generators', '生成器', 1, 10, 1, 1), p('wavelengthMin', '最小波长', 10, 200, 1, 50), p('wavelengthMax', '最大波长', 10, 200, 1, 100), p('amplitudeMin', '最小振幅', 0, 50, 1, 10), p('amplitudeMax', '最大振幅', 0, 50, 1, 20)], { 'waveType': 'sine', 'undefined': 'wrap', 'seed': 12345 }),
  f('ripple', '涟漪', '扭曲变形', [p('amount', '数量', -999, 999, 1, 100)], { 'size': 'medium' }),
  f('polarCoordinates', '极坐标', '扭曲变形', [], { 'mode': 'rectangularToPolar' }),
  f('offset', '偏移', '扭曲变形', [p('horizontal', '水平', -100, 100, 1, 16), p('vertical', '垂直', -100, 100, 1, 16)], { 'undefined': 'wrap' }),
  f('displace', '置换', '扭曲变形', [p('horizontal', '水平', -100, 100, 1, 20), p('vertical', '垂直', -100, 100, 1, 20)], { 'stretch': false, 'undefined': 'wrap' }),
  f('shear', '切变', '扭曲变形', [], { 'points': [[0.25, 0], [0.75, 0]], 'undefined': 'wrap' }),
  f('zigZag', '水波', '扭曲变形', [p('amount', '数量', -100, 100, 1, 30), p('ridges', '波纹', 0, 20, 1, 5)], { 'style': 'pondRipples' }),

  // ── 像素化 / 风格化 ────────────────────────────────────────────────────
  f('mosaic', '马赛克', '像素风格', [p('cellSize', '单元格', 2, 100, 1, 12)], {}),
  f('colorHalftone', '彩色半调', '像素风格', [p('maxRadius', '最大半径', 4, 127, 1, 8)], { 'angles': [15, 75, 15, 45] }),
  f('crystallize', '晶格化', '像素风格', [p('cellSize', '单元格', 3, 300, 1, 12)], { 'seed': 12345 }),
  f('facet', '彩块化', '像素风格', [], {}),
  f('fragment', '碎片', '像素风格', [], {}),
  f('mezzotint', '铜版雕刻', '像素风格', [], { 'kind': 'fineDots', 'seed': 12345 }),
  f('pointillize', '点状化', '像素风格', [p('cellSize', '单元格', 3, 300, 1, 12)], { 'seed': 12345, 'background': [1, 1, 1, 1] }),
  f('diffuse', '扩散', '像素风格', [], { 'mode': 'normal', 'seed': 12345 }),
  f('extrude', '挤出', '像素风格', [p('size', '大小', 2, 100, 1, 10), p('depth', '深度', 1, 100, 1, 20)], { 'kind': 'blocks', 'levelBased': false, 'solidFront': false, 'maskIncomplete': false, 'seed': 12345 }),
  f('oilPaint', '油画', '像素风格', [p('stylization', '风格化', 0.1, 10, 0.1, 3), p('cleanliness', '清洁度', 0, 10, 0.1, 4), p('scale', '缩放', 0.1, 10, 0.1, 3), p('bristleDetail', '笔刷细节', 0, 10, 0.1, 3)], { 'lighting': true, 'angle': 135, 'shine': 3 }),
  f('tiles', '拼贴', '像素风格', [p('count', '数量', 1, 99, 1, 8), p('maxOffset', '最大位移', 1, 99, 1, 10)], { 'fill': 'background', 'foreground': [0, 0, 0, 1], 'background': [1, 1, 1, 1], 'seed': 12345 }),
  f('traceContour', '描边轮廓', '像素风格', [p('level', '色阶', 0, 255, 1, 128)], { 'upper': false }),
  f('wind', '风', '像素风格', [], { 'method': 'wind', 'fromRight': true, 'seed': 12345 }),
  f('emboss', '浮雕', '像素风格', [p('angle', '角度', -180, 180, 1, 135), p('height', '高度', 1, 20, 1, 3), p('amount', '数量', 1, 500, 1, 100)], {}),
  f('findEdges', '查找边缘', '像素风格', [], {}),
  f('solarize', '曝光过度', '像素风格', [], {}),
  f('invert', '反相', '像素风格', [], {}),
  f('desaturate', '去色', '像素风格', [], {}),

  // ── 艺术 / 渲染 ────────────────────────────────────────────────────────
  f('clouds', '云彩', '艺术渲染', [], { 'seed': 12345, 'difference': false, 'foreground': [0, 0, 0, 1], 'background': [1, 1, 1, 1] }),
  f('fibers', '纤维', '艺术渲染', [p('variance', '差异', 1, 64, 1, 16), p('strength', '强度', 1, 64, 1, 16)], { 'seed': 12345, 'foreground': [0, 0, 0, 1], 'background': [1, 1, 1, 1] }),
  f('lensFlare', '镜头光晕', '艺术渲染', [p('brightness', '亮度', 10, 300, 1, 100), p('centerX', '中心X', 0, 1, 0.01, 0.5), p('centerY', '中心Y', 0, 1, 0.01, 0.5)], { 'lens': 'zoom' }),
  f('lightingEffects', '光照效果', '艺术渲染', [p('gloss', '光泽', 0, 100, 1, 0), p('exposure', '曝光', -100, 100, 1, 0), p('height', '凸凹', 0, 100, 1, 0)], {
    'lights': [spotLight()],
    'metallic': 0, 'ambience': 0, 'texture': 'none', 'whiteIsHigh': false
  }),
  f('relight', '专业补光', '艺术渲染', [p('angle', '角度', -180, 180, 1, 45), p('elevation', '高度', 0, 90, 1, 45), p('intensity', '强度', 0, 100, 1, 60), p('ambient', '环境', 0, 100, 1, 30), p('warmth', '色温', -100, 100, 1, 0), p('softness', '柔和', 1, 100, 1, 40)], {}),
  f('filterGallery', '滤镜画廊', '艺术渲染', [], {
    'effects': [watercolorEffect()]
  }),

  // ── 其他 ───────────────────────────────────────────────────────────────
  f('custom', '自定义卷积', '其他滤镜', [p('scale', '缩放', 0.1, 10, 0.1, 1), p('offset', '偏移', -255, 255, 1, 0)], {
    'kernel': [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
  }),
  f('hsbHsl', 'HSB/HSL 转换', '其他滤镜', [], { 'input': 'rgb', 'output': 'hsb' }),
  f('colorToAlpha', '颜色转透明', '其他滤镜', [p('transparencyThreshold', '透明阈值', 0, 1, 0.01, 0.1), p('opacityThreshold', '不透明阈值', 0, 1, 0.01, 0.9)], { 'color': [1, 1, 1, 1] })
];

/** 按 id 找目录条目；找不到时回退到第一项，UI 不会因脏状态崩溃。 */
export function findFilter(id: string): FilterDef {
  for (const def of FILTERS) {
    if (def.id === id) {
      return def;
    }
  }
  return FILTERS[0];
}

/** 当前分组的滤镜列表。 */
export function filtersInGroup(group: string): FilterDef[] {
  const out: FilterDef[] = [];
  for (const def of FILTERS) {
    if (def.group === group) {
      out.push(def);
    }
  }
  return out;
}

/**
 * 生成 `pc_filter_apply` 的 JSON。数值参数从 UI 状态覆盖并夹到合法范围；
 * 枚举/颜色/结构参数使用目录中已验证默认值。
 */
export function buildFilterArgs(def: FilterDef, values: Map<string, number>): string {
  const payload: Record<string, Object> = { 'filter': def.id };
  for (const param of def.params) {
    const stored: number | undefined = values.get(param.key);
    const raw: number = stored === undefined ? param.value : stored;
    payload[param.key] = Math.min(param.max, Math.max(param.min, raw));
  }
  for (const key of Object.keys(def.fixed)) {
    payload[key] = def.fixed[key];
  }
  return JSON.stringify(payload);
}
