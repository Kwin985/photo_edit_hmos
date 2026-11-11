import { image } from '@kit.ImageKit';
import { picker, fileIo } from '@kit.CoreFileKit';
import { font, promptAction } from '@kit.ArkUI';
import { hilog } from '@kit.PerformanceAnalysisKit';
import { preferences } from '@kit.ArkData';
import { Engine, DocumentInfo, LayerInfo, AdjustmentInfo, SelectionInfo, emptyAdjustment, flattenLayers, LayerEffectInfo, LayerEffectsInfo, FxGradientInfo, FxGradientStop, LayerEditInfo, LayerMaskInfo, GuidesInfo } from '../engine/Engine';

const COLOR_ROOT: string = '#0a0b0d';
const COLOR_CANVAS: string = '#101216';
const COLOR_BAR: string = '#14161b';
const COLOR_TEXT: string = '#eceff4';
const COLOR_MUTED: string = '#8b95a4';

const COLOR_LAYER_CARD: string = '#18232f';
const COLOR_LAYER_ACCENT: string = '#4aa3ff';
const COLOR_ADJUST_CARD: string = '#221d30';
const COLOR_ADJUST_ACCENT: string = '#a98bfa';
const COLOR_EXPORT_CARD: string = '#17251e';
const COLOR_EXPORT_ACCENT: string = '#48c78e';

const COLOR_BRUSH_CARD: string = '#2a1d10';
const COLOR_BRUSH_ACCENT: string = '#f59e0b';
const COLOR_SELECT_CARD: string = '#0b2b28';
const COLOR_SELECT_ACCENT: string = '#2dd4bf';
const COLOR_FILTER_CARD: string = '#2b1220';
const COLOR_FILTER_ACCENT: string = '#ec4899';
const COLOR_SHAPE_CARD: string = '#181639';
const COLOR_SHAPE_ACCENT: string = '#6366f1';
const COLOR_TYPE_CARD: string = '#1b2a10';
const COLOR_TYPE_ACCENT: string = '#84cc16';
const BRUSH_COLORS: string[] = ['#ffffff', '#111827', '#e5484d', '#f5a524', '#30a46c', '#3b82f6', '#8b5cf6'];

// 首选项存储（R2）：单实例名 + 四个键，读写全部走同步 API 并静默失败。
const PREFS_NAME: string = 'photocraft_prefs';
const PREF_KEY_RULERS: string = 'rulers';
const PREF_KEY_GRID_SIZE: string = 'gridSize';
const PREF_KEY_GUIDES: string = 'guides';
const PREF_KEY_DEFAULT_ZOOM: string = 'defaultZoom';

const PERF_DOMAIN: number = 0x0A01;
const PERF_TAG: string = 'PERF';
const MIN_ZOOM: number = 0.1;
const MAX_ZOOM: number = 16;
const DETAIL_DELAY_MS: number = 140;
const MAX_DETAIL_PIXELS: number = 8000000;
const DETAIL_MARGIN: number = 0.12;
// detail 渲染区域的网格对齐粒度（见 renderDetail）：区域边界对齐到该格后，
// 平移/缩放不足一格时缓存键保持不变，可直接命中已缓存的像素字节。
const DETAIL_TILE: number = 256;
// 左上角悬浮缩放工具条（.position({ x: 12, y: 12 })）的命中矩形：
// 该区域内的触摸不交给画布，否则画笔工具下点「适应」「1:1」会误落一笔并让文档版本号递增。
const ZOOM_BAR_RIGHT: number = 330;
const ZOOM_BAR_BOTTOM: number = 72;
// 交互式裁剪手柄（C-CROPMODE-UI@1）：手柄视觉边长 / 命中区边长 / 框最小屏幕边长。
const CROP_HANDLE_VP: number = 12;
const CROP_HANDLE_HIT_VP: number = 24;
const CROP_MIN_VP: number = 32;
// 手柄中心相对框边界的内移量（P2 缺陷修复）：初始框贴文档 0 边界时，左边缘手柄中心
// 恰好落在系统侧滑返回手势区，触摸被系统吞掉。内移量取命中区边长 24vp，
// 使手柄中心 = 边界 + 24vp（以 2x 密度计约 48px），远离边缘手势触发点。
// 注意：只影响手柄的绘制位置与命中热区，框线/遮罩/尺寸标签仍按真实框计算。
const CROP_HANDLE_INSET_VP: number = 24;
// 框外遮罩与框体强调色（CROP_ACCENT 与 COLOR_LAYER_ACCENT 同值，视觉上归入「画布」分组）。
const CROP_MASK_COLOR: string = 'rgba(0, 0, 0, 0.45)';
const CROP_ACCENT: string = '#4aa3ff';
// 视图辅助（V2）：网格与参考线都是 1px 叠加层，只画视口可见区。
const GRID_LINE_COLOR: string = 'rgba(255, 255, 255, 0.18)';
// 网格屏幕间距下限：缩放较小时线会密到糊成一片且拖慢重绘，低于该值直接跳过绘制。
const GRID_MIN_SCREEN: number = 8;
const GUIDE_LINE_COLOR: string = '#22d3ee';
// 标尺刻度条：上/左两条尺带，Canvas 2D 绘制单位为 vp（与 viewportW/H 同一坐标系）。
const RULER_SIZE_VP: number = 20;
const RULER_BG: string = 'rgba(18, 20, 24, 0.92)';
const RULER_TICK: string = '#8b98a8';
const RULER_TEXT: string = '#a9b7c6';
// 主刻度屏幕间距下限：低于该值时相邻数字会挤在一起，需升到下一档步长。
const RULER_MIN_SPACING: number = 60;

interface AdjustPatch {
  brightness?: number;
  contrast?: number;
  exposure?: number;
  offset?: number;
  gamma?: number;
  vibrance?: number;
  saturation?: number;
  hue?: number;
  lightness?: number;
  levels?: number;
  level?: number;
}

// 图层二次编辑 apply 请求参数（C-EDIT-UI@1 §A）：键名与 C-EDIT-API@1 的 EditParams 逐字一致；
// 未出现的键不覆盖引擎原值。全部字段可选，因此"只改一项"的提交就是只带一个键。
interface EditPatch {
  text?: string;
  fontFamily?: string;
  size?: number;
  color?: number[];
  fillColor?: number[];
  strokeColor?: number[];
  strokeWidth?: number;
  dx?: number;
  dy?: number;
}

// 视图辅助（V2）请求参数：与 pc_guides_set 的请求键逐字一致，未出现的键不改引擎原值。
interface GuidesPatch {
  horizontal?: number[];
  vertical?: number[];
  origin?: number[];
}

// guidesGet / guidesSet 的返回结构：水平/竖直参考线是文档像素坐标，origin 是标尺原点。
// 注意：必须直接使用 Engine 导出的 GuidesInfo，不要另声明同构接口——ArkTS 禁止结构化类型
// （arkts-no-structural-typing），本地同构接口与引擎返回类型互不相容。

interface BrushPoint {
  x: number;
  y: number;
  pressure: number;
  time: number;
}

interface BrushParams {
  size: number;
  hardness: number;
  opacity: number;
  flow: number;
  spacing: number;
  pressureSize: boolean;
  erase: boolean;
  color: number[];
}

interface SelectParams {
  kind: string;
  x0: number;
  y0: number;
  x1: number;
  y1: number;
  mode: string;
  feather: number;
  points: number[][];
  x: number;
  y: number;
  tolerance: number;
  contiguous: boolean;
}

interface FilterArgs {
  filter: string;
  radius: number;
  angle: number;
  distance: number;
  amount: number;
  cellSize: number;
  height: number;
  reduceNoise: number;
  distribution: string;
  monochromatic: boolean;
  seed: number;
}

interface ShapeParams {
  kind: string;
  x: number;
  y: number;
  w: number;
  h: number;
  fromX: number;
  fromY: number;
  toX: number;
  toY: number;
  weight: number;
  radius: number;
  fill: number[];
  stroke: number[];
  strokeWidth: number;
  name: string;
}

interface TextParams {
  text: string;
  fontFamily: string;
  size: number;
  x: number;
  y: number;
  color: number[];
  name: string;
}

interface BlendOption {
  label: string;
  value: string;
}

// 图层样式 apply 请求参数：显式接口，避免 ArkTS 严格模式下的无类型对象字面量与动态索引赋值。
// 键名与 C-EFFECTS-API@2 的 apply 请求表逐字一致；未出现在请求中的键不会覆盖引擎原值（R6.4）。
interface FxUpdateParams {
  op: string;
  kind?: string;
  index?: number;
  enabled?: boolean;
  blend?: string;
  opacity?: number;
  color?: number[];
  angle?: number;
  distance?: number;
  spread?: number;
  size?: number;
  position?: string;
  technique?: string;
  knocksOut?: boolean;
  antiAlias?: boolean;
  // 内发光源（edge/center）
  source?: string;
  // 渐变叠加：stops 必须整体提交（首尾改值、中间项原样保留）
  gradientStops?: FxGradientStop[];
  gradientStyle?: string;
  gradientScale?: number;
  reverse?: boolean;
  align?: boolean;
  dither?: boolean;
  // 斜面与浮雕
  bevelStyle?: string;
  bevelTechnique?: string;
  depth?: number;
  up?: boolean;
  soften?: number;
  altitude?: number;
  highlightColor?: number[];
  highlightOpacity?: number;
  shadowColor?: number[];
  shadowOpacity?: number;
}


const FX_CARD_BG: string = '#1b2836';
const FX_GROUP_BG: string = '#22303f';
const FX_GROUP_ACTIVE_BG: string = '#2a3d55';
const FX_TEXT: string = '#dcecff';
const FX_TEXT_MUTED: string = '#9fb3c8';
const FX_ENABLED: string = '#7ee787';
const FX_DISABLED: string = '#55606d';
// 新增四组的分组强调色（C-FX-UI@2 视觉值表）；既有三组色点（#6ea8ff/#5ad1c4/#f7b955）保持不变。
const FX_ACCENT_INNER_SHADOW: string = '#ff7a45';
const FX_ACCENT_INNER_GLOW: string = '#ff6ec7';
const FX_ACCENT_GRADIENT: string = '#b9a4ff';
const FX_ACCENT_BEVEL: string = '#d9e2ec';

// 图层蒙版（C-MASK-UI@1 §E）：黑/白通道语义的中性浅色系，只作 UI 主题色，
// 不参与任何图像像素写入（颜色通道仍为既有 BGR 直通）。
const COLOR_MASK_CARD: string = '#1c2027';
const COLOR_MASK_GROUP: string = '#262b33';
const COLOR_MASK_ACCENT: string = '#e2e8f0';
const COLOR_MASK_TEXT: string = '#f1f5f9';
const COLOR_MASK_MUTED: string = '#9aa4b2';
const COLOR_MASK_DANGER: string = '#ff8b8b';

const BLEND_OPTIONS: BlendOption[] = [
  { label: '正常', value: 'Normal' },
  { label: '正片叠底', value: 'Multiply' },
  { label: '滤色', value: 'Screen' },
  { label: '叠加', value: 'Overlay' },
  { label: '变暗', value: 'Darken' },
  { label: '变亮', value: 'Lighten' },
  { label: '颜色加深', value: 'ColorBurn' },
  { label: '颜色减淡', value: 'ColorDodge' },
  { label: '柔光', value: 'SoftLight' },
  { label: '强光', value: 'HardLight' },
  { label: '差值', value: 'Difference' },
  { label: '排除', value: 'Exclusion' },
  { label: '色相', value: 'Hue' },
  { label: '饱和度', value: 'Saturation' },
  { label: '颜色', value: 'Color' },
  { label: '明度', value: 'Luminosity' }
];

@Entry
@Component
struct Index {
  @State docId: number = 0;
  @State info: DocumentInfo | undefined = undefined;
  @State layers: LayerInfo[] = [];
  @State selectedLayerId: number = 0;
  @State pixelMap: image.PixelMap | undefined = undefined;
  @State adjust: AdjustmentInfo = emptyAdjustment();
  @State zoom: number = 1;
  @State panX: number = 0;
  @State panY: number = 0;
  @State viewportW: number = 0;
  @State viewportH: number = 0;
  @State usingDetail: boolean = false;
  @State regionX: number = 0;
  @State regionY: number = 0;
  @State regionW: number = 0;
  @State regionH: number = 0;
  @State previewPixelW: number = 0;
  @State previewPixelH: number = 0;
  @State busy: boolean = false;
  @State status: string = '打开一张图片开始编辑';
  @State isWide: boolean = false;
  @State exportFormat: string = 'png';
  @State exportQuality: number = 90;
  @State layerOpacity: number = 100;
  @State layerFillOpacity: number = 100;
  @State layerBlend: string = 'Normal';
  @State layerNameDraft: string = '';
  @State layerThumb: image.PixelMap | undefined = undefined;
  @State layerThumbVersion: number = 0;
  @State tool: string = 'pan';
  @State brushSize: number = 24;
  @State brushHardness: number = 80;
  @State brushOpacity: number = 100;
  @State brushColor: string = '#e5484d';
  @State selectionKind: string = 'rect';
  @State selectionMode: string = 'replace';
  @State selectionFeather: number = 0;
  @State wandTolerance: number = 24;
  @State selectionActive: boolean = false;
  @State selX0: number = 0;
  @State selY0: number = 0;
  @State selX1: number = 0;
  @State selY1: number = 0;
  @State selectionShape: string = 'bounds';
  @State filterKind: string = 'gaussianBlur';
  @State filterBlurRadius: number = 4;
  @State filterMotionAngle: number = 0;
  @State filterMotionDistance: number = 20;
  @State filterSharpenAmount: number = 100;
  @State filterSharpenRadius: number = 2;
  @State filterNoiseAmount: number = 12;
  @State filterMono: boolean = true;
  @State filterCellSize: number = 12;
  @State filterEmbossAngle: number = 135;
  @State filterEmbossHeight: number = 3;
  @State filterEmbossAmount: number = 100;
  @State filterTwirlAngle: number = 30;
  @State shapeKind: string = 'rect';
  @State shapeFillColor: string = '#e5484d';
  @State shapeFillOpacity: number = 100;
  @State shapeStrokeColor: string = '#ffffff';
  @State shapeStrokeWidth: number = 0;
  @State shapeRadius: number = 0;
  @State shapeLineWidth: number = 12;
  @State textContent: string = 'PhotoCraft';
  @State textSize: number = 48;
  @State textColor: string = '#ffffff';
  @State textFontFamily: string = 'Inter';
  @State textFontReady: boolean = false;
  @State textFontName: string = '';
  // 图层样式（FX）：数据直接由组件状态驱动，Builder 内一律读 this.fxInfo / this.fxKind，不靠参数刷新。
  @State fxInfo: LayerEffectsInfo = { enabled: true, items: [] };
  @State fxOpen: boolean = false;
  @State fxKind: string = 'dropShadow';
  // 滑块草稿：拖动过程中只更新这些状态，End/Click 时才提交引擎。
  @State fxOpacity: number = 75;
  @State fxAngle: number = 120;
  @State fxDistance: number = 5;
  @State fxBlur: number = 5;
  @State fxSpread: number = 0;
  @State fxWidth: number = 3;
  @State fxPosition: string = 'outside';
  @State fxColor: string = '#111827';
  // 新增四组（内阴影 / 内发光 / 渐变叠加 / 斜面与浮雕）的草稿：全部为标量。
  // 读写一律走显式 if/else if 分支（fxSyncDraft / fxDraftValue / fxOnSlider / fxAssign），禁止动态索引赋值。
  @State fxGradStartColor: string = '#000000';
  @State fxGradEndColor: string = '#ffffff';
  @State fxGradStyle: string = 'linear';
  @State fxGradScale: number = 100;
  @State fxGradReverse: boolean = false;
  @State fxGradAlign: boolean = true;
  @State fxGradDither: boolean = false;
  @State fxGlowSource: string = 'edge';
  @State fxBevelStyle: string = 'innerBevel';
  @State fxBevelTechnique: string = 'smooth';
  @State fxBevelDepth: number = 100;
  @State fxBevelUp: boolean = true;
  @State fxBevelSoften: number = 0;
  @State fxBevelAltitude: number = 30;
  @State fxBevelHighlightColor: string = '#ffffff';
  @State fxBevelHighlightOpacity: number = 75;
  @State fxBevelShadowColor: string = '#000000';
  @State fxBevelShadowOpacity: number = 75;
  // 图层蒙版（C-MASK-UI@1 §B）：数据由 loadMask 回填，MaskCard 内一律直接读 this.*，不用形参传状态。
  @State maskInfo: LayerMaskInfo = { present: false, enabled: true, linked: true, density: 1, feather: 0 };
  @State maskOpen: boolean = true;
  @State maskDensity: number = 100;   // 0..100 草稿（提交时 /100）
  @State maskFeather: number = 0;     // 0..250 草稿（直接是像素）
  // 图层二次编辑（C-EDIT-UI@1 §A）：分组类型 '' | 'Type' | 'Shape'，由 loadEdit 从原生层读取后决定。
  // 这些状态只由 loadEdit / applyEdit 回填与分组内控件草稿写入；Builder 内一律直接读 this.*。
  @State editKind: string = '';
  @State editText: string = '';
  @State editSize: number = 48;
  @State editFontFamily: string = 'Inter';
  @State editColor: string = '#ffffff';
  @State editShapeKind: string = '';
  @State editFillColor: string = '#e5484d';
  @State editFillEditable: boolean = true;
  @State editStrokeColor: string = '#ffffff';
  @State editStrokeEditable: boolean = true;
  @State editStrokeWidth: number = 0;
  @State editPosX: number = 0;
  @State editPosY: number = 0;
  // 图层自由变换（C-XFORM-UI@1 §3.2）：折叠态 + 5 个滑块草稿（缩放按百分比显示，提交时 /100）。
  // 滑块 onChange 只写这些草稿，「应用」按钮统一提交一次（避免 5 个控件各触发一次 checkpoint）。
  @State trOpen: boolean = false;
  @State trScaleX: number = 100;
  @State trScaleY: number = 100;
  @State trAngle: number = 0;
  @State trDx: number = 0;
  @State trDy: number = 0;
  // 图层多选（S1）：multiSelectMode 是模式开关；multiSelected 保存被勾选的图层 id，
  // 保持点击顺序且去重（批量操作按该顺序执行，状态栏文案里的 N 也来自该列表）。
  @State multiSelectMode: boolean = false;
  @State multiSelected: number[] = [];
  // 画布分组（C-CANVAS-UI@1 §B）：折叠态、宽高草稿、9 宫格锚点（4 = 中心）。
  // 全部为 UI 私有状态：点击折叠行/宫格只改本地状态，不调用引擎、不 bump revision。
  @State canvasOpen: boolean = false;
  @State canvasW: string = '';
  @State canvasH: string = '';
  @State canvasAnchor: number = 4;
  // 视图辅助（V2）：网格/参考线全是叠加层，开关只改本地状态并重绘叠加层，零引擎调用；
  // guidesH/guidesV 是文档像素坐标，由 syncGuides / applyGuides 从 pc_guides_get 回填。
  @State viewOpen: boolean = true;
  @State showGrid: boolean = false;
  @State showGuides: boolean = true;
  @State showRulers: boolean = false;
  @State gridSize: number = 64;
  @State guidesH: number[] = [];
  @State guidesV: number[] = [];
  // 首选项（R2）：prefsOpen 只控制折叠；defaultZoom 是下次打开文档时的初始缩放模式
  // （'fit' 适应 / 'oneToOne' 1:1），改动只落盘、不立即改变当前视图。
  @State prefsOpen: boolean = false;
  @State defaultZoom: string = 'fit';
  // 新建文档草稿（file.new）：只存 UI 输入，点击「新建文档」才提交引擎。
  @State newDocW: string = '2400';
  @State newDocH: string = '1500';
  // 交互式裁剪手柄（C-CROPMODE-UI@1 §1）：cropMode 是裁剪态的唯一判定量；
  // 四个边界为文档坐标（浮点，x1/y1 独占语义，与 Document::crop_to 一致），
  // 必须是 @State：框尺寸标签与「应用」按钮启用态依赖它们重组。
  @State cropMode: boolean = false;
  @State cropX0: number = 0;
  @State cropY0: number = 0;
  @State cropX1: number = 0;
  @State cropY1: number = 0;
  // 画布变换手势（C-TRANSFORM-GESTURE@1 §1）：trMode 是变换态的唯一判定量；
  // 标签与入口按钮的启用态依赖它重组，其余拖拽态一律普通字段。
  @State trMode: boolean = false;
  // 历史记录面板（只读）：标签来自引擎侧 undo/redo 栈，最近一步在前。
  @State historyOpen: boolean = false;
  @State historyUndo: string[] = [];
  @State historyRedo: string[] = [];

  private brushSettings: RenderingContextSettings = new RenderingContextSettings(true);
  private brushCtx: CanvasRenderingContext2D = new CanvasRenderingContext2D(this.brushSettings);
  private strokePoints: BrushPoint[] = [];
  private strokeStartMs: number = 0;
  private drawing: boolean = false;
  private previewDot: boolean = false;
  private lastScreenX: number = 0;
  private lastScreenY: number = 0;
  private selecting: boolean = false;
  private selectStartScreenX: number = 0;
  private selectStartScreenY: number = 0;
  private selectCurrentScreenX: number = 0;
  private selectCurrentScreenY: number = 0;
  private lassoPoints: number[][] = [];
  private selectionPathDoc: number[][] = [];
  private selectionKey: string = '';
  private shaping: boolean = false;
  private shapeStartScreenX: number = 0;
  private shapeStartScreenY: number = 0;
  private shapeCurrentScreenX: number = 0;
  private shapeCurrentScreenY: number = 0;

  private renderTimer: number = -1;
  private renderToken: number = 0;
  // 渲染调度与缓存度量：全部为普通字段（非 @State），避免计数变化触发 UI 重组。
  private docRevision: number = 0;
  private detailKey: string = '';
  private previewKey: string = '';
  private renderCalls: number = 0;
  private detailHits: number = 0;
  private previewCalls: number = 0;
  private previewHits: number = 0;
  private editCalls: number = 0;
  private gestureUpdates: number = 0;
  // 细节/预览像素缓存（独立所有权槽）。缓存的是“像素字节”而不是 PixelMap 对象（R8.1）：
  // image.createPixelMap(pixels, options) 与传入的 ArrayBuffer 是零拷贝共享关系，PixelMap 内部只记住裸指针；
  // 该 ArrayBuffer 由 napi_create_arraybuffer 分配、归 ArkTS 虚拟机 GC 管理（见 napi_init.cpp MakeArrayBuffer），
  // 一旦 ArkTS 侧不再引用它而被回收，已创建 PixelMap 的内容就会失效——实测表现为“缓存命中后整体变暗”。
  // 因此这里用普通字段长期强引用像素字节，命中时用同一份字节重建一张新的 PixelMap 再显示。
  // 内存代价：detail 最多 MAX_DETAIL_PIXELS * 4 B（8M 像素 ≈ 32MB），preview 最多约 16MB（2048² × 4 B）。
  private detailPixels: ArrayBuffer | undefined = undefined;
  private previewPixels: ArrayBuffer | undefined = undefined;
  // 当前显示 pixelMap 的像素源引用：保证“正在显示”的那张图背后的字节不被回收。
  // 它与上面两个缓存槽解耦，因此缓存失效（revision 变化）时可以先清缓存槽，
  // 仍能正确显示旧画面，直到新像素替换 pixelMap 为止。
  private displayPixels: ArrayBuffer | undefined = undefined;
  // 命中路径字节级自检开关：默认关闭。开启后命中时用 Engine.renderPixels 取一次真值做逐字节比对，
  // 用于区分“缓存内容错”与“绘制状态错”。该调用不参与 renderCalls 计数，也不创建 PixelMap。
  private debugCacheCheck: boolean = false;
  // 本次触摸手势是否起始于左上角悬浮缩放工具条。起始于工具条时整段手势不交给画布，
  // 否则画笔工具下点「适应」「1:1」会穿透画布落下一笔（文档 revision 随之 +1）。
  private gestureInToolbar: boolean = false;
  // 画布叠加层当前是否画有选区框。无选区时 redrawCanvas 直接返回，避免每次平移/缩放都做一次全屏 clearRect。
  private selectionFrameDrawn: boolean = false;
  // 选区/形状拖拽预览的上一帧脏矩形（屏幕 vp）：只清这一块，避免每次 Move 全屏 clearRect。
  private previewDirtyX: number = 0;
  private previewDirtyY: number = 0;
  private previewDirtyW: number = 0;
  private previewDirtyH: number = 0;
  private thumbToken: number = 0;
  private thumbTimer: number = -1;
  private thumbQueueToken: number = 0;
  private layerThumbCache: Map<number, image.PixelMap> = new Map<number, image.PixelMap>();
  private thumbPending: Set<number> = new Set<number>();
  private pinchActive: boolean = false;
  private pinchStartZoom: number = 1;
  private pinchStartPanX: number = 0;
  private pinchStartPanY: number = 0;
  private pinchFocusX: number = 0;
  private pinchFocusY: number = 0;
  private panStartX: number = 0;
  private panStartY: number = 0;
  private textMultiTouch: boolean = false;
  private textTapX: number = 0;
  private textTapY: number = 0;
  // 移动工具（tool='move'）：全部为普通字段，拖动期间只写这些字段与 status，不触发任何渲染（R3.2 / R8.1）。
  private movingLayer: boolean = false;
  private moveStartDocX: number = 0;
  private moveStartDocY: number = 0;
  private moveStartScreenX: number = 0;
  private moveStartScreenY: number = 0;
  private moveCurrentScreenX: number = 0;
  private moveCurrentScreenY: number = 0;
  private moveDx: number = 0;
  private moveDy: number = 0;
  // 滑块"未改变"判定的基线值（从原生层读回的真实值，普通字段，不参与重组）：
  // 用于 R1.3 / §1.3 示例 1 —— 拖回原值松手时不写文档、并在状态栏提示"未改变"。
  private editSizeBase: number = 48;
  private editStrokeWidthBase: number = 0;
  // 交互式裁剪手柄的拖拽态与视图快照（C-CROPMODE-UI@1 §1）：全部为普通字段，严禁 @State，
  // 避免每帧拖拽触发 UI 重组；拖拽期间只写这些字段与 @State 框坐标。
  private cropDragging: boolean = false;
  private cropDragKind: string = '';
  private cropDragStartDocX: number = 0;
  private cropDragStartDocY: number = 0;
  private cropDragOriginX0: number = 0;
  private cropDragOriginY0: number = 0;
  private cropDragOriginX1: number = 0;
  private cropDragOriginY1: number = 0;
  private cropViewZoom: number = 1;
  private cropViewPanX: number = 0;
  private cropViewPanY: number = 0;
  private cropFrameCount: number = 0;
  // 画布变换手势的拖拽态与预览值（C-TRANSFORM-GESTURE@1 §1）：拖拽态为普通字段，
  // trPreview* 为 @State（叠加层与实时标签需随拖拽每帧刷新），拖拽期间仍零引擎调用。
  // trPreview* 是相对图层当前状态的增量（与面板 5 个滑块同语义、同量纲：百分比 / 度 / 文档像素）。
  private trDragging: boolean = false;
  private trDragKind: string = '';
  private trStartDocX: number = 0;
  private trStartDocY: number = 0;
  private trOrigScaleX: number = 100;
  private trOrigScaleY: number = 100;
  private trOrigAngle: number = 0;
  private trOrigDx: number = 0;
  private trOrigDy: number = 0;
  @State private trPreviewScaleX: number = 100;
  @State private trPreviewScaleY: number = 100;
  @State private trPreviewAngle: number = 0;
  @State private trPreviewDx: number = 0;
  @State private trPreviewDy: number = 0;
  private trFrameCount: number = 0;

  // PERF 度量：固定 domain/tag，保证 `hdc shell hilog | findstr PERF` 可抓取（R11.4）。
  private perf(line: string): void {
    hilog.info(PERF_DOMAIN, PERF_TAG, '%{public}s', line);
  }

  // ===== 交互式裁剪手柄（C-CROPMODE-UI@1 §2，全部 private）=====
  // 红线：cropBeginDrag / cropUpdateDrag / cropEndDrag / drawCropOverlay 内零引擎调用、零 checkpoint（R2.4）；
  // 只有 cropApply 允许一次 documentCrop 写调用，只有 cropCancel 允许 refreshPreview/scheduleDetail（视图恢复）。

  // 启用条件：有文档且非忙（R1.2）。供 CanvasGroup 的「裁剪手柄」按钮使用。
  private cropEnabled(): boolean {
    return this.docId !== 0 && !this.busy;
  }

  // 进入裁剪模式（R1.1）：无文档/忙时直接 return（零副作用）。
  private cropEnter(): void {
    const info = this.info;
    if (!this.cropEnabled() || info === undefined) {
      return;
    }
    // ① 记录视图快照：取消时逐字恢复（R6.3，浮点精确回写）。
    this.cropViewZoom = this.zoom;
    this.cropViewPanX = this.panX;
    this.cropViewPanY = this.panY;
    // ② 结束进行中的其它拖拽，避免叠加态（画笔/选区/形状/移动）。
    this.cancelStroke();
    this.cancelSelect();
    this.cancelShape();
    this.cancelMove();
    // ③ 框初始化为整幅画布（文档坐标；x1/y1 为独占语义）。
    this.cropX0 = 0;
    this.cropY0 = 0;
    this.cropX1 = info.width;
    this.cropY1 = info.height;
    this.cropDragging = false;
    this.cropDragKind = '';
    this.cropFrameCount = 0;
    // ④ 打开裁剪模式（唯一判定裁剪态的布尔量，不依赖 tool）。
    this.cropMode = true;
    // ⑤ 状态栏提示。
    this.status = '裁剪：拖手柄调整，拖框内移动';
    // ⑥ 重绘叠加层。
    this.redrawCanvas();
  }

  // 退出并恢复视图（R1.4，用户点「取消」）：零引擎写调用。
  private cropCancel(): void {
    if (!this.cropMode) {
      return;
    }
    // 恢复进入时的视图快照（R6.3，浮点精确回写，无容差累积）。
    this.zoom = this.cropViewZoom;
    this.panX = this.cropViewPanX;
    this.panY = this.cropViewPanY;
    this.cropMode = false;
    this.cropDragging = false;
    this.cropDragKind = '';
    this.cropFrameCount = 0;
    // 退出后 redrawCanvas 走普通分支：必须先清掉叠加层，否则无选区时会早退、旧遮罩残留（R3.2）。
    this.clearBrushPreview();
    this.redrawCanvas();
    // 契约澄清 P1：恢复视图后按既有「适应」按钮同一路径刷新（只读渲染，不是引擎写接口）。
    // 目的：regionX/regionY 与 detail 像素仍停留在缩放/裁剪期间的状态，不刷新会显示过期画面。
    // 恢复状态栏（否则会停在「裁剪：拖手柄调整，拖框内移动」）。
    this.status = '已取消裁剪';
    // 刷新锚点日志：取消后若走「needsDetail 与 usingDetail 均为 false」的早退路径，
    // 不会产生 renderDetail/preview 行；靠这条 PERF 证明「取消确实触发过刷新路径」，便于验证取证。
    if (this.usingDetail) {
      this.perf('crop cancel refresh usingDetail=true');
      this.refreshPreview();
    } else {
      this.perf('crop cancel refresh usingDetail=false');
      this.scheduleDetail();
    }
  }

  // 退出但不恢复视图（R1.5：撤销/重做/重新打开文档）：保持操作后的 zoom/pan。
  private cropDiscard(): void {
    if (!this.cropMode) {
      return;
    }
    this.cropMode = false;
    this.cropDragging = false;
    this.cropDragKind = '';
    this.cropFrameCount = 0;
    // 同 cropCancel：走普通分支前先清叠加层，避免无选区时早退导致残留。
    this.clearBrushPreview();
    this.redrawCanvas();
  }

  // 应用（R4.1）：一次 checkpoint + 8 步刷新顺序。
  private async cropApply(): Promise<void> {
    if (this.docId === 0 || this.busy || !this.cropMode) {
      return;
    }
    // R4.5：框等于整幅画布 → 零引擎调用直接退出。
    if (this.cropIsFullCanvas()) {
      this.cropCancel();
      this.status = '框未变化 · 已退出裁剪';
      return;
    }
    // 取整：doc 层 x1/y1 独占；四舍五入后必须仍满足 x0<x1、y0<y1。
    const x0 = Math.round(this.cropX0);
    const y0 = Math.round(this.cropY0);
    const x1 = Math.round(this.cropX1);
    const y1 = Math.round(this.cropY1);
    if (x1 - x0 < 1 || y1 - y0 < 1) {
      this.status = '裁剪框太小，请拖大手柄';
      return; // 零引擎调用（R4.1 前置条件）
    }
    const before = this.info;
    this.busy = true;
    try {
      // 1. 调引擎：一次 checkpoint（既有 pc_document_crop 保证，本轮零改动）。
      const next = Engine.documentCrop(this.docId, x0, y0, x1, y1);
      // 2. changed 由 UI 比较宽高判定（与 canvasCropToSelection 一致）。
      const changed = before === undefined || next.width !== before.width || next.height !== before.height;
      // 3. 先退出裁剪模式并清叠加层：必须在 afterEdit 之前，否则刷新画面期间仍残留旧框
      //    （这是与 canvasCropToSelection 的唯一顺序差异）。
      this.cropMode = false;
      this.cropDragging = false;
      this.cropDragKind = '';
      this.cropFrameCount = 0;
      // 4. changed 时重置视图 + 显式清缩略图（缩略图缓存键不含 revision）。
      if (changed) {
        this.resetView();
        this.clearAllLayerThumbs();
      }
      // 5. 复用 afterEdit：比较 revision、拉 info、清 detailKey/detailPixels、重排图层、调度渲染。
      await this.afterEdit();
      // 6. 选区已被引擎裁到新画布：同步叠加层。
      this.syncSelection();
      // 7. 回填画布分组草稿。
      this.syncCanvasDraft();
      // 8. 状态栏。
      this.status = changed
        ? ('已裁剪 · ' + next.width + ' × ' + next.height)
        : '无需调整 · 尺寸未变化';
    } catch (err) {
      // R4.3：失败保持裁剪模式与框，只写状态栏（不 resetView、不清缓存、不改 docRevision）。
      this.status = '裁剪失败：' + this.messageOf(err as Object);
      this.cropMode = true;
    } finally {
      this.busy = false;
      // 退出路径必须先清叠加层（理由同 cropCancel）；失败时 cropMode=true 会重绘遮罩。
      this.clearBrushPreview();
      this.redrawCanvas();
    }
  }

  // 手柄命中（A5 优先级）：4 角 > 4 边中点 > 框内移动 > 框外空串。
  // 输入为 Stack 本地屏幕坐标（vp）；手柄中心统一取自 cropHandleCenters（与绘制同源，含内移），
  // 框内/框外判定仍用真实框边界（不含内移）。
  private cropHandleAt(screenX: number, screenY: number): string {
    const c = this.cropHandleCenters();
    const r = this.cropScreenRect();
    const half = this.cropTinyMode() ? Math.max(3, Math.min(CROP_HANDLE_HIT_VP / 2, Math.min(r[2], r[3]) / 8)) : CROP_HANDLE_HIT_VP / 2;
    const left = r[0];
    const top = r[1];
    const right = r[0] + r[2];
    const bottom = r[1] + r[3];
    // 1. 4 个角（先判，命中区重叠时角优先于边）。
    if (Math.abs(screenX - c[0]) <= half && Math.abs(screenY - c[1]) <= half) {
      return 'nw';
    }
    if (Math.abs(screenX - c[4]) <= half && Math.abs(screenY - c[5]) <= half) {
      return 'ne';
    }
    if (Math.abs(screenX - c[8]) <= half && Math.abs(screenY - c[9]) <= half) {
      return 'se';
    }
    if (Math.abs(screenX - c[12]) <= half && Math.abs(screenY - c[13]) <= half) {
      return 'sw';
    }
    // 2. 4 个边中点（tiny 框只保留角手柄，避免与角/中心重叠）。
    if (!this.cropTinyMode()) {
      if (Math.abs(screenX - c[2]) <= half && Math.abs(screenY - c[3]) <= half) {
        return 'n';
      }
      if (Math.abs(screenX - c[6]) <= half && Math.abs(screenY - c[7]) <= half) {
        return 'e';
      }
      if (Math.abs(screenX - c[10]) <= half && Math.abs(screenY - c[11]) <= half) {
        return 's';
      }
      if (Math.abs(screenX - c[14]) <= half && Math.abs(screenY - c[15]) <= half) {
        return 'w';
      }
    }
    // 3. 框内 → 整体移动；4. 框外 → 空串（onTouch 仍会消费该次触摸，R2.5）。
    // 内移后紧贴真实边界的最外侧窄带（0 ~ inset - 命中半宽 = 12vp）不再命中任何手柄，
    // 属预期：该带是系统侧滑返回手势区，用户拖手柄应落在中心（边界 + 24vp）附近。
    if (screenX > left && screenX < right && screenY > top && screenY < bottom) {
      return 'move';
    }
    return '';
  }

  // 开始拖拽（R2.1/R2.3）：kind 为空时不开始（cropDragging 保持 false）。
  private cropBeginDrag(screenX: number, screenY: number): void {
    if (!this.cropMode || this.info === undefined) {
      return;
    }
    // 幂等：上一次拖拽若丢了 Up，先复位再重新开始（R5.5）。
    if (this.cropDragging) {
      this.cropEndDrag();
    }
    const kind = this.cropHandleAt(screenX, screenY);
    this.cropDragKind = kind;
    this.cropFrameCount = 0;
    if (kind === '') {
      this.cropDragging = false;
      return;
    }
    this.cropDragging = true;
    // 起点与框原点快照：全部为文档坐标（screenToDocX/Y 内部已减 regionX/regionY）。
    this.cropDragStartDocX = this.screenToDocX(screenX);
    this.cropDragStartDocY = this.screenToDocY(screenY);
    this.cropDragOriginX0 = this.cropX0;
    this.cropDragOriginY0 = this.cropY0;
    this.cropDragOriginX1 = this.cropX1;
    this.cropDragOriginY1 = this.cropY1;
  }

  // 更新拖拽（R2.2/R2.4）：按 cropDragKind 重算框并 clamp，然后重绘叠加层；零引擎调用。
  private cropUpdateDrag(screenX: number, screenY: number): void {
    if (!this.cropMode || !this.cropDragging || this.cropDragKind === '' || this.info === undefined) {
      return;
    }
    const startedAt = Date.now();
    // dx/dy 为文档坐标增量：必须复用既有 screenToDocX/Y（内部折入 pan/zoom 与 regionX/regionY）。
    const dx = this.screenToDocX(screenX) - this.cropDragStartDocX;
    const dy = this.screenToDocY(screenY) - this.cropDragStartDocY;
    // 每帧都从拖拽起点快照重算，避免增量累积误差。
    this.cropX0 = this.cropDragOriginX0;
    this.cropY0 = this.cropDragOriginY0;
    this.cropX1 = this.cropDragOriginX1;
    this.cropY1 = this.cropDragOriginY1;
    // 边/角映射（契约第 2 段映射表逐行对应）：只改被拖动的那条边，对边保持快照值。
    const kind = this.cropDragKind;
    if (kind === 'nw') {
      this.cropX0 = this.cropDragOriginX0 + dx;
      this.cropY0 = this.cropDragOriginY0 + dy;
    } else if (kind === 'n') {
      this.cropY0 = this.cropDragOriginY0 + dy;
    } else if (kind === 'ne') {
      this.cropX1 = this.cropDragOriginX1 + dx;
      this.cropY0 = this.cropDragOriginY0 + dy;
    } else if (kind === 'e') {
      this.cropX1 = this.cropDragOriginX1 + dx;
    } else if (kind === 'se') {
      this.cropX1 = this.cropDragOriginX1 + dx;
      this.cropY1 = this.cropDragOriginY1 + dy;
    } else if (kind === 's') {
      this.cropY1 = this.cropDragOriginY1 + dy;
    } else if (kind === 'sw') {
      this.cropX0 = this.cropDragOriginX0 + dx;
      this.cropY1 = this.cropDragOriginY1 + dy;
    } else if (kind === 'w') {
      this.cropX0 = this.cropDragOriginX0 + dx;
    } else if (kind === 'move') {
      this.cropX0 = this.cropDragOriginX0 + dx;
      this.cropY0 = this.cropDragOriginY0 + dy;
      this.cropX1 = this.cropDragOriginX1 + dx;
      this.cropY1 = this.cropDragOriginY1 + dy;
    }
    this.cropClampRect();
    // 拖拽期间只重绘叠加层：不调用任何引擎/渲染/缓存接口，不产生检查点（R2.4）。
    this.redrawCanvas();
    this.cropFrameCount++;
    this.perf('cropDrag n=' + this.cropFrameCount + ' ms=' + (Date.now() - startedAt));
  }

  // 结束拖拽（R2）：只复位拖拽态；不写文档、不产生检查点。
  private cropEndDrag(): void {
    if (!this.cropDragging && this.cropDragKind === '') {
      return;
    }
    this.cropDragging = false;
    this.cropDragKind = '';
    this.cropFrameCount = 0;
    this.redrawCanvas();
  }

  // 幂等兜底复位（R5.5）：仅当 cropDragging === true 时执行 cropEndDrag 语义。
  private cropDragFallbackEnd(): void {
    if (!this.cropDragging) {
      return;
    }
    this.cropEndDrag();
  }

  // 夹取与规范化（R2.2）：不翻转、不越界、不小于 max(1, CROP_MIN_VP/displayScale())，
  // 且只向内侧推被拖动的那条边（不移动对边，否则用户会看到"另一条边也在动"）。
  private cropClampRect(): void {
    const info = this.info;
    if (info === undefined) {
      return;
    }
    const scale = this.displayScale();
    if (scale <= 0) {
      return;
    }
    if (info.width <= 0 || info.height <= 0) {
      return;
    }
    // 最小框：屏幕 32vp 对应的文档像素数，但不少于 1 像素（A5）。
    const minX = Math.min(Math.max(1, CROP_MIN_VP / scale), info.width);
    const minY = Math.min(Math.max(1, CROP_MIN_VP / scale), info.height);
    const kind = this.cropDragKind;
    if (kind === 'move') {
      // 整体平移：宽高保持不变，越界时贴边停止（R2.3）。
      const w = Math.max(0, this.cropX1 - this.cropX0);
      const h = Math.max(0, this.cropY1 - this.cropY0);
      const nx0 = Math.max(0, Math.min(info.width - w, this.cropX0));
      const ny0 = Math.max(0, Math.min(info.height - h, this.cropY0));
      this.cropX0 = nx0;
      this.cropY0 = ny0;
      this.cropX1 = nx0 + w;
      this.cropY1 = ny0 + h;
      return;
    }
    // 手柄拖拽：对角线/边只推被拖动的边，用"对边 ± 最小尺寸"夹住，永不翻转。
    let x0 = this.cropX0;
    let x1 = this.cropX1;
    let y0 = this.cropY0;
    let y1 = this.cropY1;
    const movesX0 = kind === 'nw' || kind === 'w' || kind === 'sw';
    const movesX1 = kind === 'ne' || kind === 'e' || kind === 'se';
    const movesY0 = kind === 'nw' || kind === 'n' || kind === 'ne';
    const movesY1 = kind === 'sw' || kind === 's' || kind === 'se';
    if (movesX0) {
      x0 = Math.max(0, Math.min(x1 - minX, x0));
    } else if (movesX1) {
      x1 = Math.min(info.width, Math.max(x0 + minX, x1));
    } else {
      x0 = Math.max(0, Math.min(info.width, x0));
      x1 = Math.max(0, Math.min(info.width, x1));
    }
    if (movesY0) {
      y0 = Math.max(0, Math.min(y1 - minY, y0));
    } else if (movesY1) {
      y1 = Math.min(info.height, Math.max(y0 + minY, y1));
    } else {
      y0 = Math.max(0, Math.min(info.height, y0));
      y1 = Math.max(0, Math.min(info.height, y1));
    }
    // kind === ''（未拖拽）等防御路径：只保证不翻转，不足 1 像素时兜底并贴边。
    if (x1 - x0 < 1) {
      x1 = Math.min(info.width, x0 + 1);
      if (x1 - x0 < 1) {
        x0 = Math.max(0, x1 - 1);
      }
    }
    if (y1 - y0 < 1) {
      y1 = Math.min(info.height, y0 + 1);
      if (y1 - y0 < 1) {
        y0 = Math.max(0, y1 - 1);
      }
    }
    this.cropX0 = x0;
    this.cropY0 = y0;
    this.cropX1 = x1;
    this.cropY1 = y1;
  }

  // 屏幕投影：返回 [left, top, w, h]（vp），供命中判定、绘制与标签定位共用。
  private cropScreenRect(): number[] {
    const scale = this.displayScale();
    if (scale <= 0) {
      return [0, 0, 0, 0];
    }
    // 易错点：displayLeft()/displayTop() 内部已含 regionX/regionY * scale，
    // 因此文档→屏幕换算必须减 regionX/regionY（与 drawSelectionFrame 完全一致）；
    // usingDetail 时公式不变，不得特判。
    const left = this.displayLeft() + (this.cropX0 - this.regionX) * scale;
    const top = this.displayTop() + (this.cropY0 - this.regionY) * scale;
    return [left, top, (this.cropX1 - this.cropX0) * scale, (this.cropY1 - this.cropY0) * scale];
  }

  // 8 个手柄中心（屏幕 vp，含内移）：drawCropOverlay 与 cropHandleAt 共用本函数，
  // 保证"看到的手柄"与"能点的手柄"是同一套坐标，杜绝两份实现漂移（P2 修复）。
  // 返回顺序固定：nw, n, ne, e, se, s, sw, w（每项 x,y 连续存放）。
  // 角手柄 X/Y 两个方向都内移；边手柄只在法向（上/下/左/右）内移。
  // 框屏幕宽/高不足 2×内移量（<48vp）时进入 tiny 模式：角手柄 1/4 内缩，
// 边手柄隐藏且不可命中，避免 8 个手柄中心交叉重叠。
  private cropTinyMode(): boolean {
    const r = this.cropScreenRect();
    return r[2] < 2 * CROP_HANDLE_INSET_VP || r[3] < 2 * CROP_HANDLE_INSET_VP;
  }

  private cropHandleCenters(): number[] {
    const r = this.cropScreenRect();
    const insetX = Math.min(CROP_HANDLE_INSET_VP, Math.max(0, r[2]) / 4);
    const insetY = Math.min(CROP_HANDLE_INSET_VP, Math.max(0, r[3]) / 4);
    const left = r[0] + insetX;
    const right = r[0] + r[2] - insetX;
    const top = r[1] + insetY;
    const bottom = r[1] + r[3] - insetY;
    const cx = r[0] + r[2] / 2;
    const cy = r[1] + r[3] / 2;
    return [left, top, cx, top, right, top, right, cy, right, bottom, cx, bottom, left, bottom, left, cy];
  }

  // 叠加层绘制（R3.1）：调用方负责先 clearBrushPreview()，本方法只画不清理。
  private drawCropOverlay(): void {
    const r = this.cropScreenRect();
    const l = r[0];
    const t = r[1];
    const w = r[2];
    const h = r[3];
    if (w <= 0 || h <= 0) {
      return;
    }
    const ctx = this.brushCtx;
    const vw = this.viewportW;
    const vh = this.viewportH;
    ctx.save();
    // 4 块框外遮罩（宽或高 <= 0 的块跳过）。
    ctx.fillStyle = CROP_MASK_COLOR;
    if (vw > 0 && t > 0) {
      ctx.fillRect(0, 0, vw, t);
    }
    if (vw > 0 && vh - (t + h) > 0) {
      ctx.fillRect(0, t + h, vw, vh - (t + h));
    }
    if (l > 0 && h > 0) {
      ctx.fillRect(0, t, l, h);
    }
    if (vw - (l + w) > 0 && h > 0) {
      ctx.fillRect(l + w, t, vw - (l + w), h);
    }
    // 框内轻微提亮。
    ctx.fillStyle = 'rgba(255,255,255,0.06)';
    ctx.fillRect(l, t, w, h);
    // 实线框 + 内侧虚线。
    ctx.strokeStyle = CROP_ACCENT;
    ctx.lineWidth = 1.5;
    ctx.setLineDash([]);
    ctx.strokeRect(l, t, w, h);
    ctx.strokeStyle = 'rgba(255,255,255,0.55)';
    ctx.lineWidth = 1;
    ctx.setLineDash([5, 4]);
    ctx.strokeRect(l + 0.5, t + 0.5, Math.max(0, w - 1), Math.max(0, h - 1));
    ctx.setLineDash([]);
    // 8 个白色方块手柄（深色描边）：角 4 + 边中点 4。
    // 中心坐标统一取自 cropHandleCenters（与命中判定同源，含边界内移），手柄尺寸/颜色/形状不变。
    const c = this.cropHandleCenters();

    const tiny = this.cropTinyMode();
    const handle = tiny ? Math.max(4, Math.min(CROP_HANDLE_VP, Math.min(r[2], r[3]) / 3)) : CROP_HANDLE_VP;
    const half = handle / 2;
    ctx.fillStyle = '#ffffff';
    ctx.strokeStyle = '#0b1220';
    ctx.lineWidth = 1;
    for (let i = 0; i < 8; i++) {
      if (tiny && i % 2 === 1) {
        continue;
      }
      const hx = c[i * 2];
      const hy = c[i * 2 + 1];
      ctx.fillRect(hx - half, hy - half, handle, handle);
      ctx.strokeRect(hx - half, hy - half, handle, handle);
    }
    ctx.restore();
  }

  // 尺寸文案（R3.3）：整数文档像素。
  private cropSizeLabel(): string {
    return Math.round(this.cropX1 - this.cropX0) + ' × ' + Math.round(this.cropY1 - this.cropY0);
  }

  // 框是否与整幅画布重合（R4.5）：四边差 < 0.5 视为重合。
  private cropIsFullCanvas(): boolean {
    const info = this.info;
    if (info === undefined) {
      return false;
    }
    return Math.abs(this.cropX0) < 0.5 && Math.abs(this.cropY0) < 0.5 &&
      Math.abs(this.cropX1 - info.width) < 0.5 && Math.abs(this.cropY1 - info.height) < 0.5;
  }

  // 标签横向位置：clamp(r[0], 4, max(4, viewportW - 96))。
  private cropLabelX(): number {
    const r = this.cropScreenRect();
    const maxX = Math.max(4, this.viewportW - 96);
    return Math.min(Math.max(r[0], 4), maxX);
  }

  // 标签纵向位置：框顶 >= 30vp 时放框上方 26vp，否则移到框内左上 +6vp。
  private cropLabelY(): number {
    const r = this.cropScreenRect();
    return r[1] >= 30 ? r[1] - 26 : r[1] + 6;
  }

  // ===== 画布变换手势（C-TRANSFORM-GESTURE@1）=====
  // 红线：trBeginDrag / trUpdateDrag / trEndDrag / drawTransformOverlay 内零引擎调用、零 checkpoint；
  // 只有 trApply 允许一次 layerTransform 写调用，只有 trCancel 允许 refreshPreview/scheduleDetail。
  // 变换框 = 图层内容包围盒的近似：LayerInfo 只有 id/name/kind，不含内容尺寸，原生 bounds 仅在提交后返回，
  // 因此进入时以文档 bounds 作为初始框（提交时由引擎按真实内容作用），不读不到数据就放弃。

  // 启用条件：有文档、非忙、选中图层类型支持变换（与面板 trSupported 同源门控）。
  private trEnabled(): boolean {
    return this.docId !== 0 && !this.busy && this.trSupported();
  }

  // 变换框（文档坐标，左/上/右/下）。
  private trRect(): number[] {
    const info = this.info;
    if (info === undefined) {
      return [0, 0, 0, 0];
    }
    return [0, 0, info.width, info.height];
  }

  // 预览是否偏离默认增量（1e-6 容差）；默认时投影走恒等路径，避免无谓抖动。
  private trPreviewActive(): boolean {
    const eps = 1e-6;
    return Math.abs(this.trPreviewScaleX - 100) > eps || Math.abs(this.trPreviewScaleY - 100) > eps ||
      Math.abs(this.trPreviewAngle) > eps || Math.abs(this.trPreviewDx) > eps || Math.abs(this.trPreviewDy) > eps;
  }

  // 文档点按预览增量做仿射映射：T(-c) → 缩放 → 旋转 → T(c) → 平移，与原生 pc_layer_transform 完全一致。
  private trMapDocPoint(docX: number, docY: number, rect: number[]): number[] {
    if (!this.trPreviewActive()) {
      return [docX, docY];
    }
    const cx = (rect[0] + rect[2]) / 2;
    const cy = (rect[1] + rect[3]) / 2;
    const u = (docX - cx) * (this.trPreviewScaleX / 100);
    const v = (docY - cy) * (this.trPreviewScaleY / 100);
    const rad = this.trPreviewAngle * Math.PI / 180;
    const c0 = Math.cos(rad);
    const s0 = Math.sin(rad);
    return [
      c0 * u - s0 * v + cx + this.trPreviewDx,
      s0 * u + c0 * v + cy + this.trPreviewDy
    ];
  }

  // 变换后四角的屏幕坐标（vp）：nw, ne, se, sw 各 2 个数，共 8 个；缩放不可用时全零。
  private trQuadScreen(): number[] {
    const rect = this.trRect();
    const scale = this.displayScale();
    if (scale <= 0) {
      return [0, 0, 0, 0, 0, 0, 0, 0];
    }
    // 易错点：displayLeft()/displayTop() 内部已含 regionX/regionY * scale，
    // 因此文档→屏幕换算必须减 regionX/regionY（与 trScreenRect 原实现完全一致）。
    const left = this.displayLeft();
    const top = this.displayTop();
    const docCorners: number[] = [rect[0], rect[1], rect[2], rect[1], rect[2], rect[3], rect[0], rect[3]];
    const out: number[] = [];
    for (let i = 0; i < 4; i++) {
      const p = this.trMapDocPoint(docCorners[i * 2], docCorners[i * 2 + 1], rect);
      out.push(left + (p[0] - this.regionX) * scale);
      out.push(top + (p[1] - this.regionY) * scale);
    }
    return out;
  }

  // 屏幕投影：变换后四角屏幕坐标的轴对齐包围盒 [left, top, w, h]（vp），
  // 供框内平移判定、绘制守卫与标签定位共用。
  private trScreenRect(): number[] {
    const q = this.trQuadScreen();
    const minX = Math.min(Math.min(q[0], q[2]), Math.min(q[4], q[6]));
    const maxX = Math.max(Math.max(q[0], q[2]), Math.max(q[4], q[6]));
    const minY = Math.min(Math.min(q[1], q[3]), Math.min(q[5], q[7]));
    const maxY = Math.max(Math.max(q[1], q[3]), Math.max(q[5], q[7]));
    return [minX, minY, maxX - minX, maxY - minY];
  }

  // 8 个手柄中心（屏幕 vp，含沿各边向中心的内移），顺序固定 nw, n, ne, e, se, s, sw, w。
  private trHandleCenters(): number[] {
    const q = this.trQuadScreen();
    const nwX = q[0];
    const nwY = q[1];
    const neX = q[2];
    const neY = q[3];
    const seX = q[4];
    const seY = q[5];
    const swX = q[6];
    const swY = q[7];
    // 内移量取 min(CROP_HANDLE_INSET_VP, 边长 / 4)，与轴对齐旧实现同式。
    const topLen = Math.sqrt((neX - nwX) * (neX - nwX) + (neY - nwY) * (neY - nwY));
    const rightLen = Math.sqrt((seX - neX) * (seX - neX) + (seY - neY) * (seY - neY));
    const bottomLen = Math.sqrt((swX - seX) * (swX - seX) + (swY - seY) * (swY - seY));
    const leftLen = Math.sqrt((nwX - swX) * (nwX - swX) + (nwY - swY) * (nwY - swY));
    const insetTop = Math.min(CROP_HANDLE_INSET_VP, topLen / 4);
    const insetRight = Math.min(CROP_HANDLE_INSET_VP, rightLen / 4);
    const insetBottom = Math.min(CROP_HANDLE_INSET_VP, bottomLen / 4);
    const insetLeft = Math.min(CROP_HANDLE_INSET_VP, leftLen / 4);
    // 四条边的单位方向（沿 nw→ne→se→sw 顺时针），旋转后仍按同一局部轴内移。
    const dTopX = topLen > 1e-6 ? (neX - nwX) / topLen : 0;
    const dTopY = topLen > 1e-6 ? (neY - nwY) / topLen : 0;
    const dRightX = rightLen > 1e-6 ? (seX - neX) / rightLen : 0;
    const dRightY = rightLen > 1e-6 ? (seY - neY) / rightLen : 0;
    const dBottomX = bottomLen > 1e-6 ? (swX - seX) / bottomLen : 0;
    const dBottomY = bottomLen > 1e-6 ? (swY - seY) / bottomLen : 0;
    const dLeftX = leftLen > 1e-6 ? (nwX - swX) / leftLen : 0;
    const dLeftY = leftLen > 1e-6 ? (nwY - swY) / leftLen : 0;
    // 角手柄两轴各内移；边中点取两相邻角手柄的中点（等效于单轴内移，轴对齐时与旧实现逐位相同）。
    const nwHX = nwX + dTopX * insetTop - dLeftX * insetLeft;
    const nwHY = nwY + dTopY * insetTop - dLeftY * insetLeft;
    const neHX = neX - dTopX * insetTop + dRightX * insetRight;
    const neHY = neY - dTopY * insetTop + dRightY * insetRight;
    const seHX = seX - dRightX * insetRight + dBottomX * insetBottom;
    const seHY = seY - dRightY * insetRight + dBottomY * insetBottom;
    const swHX = swX - dBottomX * insetBottom + dLeftX * insetLeft;
    const swHY = swY - dBottomY * insetBottom + dLeftY * insetLeft;
    return [
      nwHX, nwHY,
      (nwHX + neHX) / 2, (nwHY + neHY) / 2,
      neHX, neHY,
      (neHX + seHX) / 2, (neHY + seHY) / 2,
      seHX, seHY,
      (seHX + swHX) / 2, (seHY + swHY) / 2,
      swHX, swHY,
      (swHX + nwHX) / 2, (swHY + nwHY) / 2
    ];
  }

  // 旋转手柄中心：变换后顶边中点沿该边外法线方向 28vp（无旋转时即框顶中点上方 28vp）。
  private trRotateCenter(): number[] {
    const q = this.trQuadScreen();
    const mx = (q[0] + q[2]) / 2;
    const my = (q[1] + q[3]) / 2;
    const dx = q[2] - q[0];
    const dy = q[3] - q[1];
    const len = Math.sqrt(dx * dx + dy * dy);
    if (len <= 1e-6) {
      return [mx, my - 28];
    }
    // 顶边方向在屏幕坐标（y 向下）顺时针转 -90° 即外法线。
    return [mx + dy / len * 28, my - dx / len * 28];
  }

  // 手柄命中：旋转手柄 > 4 角 > 4 边中点 > 框内平移 > 框外空串。
  private trHandleAt(screenX: number, screenY: number): string {
    const c = this.trHandleCenters();
    const rot = this.trRotateCenter();
    const half = CROP_HANDLE_HIT_VP / 2;
    if (Math.abs(screenX - rot[0]) <= half && Math.abs(screenY - rot[1]) <= half) {
      return 'rot';
    }
    if (Math.abs(screenX - c[0]) <= half && Math.abs(screenY - c[1]) <= half) {
      return 'nw';
    }
    if (Math.abs(screenX - c[4]) <= half && Math.abs(screenY - c[5]) <= half) {
      return 'ne';
    }
    if (Math.abs(screenX - c[8]) <= half && Math.abs(screenY - c[9]) <= half) {
      return 'se';
    }
    if (Math.abs(screenX - c[12]) <= half && Math.abs(screenY - c[13]) <= half) {
      return 'sw';
    }
    if (Math.abs(screenX - c[2]) <= half && Math.abs(screenY - c[3]) <= half) {
      return 'n';
    }
    if (Math.abs(screenX - c[6]) <= half && Math.abs(screenY - c[7]) <= half) {
      return 'e';
    }
    if (Math.abs(screenX - c[10]) <= half && Math.abs(screenY - c[11]) <= half) {
      return 's';
    }
    if (Math.abs(screenX - c[14]) <= half && Math.abs(screenY - c[15]) <= half) {
      return 'w';
    }
    const r = this.trScreenRect();
    if (screenX > r[0] && screenX < r[0] + r[2] && screenY > r[1] && screenY < r[1] + r[3]) {
      return 'move';
    }
    return '';
  }

  // 进入变换模式：记录拖拽起点快照，结束进行中的其它拖拽，不调用任何引擎接口。
  private trEnter(): void {
    if (!this.trEnabled() || this.info === undefined) {
      if (this.docId !== 0 && !this.busy) {
        this.status = '该图层类型不支持变换';
      }
      return;
    }
    this.cancelStroke();
    this.cancelSelect();
    this.cancelShape();
    this.cancelMove();
    this.trDragging = false;
    this.trDragKind = '';
    this.trFrameCount = 0;
    this.trResetPreview();
    this.trMode = true;
    this.status = '变换：拖手柄缩放/旋转，拖框内平移';
    this.redrawCanvas();
  }

  // 退出并丢弃预览：恢复画布叠加层与画面（零引擎写调用）。
  private trCancel(): void {
    if (!this.trMode) {
      return;
    }
    this.trMode = false;
    this.trDragging = false;
    this.trDragKind = '';
    this.trFrameCount = 0;
    this.trResetPreview();
    // 退出后 redrawCanvas 走普通分支：必须先清掉叠加层，否则无选区时会早退、旧框残留。
    this.clearBrushPreview();
    this.redrawCanvas();
    this.status = '已取消变换';
    // 拖拽期间画布像素始终是变换前的状态，退出后按既有刷新路径主动对齐一次。
    if (this.usingDetail) {
      this.perf('tr cancel refresh usingDetail=true');
      this.refreshPreview();
    } else {
      this.perf('tr cancel refresh usingDetail=false');
      this.scheduleDetail();
    }
  }

  // 预览值归零（相对增量的默认值：100/100/0/0/0，与面板草稿同量纲）。
  private trResetPreview(): void {
    this.trPreviewScaleX = 100;
    this.trPreviewScaleY = 100;
    this.trPreviewAngle = 0;
    this.trPreviewDx = 0;
    this.trPreviewDy = 0;
  }

  // 应用：一次 layerTransform 提交（原生层一次 checkpoint）→ afterEdit 单路径刷新 → 退出并归零。
  // 失败只写状态栏并保持变换模式与预览，便于修正后重试。
  private async trApply(): Promise<void> {
    if (this.docId === 0 || this.selectedLayerId === 0 || !this.trMode) {
      return;
    }
    if (!this.trSupported()) {
      this.status = '该图层类型不支持变换';
      return;
    }
    const layerId = this.selectedLayerId;
    this.busy = true;
    try {
      Engine.layerTransform(this.docId, layerId, JSON.stringify({
        scaleX: this.trPreviewScaleX / 100,
        scaleY: this.trPreviewScaleY / 100,
        angle: this.trPreviewAngle,
        dx: this.trPreviewDx,
        dy: this.trPreviewDy
      }));
      await this.afterEdit();
      this.trMode = false;
      this.trDragging = false;
      this.trDragKind = '';
      this.trFrameCount = 0;
      this.trResetPreview();
      this.status = '已应用变换';
    } catch (err) {
      this.status = '变换失败：' + this.messageOf(err as Object);
    } finally {
      this.busy = false;
      this.clearBrushPreview();
      this.redrawCanvas();
    }
  }

  // 开始拖拽：kind 为空时不开始（trDragging 保持 false）；幂等处理上一次丢失的 Up。
  private trBeginDrag(screenX: number, screenY: number): void {
    if (!this.trMode || this.info === undefined) {
      return;
    }
    if (this.trDragging) {
      this.trEndDrag();
    }
    const kind = this.trHandleAt(screenX, screenY);
    this.trDragKind = kind;
    this.trFrameCount = 0;
    if (kind === '') {
      this.trDragging = false;
      return;
    }
    this.trDragging = true;
    this.trStartDocX = this.screenToDocX(screenX);
    this.trStartDocY = this.screenToDocY(screenY);
    this.trOrigScaleX = this.trPreviewScaleX;
    this.trOrigScaleY = this.trPreviewScaleY;
    this.trOrigAngle = this.trPreviewAngle;
    this.trOrigDx = this.trPreviewDx;
    this.trOrigDy = this.trPreviewDy;
  }

  // 更新拖拽：每帧从起点快照重算（不累积误差），只写 trPreview* 并重绘叠加层；零引擎调用。
  // 缩放比例按「拖拽点相对框中心的距离」计算，旋转按「相对框中心的角度增量」计算。
  private trUpdateDrag(screenX: number, screenY: number): void {
    if (!this.trMode || !this.trDragging || this.trDragKind === '' || this.info === undefined) {
      return;
    }
    const startedAt = Date.now();
    const rect = this.trRect();
    const cx = (rect[0] + rect[2]) / 2;
    const cy = (rect[1] + rect[3]) / 2;
    const curDocX = this.screenToDocX(screenX);
    const curDocY = this.screenToDocY(screenY);
    const startX = this.trStartDocX - cx;
    const startY = this.trStartDocY - cy;
    const curX = curDocX - cx;
    const curY = curDocY - cy;
    // 起点几乎落在框中心（< 0.5 文档像素）时该方向比例不可用，保持原值。
    const minBase = 0.5;
    const hasX = Math.abs(startX) > minBase;
    const hasY = Math.abs(startY) > minBase;
    const ratioX = hasX ? Math.abs(curX) / Math.abs(startX) : 1;
    const ratioY = hasY ? Math.abs(curY) / Math.abs(startY) : 1;
    const kind = this.trDragKind;
    let scaleX = this.trOrigScaleX;
    let scaleY = this.trOrigScaleY;
    let angle = this.trOrigAngle;
    let dx = this.trOrigDx;
    let dy = this.trOrigDy;
    if (kind === 'nw' || kind === 'ne' || kind === 'se' || kind === 'sw') {
      // 角手柄等比：两轴都可用时取平均，否则用唯一可用轴。
      let ratio = 1;
      if (hasX && hasY) {
        ratio = (ratioX + ratioY) / 2;
      } else if (hasX) {
        ratio = ratioX;
      } else if (hasY) {
        ratio = ratioY;
      }
      scaleX = this.trOrigScaleX * ratio;
      scaleY = this.trOrigScaleY * ratio;
    } else if (kind === 'e' || kind === 'w') {
      scaleX = this.trOrigScaleX * (hasX ? ratioX : 1);
    } else if (kind === 'n' || kind === 's') {
      scaleY = this.trOrigScaleY * (hasY ? ratioY : 1);
    } else if (kind === 'rot') {
      const startLen = Math.sqrt(startX * startX + startY * startY);
      if (startLen > minBase) {
        const delta = Math.atan2(curY, curX) - Math.atan2(startY, startX);
        angle = this.trOrigAngle + delta * 180 / Math.PI;
        // 归一化到 (-180, 180]，避免越过 ±180 时角度突变。
        while (angle > 180) {
          angle -= 360;
        }
        while (angle <= -180) {
          angle += 360;
        }
      }
    } else if (kind === 'move') {
      dx = this.trOrigDx + (curDocX - this.trStartDocX);
      dy = this.trOrigDy + (curDocY - this.trStartDocY);
    }
    this.trPreviewScaleX = scaleX;
    this.trPreviewScaleY = scaleY;
    this.trPreviewAngle = angle;
    this.trPreviewDx = dx;
    this.trPreviewDy = dy;
    this.trClampPreview();
    // 拖拽期间只重绘叠加层：不调用任何引擎/渲染/缓存接口，不产生检查点。
    this.redrawCanvas();
    this.trFrameCount++;
    this.perf('trDrag n=' + this.trFrameCount + ' ms=' + (Date.now() - startedAt));
  }

  // 结束拖拽：只复位拖拽态；预览值保留供「应用」提交。
  private trEndDrag(): void {
    if (!this.trDragging && this.trDragKind === '') {
      return;
    }
    this.trDragging = false;
    this.trDragKind = '';
    this.trFrameCount = 0;
    this.redrawCanvas();
  }

  // 预览值夹取：范围与面板 5 个滑块完全一致（缩放 10~400%，旋转 -180~180°，偏移 ±200px）。
  private trClampPreview(): void {
    this.trPreviewScaleX = Math.min(400, Math.max(10, this.trPreviewScaleX));
    this.trPreviewScaleY = Math.min(400, Math.max(10, this.trPreviewScaleY));
    this.trPreviewAngle = Math.min(180, Math.max(-180, this.trPreviewAngle));
    this.trPreviewDx = Math.min(200, Math.max(-200, this.trPreviewDx));
    this.trPreviewDy = Math.min(200, Math.max(-200, this.trPreviewDy));
  }

  // 叠加层绘制：调用方负责先 clearBrushPreview()，本方法只画不清理。
  private drawTransformOverlay(): void {
    const r = this.trScreenRect();
    if (r[2] <= 0 || r[3] <= 0) {
      return;
    }
    const q = this.trQuadScreen();
    const ctx = this.brushCtx;
    ctx.save();
    // 框内轻微提亮，标出变换作用范围；不做框外遮罩，避免与画布内容抢注意力。
    ctx.beginPath();
    ctx.moveTo(q[0], q[1]);
    ctx.lineTo(q[2], q[3]);
    ctx.lineTo(q[4], q[5]);
    ctx.lineTo(q[6], q[7]);
    ctx.closePath();
    ctx.fillStyle = 'rgba(255,255,255,0.06)';
    ctx.fill();
    ctx.strokeStyle = CROP_ACCENT;
    ctx.lineWidth = 1.5;
    ctx.setLineDash([]);
    ctx.beginPath();
    ctx.moveTo(q[0], q[1]);
    ctx.lineTo(q[2], q[3]);
    ctx.lineTo(q[4], q[5]);
    ctx.lineTo(q[6], q[7]);
    ctx.closePath();
    ctx.stroke();
    // 旋转手柄：顶边中点沿外法线 28vp，短线连接框边以便识别归属。
    const rot = this.trRotateCenter();
    const topMidX = (q[0] + q[2]) / 2;
    const topMidY = (q[1] + q[3]) / 2;
    ctx.beginPath();
    ctx.moveTo(topMidX, topMidY);
    ctx.lineTo(rot[0], rot[1]);
    ctx.lineWidth = 1;
    ctx.stroke();
    ctx.beginPath();
    ctx.arc(rot[0], rot[1], CROP_HANDLE_VP / 2 + 2, 0, Math.PI * 2);
    ctx.fillStyle = '#ffffff';
    ctx.fill();
    ctx.strokeStyle = '#0b1220';
    ctx.stroke();
    // 8 个白色方块手柄（与命中判定同源，含边界内移）。
    const c = this.trHandleCenters();
    const half = CROP_HANDLE_VP / 2;
    ctx.fillStyle = '#ffffff';
    ctx.strokeStyle = '#0b1220';
    ctx.lineWidth = 1;
    for (let i = 0; i < 8; i++) {
      const hx = c[i * 2];
      const hy = c[i * 2 + 1];
      ctx.fillRect(hx - half, hy - half, CROP_HANDLE_VP, CROP_HANDLE_VP);
      ctx.strokeRect(hx - half, hy - half, CROP_HANDLE_VP, CROP_HANDLE_VP);
    }
    ctx.restore();
  }

  // 实时标签：缩放 / 旋转 / 偏移（偏移为相对当前状态的增量，单位文档像素）。
  private trLabelText(): string {
    return '缩放 ' + Math.round(this.trPreviewScaleX) + '%' +
      ' / 旋转 ' + Math.round(this.trPreviewAngle) + '°' +
      ' / 偏移 ' + Math.round(this.trPreviewDx) + ',' + Math.round(this.trPreviewDy);
  }

  // 标签横向位置：clamp(框左, 4, max(4, viewportW - 220))。
  private trLabelX(): number {
    const r = this.trScreenRect();
    const maxX = Math.max(4, this.viewportW - 220);
    return Math.min(Math.max(r[0], 4), maxX);
  }

  // 标签纵向位置：框顶 >= 56vp 时放框上方（避开旋转手柄），否则移到框内左上 +6vp。
  private trLabelY(): number {
    const r = this.trScreenRect();
    return r[1] >= 56 ? r[1] - 52 : r[1] + 6;
  }

  private resetRenderCache(): void {
    this.detailKey = '';
    this.previewKey = '';
    this.clearPixelCaches();
  }

  // 统一释放入口：release 失败只影响像素回收，不允许异常打断 UI 线程。
  private safeRelease(map: image.PixelMap): void {
    try {
      map.release();
    } catch (err) {
      // 已释放或渲染上下文已销毁：忽略即可，不因回收失败改写状态栏。
    }
  }

  // 释放像素缓存槽。displayPixels 独立持有“当前显示像素”的引用，
  // 因此这里置空 detailPixels / previewPixels 不会让仍在显示的 PixelMap 失去底层字节
  // （同一个 ArrayBuffer 的引用计数仍大于 0），只释放确实已经没人使用的缓存字节。
  private clearPixelCaches(): void {
    this.detailPixels = undefined;
    this.previewPixels = undefined;
  }

  // 命中字节级自检（仅 debugCacheCheck=true 时由 renderDetail 调用）。
  // 用引擎真值比对缓存字节：same=true 说明缓存像素本身正确，画面差异只可能来自绘制状态；
  // same=false 说明缓存槽内容错（字节被复用/改写/长度不符），需要在缓存层修。
  // 该路径只读返回值、不创建 PixelMap，也不计入 renderCalls，避免污染 PERF 语义。
  private runCacheCheck(cached: ArrayBuffer, x0: number, y0: number, regionW: number, regionH: number): void {
    try {
      const truth = Engine.renderPixels(this.docId, x0, y0, regionW, regionH);
      const got = new Uint8Array(cached);
      const want = new Uint8Array(truth);
      if (got.length !== want.length) {
        this.perf('cacheCheck same=false bytes=' + got.length + ' want=' + want.length);
        return;
      }
      let diff = 0;
      let first = -1;
      for (let i = 0; i < got.length; i++) {
        if (got[i] !== want[i]) {
          diff++;
          if (first < 0) {
            first = i;
          }
        }
      }
      if (diff === 0) {
        this.perf('cacheCheck same=true bytes=' + got.length);
        return;
      }
      const pixel = Math.floor(first / 4);
      this.perf('cacheCheck same=false diff=' + diff + ' bytes=' + got.length + ' first=' + first +
        ' xy=' + (pixel % regionW) + ':' + Math.floor(pixel / regionW) +
        ' got=' + got[first] + ' want=' + want[first]);
    } catch (err) {
      this.perf('cacheCheck error=true');
    }
  }

  private resetRenderStats(): void {
    this.renderCalls = 0;
    this.detailHits = 0;
    this.previewCalls = 0;
    this.previewHits = 0;
    this.editCalls = 0;
  }

  // 打开新文档后同步版本号；读取失败时降级为 0（缓存键随之失效，不阻塞渲染）。
  private syncDocRevision(): void {
    if (this.docId === 0) {
      this.docRevision = 0;
      return;
    }
    try {
      this.docRevision = Engine.revision(this.docId);
    } catch (err) {
      this.docRevision = 0;
    }
  }

  // 手势 update 只记度量，绝不触发引擎渲染（R7.3）：每次手势只在首次 update 记一行，避免日志刷屏。
  private noteGestureUpdate(name: string): void {
    this.gestureUpdates++;
    if (this.gestureUpdates === 1) {
      this.perf('gesture name=' + name + ' phase=update updates=' + this.gestureUpdates + ' renders=0 rev=' +
        this.docRevision + ' key=- hit=false calls=' + this.renderCalls + ' ms=0');
    }
  }

  private endGesture(name: string): void {
    this.perf('gesture name=' + name + ' phase=end updates=' + this.gestureUpdates + ' renders=0 rev=' +
      this.docRevision + ' key=- hit=false calls=' + this.renderCalls + ' ms=0');
    this.gestureUpdates = 0;
  }

  aboutToAppear(): void {
    this.initTextFont();
    // 首选项在既有初始化之后读取：读不到时保持默认值，不影响字体/渲染初始化。
    this.loadPrefs();
  }

  private async initTextFont(): Promise<void> {
    try {
      const names: string[] = font.getSystemFontList();
      let target = '';
      const preferred: string[] = ['HarmonyOS Sans SC', 'HarmonyOS Sans', 'Noto Sans CJK SC', 'Noto Sans SC', 'Source Han Sans SC'];
      for (const wanted of preferred) {
        for (const name of names) {
          if (name.indexOf(wanted) >= 0) {
            target = name;
            break;
          }
        }
        if (target !== '') {
          break;
        }
      }
      if (target === '') {
        this.status = '文字：未找到系统中文字体，使用内置字体（中文可能缺失）';
        return;
      }
      const info = font.getFontByName(target);
      if (info.path.length === 0) {
        return;
      }
      const stat = fileIo.statSync(info.path);
      if (stat.size <= 0) {
        return;
      }
      const file = fileIo.openSync(info.path, fileIo.OpenMode.READ_ONLY);
      const buffer = new ArrayBuffer(stat.size);
      const read = fileIo.readSync(file.fd, buffer);
      fileIo.closeSync(file);
      if (read <= 0) {
        return;
      }
      const families = Engine.fontRegister(buffer.slice(0, read));
      if (families.length > 0) {
        this.textFontFamily = families[0];
        this.textFontName = families[0];
        this.textFontReady = true;
      }
    } catch (err) {
      // Keep the bundled font; the card shows which one is in use.
    }
  }

  aboutToDisappear(): void {
    if (this.renderTimer !== -1) {
      clearTimeout(this.renderTimer);
      this.renderTimer = -1;
    }
    if (this.thumbTimer !== -1) {
      clearTimeout(this.thumbTimer);
      this.thumbTimer = -1;
    }
    if (this.layerThumb !== undefined) {
      this.layerThumb = undefined;
    }
    this.clearAllLayerThumbs();
    if (this.docId !== 0) {
      Engine.close(this.docId);
      this.docId = 0;
    }
    // 离开页面：清空编辑草稿与变换草稿（不产生任何引擎调用）。
    this.editReset();
    this.trResetDraft();
    // 页面销毁等价于关闭文档：先清缓存槽（displayPixels 仍持有显示像素的字节，不会提前失效），
    // 再回收显示像素、清空显示像素源与版本号（R8.3）。
    this.resetRenderCache();
    this.resetRenderStats();
    this.docRevision = 0;
    if (this.pixelMap !== undefined) {
      this.safeRelease(this.pixelMap);
      this.pixelMap = undefined;
    }
    // 显示像素已回收：像素源引用一并释放，避免 ArrayBuffer 残留（≈32MB / ≈16MB）。
    this.displayPixels = undefined;
  }

  private imageSizeLabel(): string {
    const info = this.info;
    if (info === undefined) {
      return '';
    }
    return info.width + ' × ' + info.height;
  }

  private messageOf(err: Object): string {
    const message = (err as Error).message;
    return message !== undefined && message.length > 0 ? message : String(err);
  }

  private async openFile(): Promise<void> {
    try {
      const options = new picker.DocumentSelectOptions();
      options.maxSelectNumber = 1;
      options.fileSuffixFilters = ['图片|.png,.jpg,.jpeg,.webp,.bmp,.tif,.tiff,.psd,.psb'];
      const uris = await new picker.DocumentViewPicker(getContext(this)).select(options);
      if (uris.length === 0) {
        return;
      }
      this.busy = true;
      this.status = '正在解析文件…';
      const uri = uris[0];
      const file = fileIo.openSync(uri, fileIo.OpenMode.READ_ONLY);
      const stat = fileIo.statSync(file.fd);
      const data = new ArrayBuffer(stat.size);
      fileIo.readSync(file.fd, data);
      fileIo.closeSync(file);

      if (this.docId !== 0) {
        Engine.close(this.docId);
        this.docId = 0;
      }
      const name = uri.substring(uri.lastIndexOf('/') + 1);
      const outcome = Engine.open(name, data);
      this.docId = outcome.id;
      this.info = outcome.info;
      // 文档被替换：丢弃裁剪框且不恢复视图快照（R1.5）。
      if (this.cropMode) {
        this.cropDiscard();
      }
      this.layers = flattenLayers(outcome.info.layers, 0);
      this.selectedLayerId = 0;
      this.layerOpacity = 100;
      this.adjust = emptyAdjustment();
      this.fxReset();
      // 新文档：编辑分组草稿必须清空（防止上一张图的图层参数残留）。
      this.editReset();
      this.trResetDraft();
      this.cancelStroke();
      this.cancelShape();
      this.resetView();
      this.applyDefaultZoom();
      this.clearAllLayerThumbs();
      this.loadAllLayerThumbs();
      // 新文档：版本号对齐、缓存键与计数清零（R8.3 / R11.2）。
      this.resetRenderCache();
      this.resetRenderStats();
      this.syncDocRevision();
      // 打开文档：画布宽高草稿按新文档尺寸预填一次（C-CANVAS-UI@1 §B）。
      this.syncCanvasDraft();
      // 视图辅助（V2）：回填新文档的参考线（失败静默置空）。
      this.syncGuides();
      await this.refreshPreview();
      const info = outcome.info;
      this.status = info.width + ' × ' + info.height + ' · ' + info.mode + ' · ' + info.depth;
      if (outcome.warnings.length > 0) {
        promptAction.showToast({ message: outcome.warnings.join('\n'), duration: 4000 });
      }
    } catch (err) {
      this.status = '打开失败：' + this.messageOf(err as Object);
      promptAction.showToast({ message: '打开失败：' + this.messageOf(err as Object) });
    } finally {
      this.busy = false;
    }
  }

  private async openSample(): Promise<void> {
    try {
      this.busy = true;
      this.status = '正在载入示例…';
      const raw = getContext(this).resourceManager.getRawFileContentSync('sample.jpg');
      const data = raw.buffer.slice(raw.byteOffset, raw.byteOffset + raw.byteLength) as ArrayBuffer;
      if (this.docId !== 0) {
        Engine.close(this.docId);
        this.docId = 0;
      }
      const outcome = Engine.open('sample.jpg', data);
      this.docId = outcome.id;
      this.info = outcome.info;
      // 文档被替换：丢弃裁剪框且不恢复视图快照（R1.5）。
      if (this.cropMode) {
        this.cropDiscard();
      }
      this.layers = flattenLayers(outcome.info.layers, 0);
      this.selectedLayerId = 0;
      this.layerOpacity = 100;
      this.adjust = emptyAdjustment();
      this.fxReset();
      // 新文档：编辑分组草稿必须清空（防止上一张图的图层参数残留）。
      this.editReset();
      this.trResetDraft();
      this.cancelStroke();
      this.cancelShape();
      this.resetView();
      this.applyDefaultZoom();
      this.clearAllLayerThumbs();
      this.loadAllLayerThumbs();
      // 新文档：版本号对齐、缓存键与计数清零（R8.3 / R11.2）。
      this.resetRenderCache();
      this.resetRenderStats();
      this.syncDocRevision();
      // 打开文档：画布宽高草稿按新文档尺寸预填一次（C-CANVAS-UI@1 §B）。
      this.syncCanvasDraft();
      // 视图辅助（V2）：回填示例文档的参考线（失败静默置空）。
      this.syncGuides();
      await this.refreshPreview();
      const info = outcome.info;
      this.status = info.width + ' × ' + info.height + ' · ' + info.mode + ' · ' + info.depth;
    } catch (err) {
      this.status = '示例载入失败：' + this.messageOf(err as Object);
      promptAction.showToast({ message: '示例载入失败：' + this.messageOf(err as Object) });
    } finally {
      this.busy = false;
    }
  }

  // 新建空白文档（RGB / 8 位、白色 Background）：装载流程与 openSample 完全一致。
  private async newDocument(): Promise<void> {
    const width = Math.round(Number(this.newDocW));
    const height = Math.round(Number(this.newDocH));
    if (width < 1 || height < 1 || width > 16384 || height > 16384) {
      this.status = '新建文档：宽高需在 1–16384 之间';
      return;
    }
    try {
      this.busy = true;
      this.status = '正在新建文档…';
      if (this.docId !== 0) {
        Engine.close(this.docId);
        this.docId = 0;
      }
      const outcome = Engine.newDocument(width, height, '未命名');
      this.docId = outcome.id;
      this.info = outcome.info;
      // 文档被替换：丢弃裁剪框且不恢复视图快照（R1.5）。
      if (this.cropMode) {
        this.cropDiscard();
      }
      this.layers = flattenLayers(outcome.info.layers, 0);
      this.selectedLayerId = 0;
      this.layerOpacity = 100;
      this.adjust = emptyAdjustment();
      this.fxReset();
      this.editReset();
      this.trResetDraft();
      this.cancelStroke();
      this.cancelShape();
      this.resetView();
      this.applyDefaultZoom();
      this.clearAllLayerThumbs();
      this.loadAllLayerThumbs();
      this.resetRenderCache();
      this.resetRenderStats();
      this.syncDocRevision();
      this.syncCanvasDraft();
      // 视图辅助（V2）：新文档参考线为空，仍走一次回填以清理上一张图的叠加层。
      this.syncGuides();
      await this.refreshPreview();
      this.status = '已新建文档 · ' + outcome.info.width + ' × ' + outcome.info.height;
    } catch (err) {
      this.status = '新建失败：' + this.messageOf(err as Object);
      promptAction.showToast({ message: '新建失败：' + this.messageOf(err as Object) });
    } finally {
      this.busy = false;
    }
  }

  private resetView(): void {
    this.zoom = 1;
    this.panX = 0;
    this.panY = 0;
  }

  private fitScale(): number {
    const info = this.info;
    if (info === undefined || info.width <= 0 || info.height <= 0 || this.viewportW <= 0 || this.viewportH <= 0) {
      return 0;
    }
    return Math.min(this.viewportW / info.width, this.viewportH / info.height);
  }

  private displayScale(): number {
    const fit = this.fitScale();
    return fit > 0 ? fit * this.zoom : 0;
  }

  private displayLeft(): number {
    const info = this.info;
    if (info === undefined) {
      return 0;
    }
    const scale = this.displayScale();
    const base = (this.viewportW - info.width * scale) / 2 + this.panX;
    return base + this.regionX * scale;
  }

  private displayTop(): number {
    const info = this.info;
    if (info === undefined) {
      return 0;
    }
    const scale = this.displayScale();
    const base = (this.viewportH - info.height * scale) / 2 + this.panY;
    return base + this.regionY * scale;
  }

  private displayWidth(): number {
    const info = this.info;
    if (info === undefined) {
      return 0;
    }
    return (this.usingDetail ? this.regionW : info.width) * this.displayScale();
  }

  private displayHeight(): number {
    const info = this.info;
    if (info === undefined) {
      return 0;
    }
    return (this.usingDetail ? this.regionH : info.height) * this.displayScale();
  }

  private previewFactor(): number {
    const info = this.info;
    if (info === undefined || info.width <= 0 || info.height <= 0 || this.previewPixelW <= 0) {
      return 0;
    }
    return Math.max(this.previewPixelW / info.width, this.previewPixelH / info.height);
  }

  private needsDetail(): boolean {
    const factor = this.previewFactor();
    return factor > 0 && this.displayScale() > factor * 1.02;
  }

  private setZoom(target: number, focusX: number, focusY: number): void {
    const next = Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, target));
    const ratio = next / this.zoom;
    if (Math.abs(ratio - 1) < 0.0001) {
      return;
    }
    const centerX = this.viewportW / 2;
    const centerY = this.viewportH / 2;
    this.panX = (focusX - centerX) * (1 - ratio) + this.panX * ratio;
    this.panY = (focusY - centerY) * (1 - ratio) + this.panY * ratio;
    this.zoom = next;
    this.scheduleDetail();
    this.redrawCanvas();
  }

  private scheduleDetail(): void {
    if (this.renderTimer !== -1) {
      clearTimeout(this.renderTimer);
    }
    this.renderTimer = setTimeout(() => {
      this.renderTimer = -1;
      this.renderDetail();
    }, DETAIL_DELAY_MS);
  }

  private onViewportChanged(width: number, height: number): void {
    if (Math.abs(this.viewportW - width) < 0.5 && Math.abs(this.viewportH - height) < 0.5) {
      return;
    }
    this.viewportW = width;
    this.viewportH = height;
    this.scheduleDetail();
    this.redrawCanvas();
  }

  private clampDocX(value: number, max: number): number {
    return Math.min(max, Math.max(0, value));
  }

  private renderDetail(): void {
    const info = this.info;
    if (this.docId === 0 || info === undefined || this.viewportW <= 0 || this.viewportH <= 0) {
      return;
    }
    if (!this.needsDetail()) {
      if (this.usingDetail) {
        this.refreshPreview();
      }
      return;
    }
    const scale = this.displayScale();
    if (scale <= 0) {
      return;
    }
    const visibleW = this.viewportW / scale;
    const visibleH = this.viewportH / scale;
    const centerX = info.width / 2 - this.panX / scale;
    const centerY = info.height / 2 - this.panY / scale;
    // 平移复用（scroll reuse）：detail 区域自带 DETAIL_MARGIN 余量，且 displayLeft/displayTop 依赖
    // panX/panY，因此只要「当前可见区仍完全落在已渲染区域内」且文档版本未变，平移只需改显示偏移，
    // 不必重新渲染、也不必重建 PixelMap。这一步把「平移后必然重渲染一次」变成「余量内零渲染」。
    const visX0: number = centerX - visibleW / 2;
    const visY0: number = centerY - visibleH / 2;
    const visX1: number = centerX + visibleW / 2;
    const visY1: number = centerY + visibleH / 2;
    if (this.usingDetail && this.detailPixels !== undefined && this.detailKey !== '' &&
      this.detailKey.indexOf(this.docRevision + ':') === 0 &&
      visX0 >= this.regionX && visY0 >= this.regionY &&
      visX1 <= this.regionX + this.regionW && visY1 <= this.regionY + this.regionH) {
      this.detailHits++;
      this.perf('renderDetail reuse rev=' + this.docRevision + ' key=' + this.detailKey +
        ' hit=true calls=' + this.renderCalls + ' ms=0 w=' + this.regionW + ' h=' + this.regionH);
      return;
    }
    // 渲染区域按 DETAIL_TILE 网格对齐（R3）：原始区域先向外扩到网格边界，再 clamp 到文档范围，
    // 这样平移/缩放不足一格时区域边界不变，缓存键稳定从而命中既有字节缓存。
    // 顺序必须固定为「先按 tile 外扩 → 再 clamp」：若先 clamp 再对齐，文档边缘处的对齐结果会被裁歪，
    // 同一网格位置会算出不同区域，反而破坏命中。
    // 内存代价：对齐后每边最多多出 DETAIL_TILE-1 = 255 px（宽高各最多 +510 px），且结果必为 256 的整数倍。
    // 典型 1:1 视图可见区 1677×1247，含 DETAIL_MARGIN 后原始区域约 2080×1547（≈3.22M px / 12.3 MiB），
    // 对齐后典型 2304×1792（≈4.13M px / 15.8 MiB），最坏 2560×2048（≈5.24M px / 20.0 MiB），
    // 仍低于 MAX_DETAIL_PIXELS = 8M 上限。
    const baseX0: number = centerX - visibleW * (0.5 + DETAIL_MARGIN);
    const baseY0: number = centerY - visibleH * (0.5 + DETAIL_MARGIN);
    const baseX1: number = centerX + visibleW * (0.5 + DETAIL_MARGIN);
    const baseY1: number = centerY + visibleH * (0.5 + DETAIL_MARGIN);
    const ALIGNER: number = DETAIL_TILE;
    let x0: number = Math.floor(baseX0 / ALIGNER) * ALIGNER;
    let y0: number = Math.floor(baseY0 / ALIGNER) * ALIGNER;
    let x1: number = Math.ceil(baseX1 / ALIGNER) * ALIGNER;
    let y1: number = Math.ceil(baseY1 / ALIGNER) * ALIGNER;
    // 对齐结果可能越出文档边界（负值或超过 width/height），统一收敛回合法区间。
    x0 = this.clampDocX(x0, info.width);
    y0 = this.clampDocX(y0, info.height);
    x1 = this.clampDocX(x1, info.width);
    y1 = this.clampDocX(y1, info.height);
    const regionW: number = x1 - x0;
    const regionH: number = y1 - y0;
    if (regionW <= 0 || regionH <= 0 || regionW * regionH > MAX_DETAIL_PIXELS) {
      return;
    }
    // 缓存键：版本号 + 渲染区域（C-RENDER-API@1 约定）。
    const key = this.docRevision + ':' + x0 + ':' + y0 + ':' + regionW + ':' + regionH;
    const cachedPixels = this.detailPixels;
    // 命中条件：键一致 + 缓存字节仍被持有 + 字节长度与本次区域严格匹配（防御脏缓存）。
    // detailPixels === undefined 时条件自然不成立，直接退化为下面的 miss 重新渲染。
    if (key === this.detailKey && cachedPixels !== undefined && cachedPixels.byteLength === regionW * regionH * 4) {
      // 命中：版本与区域都没变，不再调用引擎（renderCalls 不增，R8.1 / R11.3③）；
      // 但必须用缓存字节重建一张新的 PixelMap——旧的 PixelMap 对象及其共享的字节可能已不可信。
      if (this.debugCacheCheck) {
        // 自检：比对缓存字节与引擎真值，判定差异来自缓存内容还是绘制状态（默认关闭）。
        this.runCacheCheck(cachedPixels, x0, y0, regionW, regionH);
      }
      this.rebuildDetail(key, cachedPixels, x0, y0, regionW, regionH);
      return;
    }
    const token = ++this.renderToken;
    const startedAt = Date.now();
    try {
      // 纯像素接口：返回裸 RGBA8，不再读取 8 字节尺寸头，也不做 slice(8) 拷贝（R10.1）。
      const pixels = Engine.renderPixels(this.docId, x0, y0, regionW, regionH);
      this.renderCalls++;
      const expected = regionW * regionH * 4;
      if (pixels.byteLength !== expected) {
        this.status = '细节渲染失败：像素长度 ' + pixels.byteLength + ' 与 ' + regionW + '×' + regionH + ' 不符';
        this.perf('renderDetail rev=' + this.docRevision + ' key=' + key + ' hit=false calls=' + this.renderCalls +
          ' ms=' + (Date.now() - startedAt) + ' w=' + regionW + ' h=' + regionH + ' bad=' + pixels.byteLength);
        return;
      }
      const options: image.InitializationOptions = {
        size: { width: regionW, height: regionH },
        pixelFormat: image.PixelMapFormat.RGBA_8888,
        alphaType: image.AlphaType.UNPREMUL
      };
      image.createPixelMap(pixels, options).then((map: image.PixelMap) => {
        if (token !== this.renderToken) {
          this.safeRelease(map);
          return;
        }
        const previous = this.pixelMap;
        // 缓存槽在 PixelMap 提交之前写入，并长期强引用这份字节：
        // PixelMap 与 pixels 零拷贝共享内存，只有 pixels 活着，PixelMap 的内容才可靠。
        this.detailPixels = pixels;
        this.detailKey = key;
        this.pixelMap = map;
        this.displayPixels = pixels;
        this.usingDetail = true;
        this.regionX = x0;
        this.regionY = y0;
        this.regionW = regionW;
        this.regionH = regionH;
        if (previous !== undefined && previous !== map) {
          // 旧显示像素（预览图或上一张细节图）只回收对象本身：其字节源仍由缓存槽或 displayPixels 持有。
          this.safeRelease(previous);
        }
        this.redrawCanvas();
        this.perf('renderDetail rev=' + this.docRevision + ' key=' + key + ' hit=false calls=' + this.renderCalls +
          ' ms=' + (Date.now() - startedAt) + ' w=' + regionW + ' h=' + regionH);
      });
    } catch (err) {
      this.status = '细节渲染失败：' + this.messageOf(err as Object);
      this.perf('renderDetail rev=' + this.docRevision + ' key=' + key + ' hit=false calls=' + this.renderCalls +
        ' ms=' + (Date.now() - startedAt) + ' error=true');
    }
  }

  // 命中缓存后只用缓存字节重建 PixelMap：不请求引擎（renderCalls 不增），画面与首次 miss 完全一致（R8.1 / R11.3③）。
  private rebuildDetail(key: string, source: ArrayBuffer, x0: number, y0: number, regionW: number, regionH: number): void {
    const token = ++this.renderToken;
    const options: image.InitializationOptions = {
      size: { width: regionW, height: regionH },
      pixelFormat: image.PixelMapFormat.RGBA_8888,
      alphaType: image.AlphaType.UNPREMUL
    };
    image.createPixelMap(source, options).then((map: image.PixelMap) => {
      if (token !== this.renderToken || key !== this.detailKey) {
        // 期间有更新的渲染请求，或缓存已被失效（revision 变化）：本张作废，避免旧区域/旧内容盖住新画面。
        this.safeRelease(map);
        return;
      }
      const previous = this.pixelMap;
      this.pixelMap = map;
      // source 同时被局部强引用与 detailPixels 槽持有，重建出的 PixelMap 内容可靠。
      this.displayPixels = source;
      this.usingDetail = true;
      this.regionX = x0;
      this.regionY = y0;
      this.regionW = regionW;
      this.regionH = regionH;
      if (previous !== undefined && previous !== map) {
        // 旧显示像素（通常是预览图）只回收对象本身，其字节源仍由 previewPixels / displayPixels 持有。
        this.safeRelease(previous);
      }
      this.detailHits++;
      this.redrawCanvas();
      // 命中口径：引擎零调用，因此 ms=0（该字段度量引擎渲染耗时，不含 PixelMap 重建）。
      // bytes= 为附加诊断字段：确认命中时确实持有与区域等长的缓存字节。
      this.perf('renderDetail rev=' + this.docRevision + ' key=' + key + ' hit=true calls=' + this.renderCalls +
        ' ms=0 hits=' + this.detailHits + ' w=' + regionW + ' h=' + regionH + ' bytes=' + source.byteLength);
    }).catch((err: Error) => {
      if (token !== this.renderToken) {
        return;
      }
      // 缓存字节不可用：作废缓存（键 + 字节），下一次调度按 miss 重新渲染，避免反复失败。
      this.detailKey = '';
      this.detailPixels = undefined;
      this.status = '细节渲染失败：' + this.messageOf(err as Object);
      this.perf('renderDetail rev=' + this.docRevision + ' key=' + key + ' hit=true calls=' + this.renderCalls +
        ' ms=0 hits=' + this.detailHits + ' error=true');
    });
  }

  private async refreshPreview(): Promise<void> {
    if (this.docId === 0) {
      return;
    }
    const side = this.isWide ? 2048 : 1280;
    // 缓存键：版本号 + 预览边长（C-RENDER-API@1 约定）。
    const key = this.docRevision + ':' + side;
    const cached = this.previewPixels;
    const cachedBytes = this.previewPixelW * this.previewPixelH * 4;
    // 命中条件：键一致 + 缓存字节仍被持有 + 字节长度与预览尺寸匹配（防御脏缓存）。
    if (key === this.previewKey && cached !== undefined && cachedBytes > 0 && cached.byteLength === cachedBytes) {
      if (this.pixelMap !== undefined && !this.usingDetail && this.displayPixels === cached) {
        // 当前显示的正是这份预览字节：无需任何重建，真正的零开销命中。
        this.previewHits++;
        this.perf('preview rev=' + this.docRevision + ' key=' + key + ' hit=true calls=' + this.previewCalls +
          ' ms=0 hits=' + this.previewHits);
        return;
      }
      // 缓存有效但当前显示的是细节图：同样不请求引擎，用缓存字节重建一张预览 PixelMap。
      await this.rebuildPreview(key, cached);
      return;
    }
    const token = ++this.renderToken;
    const startedAt = Date.now();
    try {
      const bytes = Engine.preview(this.docId, side);
      this.previewCalls++;
      const view = new DataView(bytes);
      const width = view.getUint32(0, true);
      const height = view.getUint32(4, true);
      if (width === 0 || height === 0) {
        return;
      }
      const options: image.InitializationOptions = {
        size: { width: width, height: height },
        pixelFormat: image.PixelMapFormat.RGBA_8888,
        alphaType: image.AlphaType.UNPREMUL
      };
      // slice(8) 跳过 8 字节尺寸头并产生一次独立拷贝：拷贝出的字节由 previewPixels 长期持有，
      // 保证 PixelMap 背后的预览内存不被 GC 回收（与 detail 路径同一所有权模型）。
      const pixels: ArrayBuffer = bytes.slice(8);
      const next = await image.createPixelMap(pixels, options);
      if (token !== this.renderToken) {
        this.safeRelease(next);
        return;
      }
      const previous = this.pixelMap;
      // 细节像素已由 detailPixels 槽持有，切回预览时只需回收旧的细节 PixelMap 对象本身，不再做转存。
      this.previewPixels = pixels;
      this.previewKey = key;
      this.pixelMap = next;
      this.displayPixels = pixels;
      this.usingDetail = false;
      this.regionX = 0;
      this.regionY = 0;
      this.regionW = width;
      this.regionH = height;
      this.previewPixelW = width;
      this.previewPixelH = height;
      if (previous !== undefined && previous !== next) {
        this.safeRelease(previous);
      }
      this.syncSelection();
      this.perf('preview rev=' + this.docRevision + ' key=' + key + ' hit=false calls=' + this.previewCalls +
        ' ms=' + (Date.now() - startedAt));
      // 仅在确实需要细节时调度，避免“预览 → 调度 → 不满足 → 再次预览”的循环（R7.2）。
      if (this.needsDetail()) {
        this.scheduleDetail();
      }
    } catch (err) {
      this.status = '预览渲染失败：' + this.messageOf(err as Object);
      this.perf('preview rev=' + this.docRevision + ' key=' + key + ' hit=false calls=' + this.previewCalls +
        ' ms=' + (Date.now() - startedAt) + ' error=true');
    }
  }

  // 预览缓存命中但当前显示的是细节图时：用缓存字节重建预览 PixelMap（不请求引擎）。
  private async rebuildPreview(key: string, source: ArrayBuffer): Promise<void> {
    const width = this.previewPixelW;
    const height = this.previewPixelH;
    const token = ++this.renderToken;
    const options: image.InitializationOptions = {
      size: { width: width, height: height },
      pixelFormat: image.PixelMapFormat.RGBA_8888,
      alphaType: image.AlphaType.UNPREMUL
    };
    try {
      const map = await image.createPixelMap(source, options);
      if (token !== this.renderToken || key !== this.previewKey) {
        // 期间有更新的渲染请求，或预览缓存已被失效：本张作废，避免旧内容盖住新画面。
        this.safeRelease(map);
        return;
      }
      const previous = this.pixelMap;
      this.pixelMap = map;
      this.displayPixels = source;
      this.usingDetail = false;
      this.regionX = 0;
      this.regionY = 0;
      this.regionW = width;
      this.regionH = height;
      if (previous !== undefined && previous !== map) {
        // 旧的细节 PixelMap 对象回收；它的字节仍由 detailPixels 槽持有，下次 1:1 命中可继续重建。
        this.safeRelease(previous);
      }
      this.previewHits++;
      this.syncSelection();
      this.perf('preview rev=' + this.docRevision + ' key=' + key + ' hit=true calls=' + this.previewCalls +
        ' ms=0 hits=' + this.previewHits);
      if (this.needsDetail()) {
        this.scheduleDetail();
      }
    } catch (err) {
      if (token !== this.renderToken) {
        return;
      }
      // 缓存字节不可用：作废预览缓存，下一次按 miss 重新请求引擎。
      this.previewKey = '';
      this.previewPixels = undefined;
      this.status = '预览渲染失败：' + this.messageOf(err as Object);
      this.perf('preview rev=' + this.docRevision + ' key=' + key + ' hit=true calls=' + this.previewCalls +
        ' ms=0 hits=' + this.previewHits + ' error=true');
    }
  }

  private async afterEdit(): Promise<void> {
    if (this.docId === 0) {
      return;
    }
    this.editCalls++;
    let revision = this.docRevision;
    try {
      revision = Engine.revision(this.docId);
    } catch (err) {
      this.status = this.messageOf(err as Object);
      return;
    }
    if (revision === this.docRevision && this.info !== undefined) {
      // 版本号未变化（失败操作/重复提交）：跳过元数据拉取与渲染（R9.2）。
      this.perf('afterEdit rev=' + revision + ' key=- hit=true calls=' + this.editCalls + ' ms=0 skip=true');
      return;
    }
    try {
      const info = Engine.info(this.docId);
      // 元数据成功后才提交版本号，避免 info 失败时版本号与界面状态错位。
      this.docRevision = revision;
      // 版本变化后旧的细节字节不再匹配任何键：清键并释放细节缓存槽（R8.2）。
      // 预览缓存键自带 revision 前缀，天然失效，这里不动；正在显示的旧像素由 displayPixels 继续保活。
      this.detailKey = '';
      this.detailPixels = undefined;
      this.info = info;
      this.layers = flattenLayers(info.layers, 0);
    } catch (err) {
      this.status = this.messageOf(err as Object);
      return;
    }
    const startedAt = Date.now();
    const detailNeeded = this.needsDetail();
    if (detailNeeded) {
      // 单路径渲染：需要细节时只调度细节，不与预览叠加（R7.2）。
      this.scheduleDetail();
    } else {
      await this.refreshPreview();
    }
    this.perf('afterEdit rev=' + revision + ' key=- hit=false calls=' + this.editCalls +
      ' ms=' + (Date.now() - startedAt) + ' detail=' + detailNeeded);
    // 文档已刷新：按当前选中层回填蒙版状态（C-MASK-UI@1 §D.2，必须在 info/layers 更新之后）。
    if (this.selectedLayerId !== 0 && this.maskSupported()) {
      this.loadMask(this.selectedLayerId);
    }
    this.scheduleThumbRefresh();
    // 每次成功编辑后同步历史面板（只读标签，不额外触发渲染）。
    this.loadHistory();
  }

  // 历史记录只读刷新：失败静默（历史面板不是主流程，不能因它中断编辑反馈）。
  private loadHistory(): void {
    if (this.docId === 0) {
      this.historyUndo = [];
      this.historyRedo = [];
      return;
    }
    try {
      const labels = Engine.history(this.docId);
      this.historyUndo = labels.undo;
      this.historyRedo = labels.redo;
    } catch (err) {
      this.historyUndo = [];
      this.historyRedo = [];
    }
  }

  // 历史跳转：index 是列表中的序号（0 = 最近一步），实际跳转步数为 index + 1。
  // 复用 undo()/redo() 的文档替换流程：清缓存、重置选中/草稿、回填画布尺寸、刷新历史。
  private async jumpHistory(kind: string, index: number): Promise<void> {
    if (this.docId === 0) {
      return;
    }
    const steps = index + 1;
    try {
      const jump = kind === 'undo'
        ? Engine.undoSteps(this.docId, steps)
        : Engine.redoSteps(this.docId, steps);
      this.syncDocRevision();
      this.resetRenderCache();
      this.info = jump.info;
      if (this.cropMode) {
        this.cropDiscard();
      }
      this.layers = flattenLayers(jump.info.layers, 0);
      this.selectedLayerId = 0;
      this.adjust = emptyAdjustment();
      this.editReset();
      this.maskReset();
      await this.refreshPreview();
      this.syncSelection();
      this.syncCanvasDraft();
      this.loadHistory();
      this.status = (kind === 'undo' ? '已撤销 ' : '已重做 ') + jump.steps + ' 步';
    } catch (err) {
      this.status = '历史跳转失败：' + this.messageOf(err as Object);
    }
  }

  private selectTool(tool: string): void {
    // 裁剪模式是画布态：切工具视为放弃裁剪（R1.6），退出后再按既有语义切换。
    if (this.cropMode) {
      this.cropCancel();
    }
    // 变换模式同为画布态：切工具视为放弃变换（丢弃预览、不写文档）。
    if (this.trMode) {
      this.trCancel();
    }
    this.cancelMove();
    this.tool = tool;
    if (tool === 'pan') {
      this.cancelStroke();
      this.cancelSelect();
      this.cancelShape();
      this.status = '移动工具：单指拖动平移，双指缩放';
    } else if (tool === 'brush') {
      this.cancelSelect();
      this.cancelShape();
      this.status = '画笔：单指在画布上绘制，双指缩放';
    } else if (tool === 'select') {
      this.cancelStroke();
      this.cancelShape();
      this.status = '选区：单指拖拽创建选区，双指缩放；画笔与滤镜受选区限制';
    } else if (tool === 'shape') {
      this.cancelStroke();
      this.cancelSelect();
      this.status = '形状：拖拽创建' + this.shapeLabel() + '，双指缩放';
    } else if (tool === 'text') {
      this.cancelStroke();
      this.cancelSelect();
      this.cancelShape();
      this.status = '文字：点击画布放置文字，双指缩放';
    } else if (tool === 'move') {
      // 移动工具：不落笔、不建选区、不建形状；提示当前是否具备可移动的选中图层（R3.3 / R3.4）。
      this.cancelStroke();
      this.cancelSelect();
      this.cancelShape();
      this.status = this.editMoverHint();
    } else {
      this.cancelSelect();
      this.cancelShape();
      this.status = this.selectedLayerId === 0 ? '橡皮：请先在图层列表选中像素图层' : '橡皮：单指擦除，双指缩放';
    }
  }

  private hexToRgba(hex: string): number[] {
    const value = parseInt(hex.substring(1), 16);
    const r = ((value >> 16) & 255) / 255;
    const g = ((value >> 8) & 255) / 255;
    const b = (value & 255) / 255;
    // The engine's colour arrays reach the raster pipeline as BGR on HarmonyOS.
    return [b, g, r, 1];
  }

  private clearBrushPreview(): void {
    try {
      this.brushCtx.clearRect(0, 0, this.viewportW, this.viewportH);
    } catch (err) {
      // The canvas may not be ready yet.
    }
    this.previewDot = false;
    this.previewDirtyW = 0;
    this.previewDirtyH = 0;
  }

  private clearPreviewDirty(): void {
    if (this.previewDirtyW <= 0 || this.previewDirtyH <= 0) {
      return;
    }
    try {
      this.brushCtx.clearRect(
        this.previewDirtyX - 6,
        this.previewDirtyY - 6,
        this.previewDirtyW + 12,
        this.previewDirtyH + 12
      );
    } catch (err) {
      // The canvas may not be ready yet.
    }
    this.previewDirtyW = 0;
    this.previewDirtyH = 0;
  }

  private markPreviewDirty(x: number, y: number, w: number, h: number): void {
    this.previewDirtyX = x;
    this.previewDirtyY = y;
    this.previewDirtyW = Math.max(0, w);
    this.previewDirtyH = Math.max(0, h);
  }

  private beginStroke(x: number, y: number): void {
    if (this.docId === 0 || this.info === undefined || this.displayScale() <= 0) {
      return;
    }
    this.clearBrushPreview();
    this.strokePoints = [];
    this.strokeStartMs = Date.now();
    this.drawing = true;
    this.appendStroke(x, y);
  }

  private appendStroke(x: number, y: number): void {
    const info = this.info;
    const scale = this.displayScale();
    if (info === undefined || scale <= 0) {
      return;
    }
    const docX = (x - this.displayLeft()) / scale + this.regionX;
    const docY = (y - this.displayTop()) / scale + this.regionY;
    if (docX < 0 || docY < 0 || docX > info.width || docY > info.height) {
      return;
    }
    const points = this.strokePoints;
    if (points.length > 0) {
      const last = points[points.length - 1];
      const dx = docX - last.x;
      const dy = docY - last.y;
      if (dx * dx + dy * dy < 2.0) {
        return;
      }
    }
    points.push({ x: docX, y: docY, pressure: 1, time: Date.now() - this.strokeStartMs });
    this.drawBrushPreview(x, y);
  }

  private drawBrushPreview(x: number, y: number): void {
    const radius = Math.max(0.5, this.brushSize * this.displayScale() / 2);
    const color = this.tool === 'eraser' ? 'rgba(255, 120, 120, 0.45)' : this.brushColor;
    const ctx = this.brushCtx;
    ctx.lineCap = 'round';
    ctx.lineJoin = 'round';
    ctx.lineWidth = radius * 2;
    ctx.strokeStyle = color;
    ctx.fillStyle = color;
    if (!this.previewDot) {
      ctx.beginPath();
      ctx.arc(x, y, radius, 0, Math.PI * 2);
      ctx.fill();
      this.previewDot = true;
    } else {
      ctx.beginPath();
      ctx.moveTo(this.lastScreenX, this.lastScreenY);
      ctx.lineTo(x, y);
      ctx.stroke();
    }
    this.lastScreenX = x;
    this.lastScreenY = y;
  }

  private cancelStroke(): void {
    this.drawing = false;
    this.strokePoints = [];
    this.clearBrushPreview();
  }

  private async endStroke(): Promise<void> {
    if (!this.drawing) {
      return;
    }
    this.drawing = false;
    const points = this.strokePoints;
    this.strokePoints = [];
    this.clearBrushPreview();
    if (points.length === 0 || this.docId === 0) {
      return;
    }
    if (this.tool === 'eraser' && this.selectedLayerId === 0) {
      this.status = '橡皮：请先在图层列表选中像素图层';
      return;
    }
    try {
      const params: BrushParams = {
        size: this.brushSize,
        hardness: this.brushHardness / 100,
        opacity: this.brushOpacity / 100,
        flow: 1,
        spacing: 0.1,
        pressureSize: false,
        erase: this.tool === 'eraser',
        color: this.hexToRgba(this.brushColor)
      };
      const layerId = Engine.paintStroke(this.docId, this.selectedLayerId, JSON.stringify(params), JSON.stringify(points));
      this.selectedLayerId = layerId;
      await this.afterEdit();
      this.status = this.tool === 'eraser' ? '已擦除一笔' : '已绘制一笔';
    } catch (err) {
      this.status = '绘制失败：' + this.messageOf(err as Object);
    }
  }

  private screenToDocX(x: number): number {
    return (x - this.displayLeft()) / this.displayScale() + this.regionX;
  }

  private screenToDocY(y: number): number {
    return (y - this.displayTop()) / this.displayScale() + this.regionY;
  }

  private redrawCanvas(): void {
    if (this.drawing || this.selecting || this.shaping || this.movingLayer) {
      return;
    }
    // 裁剪模式：全屏清叠加层后绘制遮罩/框/手柄（R3.1/R3.2）。
    // 遮罩覆盖「视口 − 框」，框变化时新旧遮罩并集等于整个视口，脏矩形无收益（见计划 2.1 A3）。
    // 裁剪叠加层优先：同时把 selectionFrameDrawn 置 false，跳过选区绘制（R3.4）。
    if (this.cropMode) {
      this.clearBrushPreview();
      this.drawCropOverlay();
      this.selectionFrameDrawn = false;
      return;
    }
    // 变换手势模式：全屏清叠加层后绘制变换框/手柄/旋转手柄，并跳过选区绘制。
    if (this.trMode) {
      this.clearBrushPreview();
      this.drawTransformOverlay();
      this.selectionFrameDrawn = false;
      return;
    }
    // 早退条件：无选区、上一帧也没画过选区框、网格关闭、没有可见参考线且标尺关闭时，没有任何叠加层可画（V2）。
    // 保留 selectionFrameDrawn 的判定：选区被清除时仍要靠这里清屏，否则旧选区框会残留在画布上。
    // 参考线为空时同样早退：showGuides 默认开启，无参考线的常见状态下不能为此每帧全屏清屏。
    const guidesVisible = this.showGuides && (this.guidesH.length > 0 || this.guidesV.length > 0);
    if (!this.selectionActive && !this.selectionFrameDrawn && !this.showGrid && !guidesVisible &&
      !this.showRulers) {
      return;
    }
    this.clearBrushPreview();
    // 叠加层自下而上：网格 → 参考线 → 选区框 → 标尺条（标尺始终覆盖在最上层）。
    if (this.showGrid) {
      this.drawGridOverlay();
    }
    if (this.showGuides) {
      this.drawGuideLines();
    }
    if (this.selectionActive) {
      this.drawSelectionFrame();
    }
    if (this.showRulers) {
      this.drawRulers();
    }
    this.selectionFrameDrawn = this.selectionActive;
  }

  // 网格叠加层：按 gridSize（文档像素）换算屏幕间距，只画视口可见区。
  // 屏幕间距小于 GRID_MIN_SCREEN 时跳过——缩放较小时线会密到糊成一片，既看不清也拖慢重绘。
  private drawGridOverlay(): void {
    const scale = this.displayScale();
    if (scale <= 0 || this.viewportW <= 0 || this.viewportH <= 0) {
      return;
    }
    const step = this.gridSize * scale;
    if (step < GRID_MIN_SCREEN) {
      return;
    }
    const left = this.displayLeft();
    const top = this.displayTop();
    const docX1 = this.screenToDocX(this.viewportW);
    const docY1 = this.screenToDocY(this.viewportH);
    // 起点对齐到文档坐标网格：屏幕上第一条线由视口左上角对应的文档坐标向上取整得到。
    let gx = Math.ceil(this.screenToDocX(0) / this.gridSize) * this.gridSize;
    let gy = Math.ceil(this.screenToDocY(0) / this.gridSize) * this.gridSize;
    const ctx = this.brushCtx;
    ctx.save();
    ctx.strokeStyle = GRID_LINE_COLOR;
    ctx.lineWidth = 1;
    ctx.setLineDash([]);
    ctx.beginPath();
    while (gx <= docX1) {
      const sx = left + (gx - this.regionX) * scale;
      ctx.moveTo(sx, 0);
      ctx.lineTo(sx, this.viewportH);
      gx += this.gridSize;
    }
    while (gy <= docY1) {
      const sy = top + (gy - this.regionY) * scale;
      ctx.moveTo(0, sy);
      ctx.lineTo(this.viewportW, sy);
      gy += this.gridSize;
    }
    ctx.stroke();
    ctx.restore();
  }

  // 参考线叠加层：文档坐标 → 屏幕坐标与选区框换算完全一致（减 regionX/regionY），只画落在视口内的。
  private drawGuideLines(): void {
    const scale = this.displayScale();
    if (scale <= 0 || this.viewportW <= 0 || this.viewportH <= 0) {
      return;
    }
    const left = this.displayLeft();
    const top = this.displayTop();
    const ctx = this.brushCtx;
    ctx.save();
    ctx.strokeStyle = GUIDE_LINE_COLOR;
    ctx.lineWidth = 1;
    ctx.setLineDash([]);
    ctx.beginPath();
    for (let i = 0; i < this.guidesH.length; i++) {
      const y = top + (this.guidesH[i] - this.regionY) * scale;
      if (y < 0 || y > this.viewportH) {
        continue;
      }
      ctx.moveTo(0, y);
      ctx.lineTo(this.viewportW, y);
    }
    for (let i = 0; i < this.guidesV.length; i++) {
      const x = left + (this.guidesV[i] - this.regionX) * scale;
      if (x < 0 || x > this.viewportW) {
        continue;
      }
      ctx.moveTo(x, 0);
      ctx.lineTo(x, this.viewportH);
    }
    ctx.stroke();
    ctx.restore();
  }

  // 标尺刻度条叠加层：上/左尺带 + 自适应步长刻度。刻度位置在文档坐标上取整到步长的整数倍，
  // 因此 pan/zoom 后刻度与画布内容一起移动、不会漂移；全程零引擎调用。
  private drawRulers(): void {
    const scale = this.displayScale();
    if (scale <= 0 || this.viewportW <= 0 || this.viewportH <= 0) {
      return;
    }
    // 步长自适应：按候选档位取第一个屏幕间距够宽的；极端缩小时全部不满足，退回最大档避免密到糊成一片。
    const steps: number[] = [10, 20, 50, 100, 200, 500, 1000, 2000, 5000];
    let step = steps[steps.length - 1];
    for (let i = 0; i < steps.length; i++) {
      if (steps[i] * scale >= RULER_MIN_SPACING) {
        step = steps[i];
        break;
      }
    }
    const minor = step / 5;
    const left = this.displayLeft();
    const top = this.displayTop();
    const startDocX = this.screenToDocX(0);
    const startDocY = this.screenToDocY(0);
    const endDocX = this.screenToDocX(this.viewportW);
    const endDocY = this.screenToDocY(this.viewportH);
    const ctx = this.brushCtx;
    ctx.save();
    // 先铺满背景：上尺带、左尺带与左上角方格（单独填充，避免依赖前两次填充的覆盖关系）。
    ctx.fillStyle = RULER_BG;
    ctx.fillRect(0, 0, this.viewportW, RULER_SIZE_VP);
    ctx.fillRect(0, 0, RULER_SIZE_VP, this.viewportH);
    ctx.fillRect(0, 0, RULER_SIZE_VP, RULER_SIZE_VP);
    ctx.strokeStyle = RULER_TICK;
    ctx.lineWidth = 1;
    ctx.setLineDash([]);
    // 副刻度：短线 4vp，跳过主刻度位置（副刻度的第 5 个就是主刻度）。
    ctx.beginPath();
    let mx = Math.ceil(startDocX / minor) * minor;
    while (mx <= endDocX) {
      if (mx % step !== 0) {
        const sx = left + (mx - this.regionX) * scale;
        if (sx >= RULER_SIZE_VP && sx <= this.viewportW) {
          ctx.moveTo(sx, RULER_SIZE_VP);
          ctx.lineTo(sx, RULER_SIZE_VP - 4);
        }
      }
      mx += minor;
    }
    let my = Math.ceil(startDocY / minor) * minor;
    while (my <= endDocY) {
      if (my % step !== 0) {
        const sy = top + (my - this.regionY) * scale;
        if (sy >= RULER_SIZE_VP && sy <= this.viewportH) {
          ctx.moveTo(RULER_SIZE_VP, sy);
          ctx.lineTo(RULER_SIZE_VP - 4, sy);
        }
      }
      my += minor;
    }
    ctx.stroke();
    // 主刻度：长线 8vp，与副刻度分两次 stroke 以便使用不同线宽。
    ctx.beginPath();
    let gx = Math.ceil(startDocX / step) * step;
    while (gx <= endDocX) {
      const sx = left + (gx - this.regionX) * scale;
      if (sx >= RULER_SIZE_VP && sx <= this.viewportW) {
        ctx.moveTo(sx, RULER_SIZE_VP);
        ctx.lineTo(sx, RULER_SIZE_VP - 8);
      }
      gx += step;
    }
    let gy = Math.ceil(startDocY / step) * step;
    while (gy <= endDocY) {
      const sy = top + (gy - this.regionY) * scale;
      if (sy >= RULER_SIZE_VP && sy <= this.viewportH) {
        ctx.moveTo(RULER_SIZE_VP, sy);
        ctx.lineTo(RULER_SIZE_VP - 8, sy);
      }
      gy += step;
    }
    ctx.stroke();
    // 数字最后画，避免被尺带背景或刻度线覆盖。
    ctx.fillStyle = RULER_TEXT;
    ctx.font = '9vp sans-serif';
    let tx = Math.ceil(startDocX / step) * step;
    while (tx <= endDocX) {
      const sx = left + (tx - this.regionX) * scale;
      if (sx >= RULER_SIZE_VP && sx <= this.viewportW) {
        ctx.fillText(String(Math.round(tx)), sx + 3, 9);
      }
      tx += step;
    }
    let ty = Math.ceil(startDocY / step) * step;
    while (ty <= endDocY) {
      const sy = top + (ty - this.regionY) * scale;
      if (sy >= RULER_SIZE_VP && sy <= this.viewportH) {
        ctx.fillText(String(Math.round(ty)), 3, sy + 10);
      }
      ty += step;
    }
    ctx.restore();
  }

  private drawSelectionFrame(): void {
    if (!this.selectionActive) {
      return;
    }
    const scale = this.displayScale();
    if (scale <= 0) {
      return;
    }
    const x0 = Math.min(this.selX0, this.selX1);
    const y0 = Math.min(this.selY0, this.selY1);
    const x1 = Math.max(this.selX0, this.selX1);
    const y1 = Math.max(this.selY0, this.selY1);
    const left = this.displayLeft() + (x0 - this.regionX) * scale;
    const top = this.displayTop() + (y0 - this.regionY) * scale;
    const w = (x1 - x0) * scale;
    const h = (y1 - y0) * scale;
    if (w <= 0 || h <= 0) {
      return;
    }
    const ctx = this.brushCtx;
    ctx.save();
    ctx.strokeStyle = COLOR_SELECT_ACCENT;
    ctx.fillStyle = 'rgba(45, 212, 191, 0.12)';
    ctx.lineWidth = 1.5;
    ctx.setLineDash([6, 4]);
    if (this.selectionShape === 'ellipse' && this.selectionPathDoc.length >= 2) {
      const p0 = this.selectionPathDoc[0];
      const p1 = this.selectionPathDoc[1];
      const ex0 = Math.min(p0[0], p1[0]);
      const ey0 = Math.min(p0[1], p1[1]);
      const ex1 = Math.max(p0[0], p1[0]);
      const ey1 = Math.max(p0[1], p1[1]);
      const cx = this.displayLeft() + ((ex0 + ex1) / 2 - this.regionX) * scale;
      const cy = this.displayTop() + ((ey0 + ey1) / 2 - this.regionY) * scale;
      const rx = Math.max(0.5, (ex1 - ex0) * scale / 2);
      const ry = Math.max(0.5, (ey1 - ey0) * scale / 2);
      ctx.beginPath();
      ctx.ellipse(cx, cy, rx, ry, 0, 0, Math.PI * 2);
      ctx.fill();
      ctx.stroke();
    } else if (this.selectionShape === 'lasso' && this.selectionPathDoc.length > 2) {
      ctx.beginPath();
      const first = this.selectionPathDoc[0];
      ctx.moveTo(this.displayLeft() + (first[0] - this.regionX) * scale, this.displayTop() + (first[1] - this.regionY) * scale);
      for (let i = 1; i < this.selectionPathDoc.length; i++) {
        const p = this.selectionPathDoc[i];
        ctx.lineTo(this.displayLeft() + (p[0] - this.regionX) * scale, this.displayTop() + (p[1] - this.regionY) * scale);
      }
      ctx.closePath();
      ctx.fill();
      ctx.stroke();
    } else if (this.selectionShape === 'rect' || this.selectionShape === 'all') {
      ctx.fillRect(left, top, w, h);
      ctx.strokeRect(left, top, w, h);
    } else {
      ctx.strokeRect(left, top, w, h);
    }
    ctx.restore();
  }

  private drawSelectPreview(): void {
    if (this.displayScale() <= 0) {
      return;
    }
    const ctx = this.brushCtx;
    let bx = 0;
    let by = 0;
    let bw = 0;
    let bh = 0;
    let hasBox = false;
    ctx.save();
    ctx.strokeStyle = COLOR_SELECT_ACCENT;
    ctx.lineWidth = 1.5;
    ctx.setLineDash([6, 4]);
    if (this.selectionKind === 'lasso') {
      const pts = this.lassoPoints;
      if (pts.length > 1) {
        let minX = pts[0][0];
        let maxX = pts[0][0];
        let minY = pts[0][1];
        let maxY = pts[0][1];
        ctx.beginPath();
        ctx.moveTo(pts[0][0], pts[0][1]);
        for (let i = 1; i < pts.length; i++) {
          ctx.lineTo(pts[i][0], pts[i][1]);
          minX = Math.min(minX, pts[i][0]);
          maxX = Math.max(maxX, pts[i][0]);
          minY = Math.min(minY, pts[i][1]);
          maxY = Math.max(maxY, pts[i][1]);
        }
        ctx.stroke();
        bx = minX;
        by = minY;
        bw = maxX - minX;
        bh = maxY - minY;
        hasBox = true;
      }
    } else if (this.selectionKind === 'ellipse') {
      const x0 = Math.min(this.selectStartScreenX, this.selectCurrentScreenX);
      const y0 = Math.min(this.selectStartScreenY, this.selectCurrentScreenY);
      const w = Math.abs(this.selectCurrentScreenX - this.selectStartScreenX);
      const h = Math.abs(this.selectCurrentScreenY - this.selectStartScreenY);
      ctx.beginPath();
      ctx.ellipse(x0 + w / 2, y0 + h / 2, Math.max(0.5, w / 2), Math.max(0.5, h / 2), 0, 0, Math.PI * 2);
      ctx.stroke();
      bx = x0;
      by = y0;
      bw = w;
      bh = h;
      hasBox = true;
    } else if (this.selectionKind !== 'wand') {
      const x0 = Math.min(this.selectStartScreenX, this.selectCurrentScreenX);
      const y0 = Math.min(this.selectStartScreenY, this.selectCurrentScreenY);
      const w = Math.abs(this.selectCurrentScreenX - this.selectStartScreenX);
      const h = Math.abs(this.selectCurrentScreenY - this.selectStartScreenY);
      ctx.strokeRect(x0, y0, w, h);
      bx = x0;
      by = y0;
      bw = w;
      bh = h;
      hasBox = true;
    }
    ctx.restore();
    if (hasBox) {
      this.markPreviewDirty(bx, by, bw, bh);
    }
  }

  private beginSelect(x: number, y: number): void {
    if (this.docId === 0 || this.info === undefined || this.displayScale() <= 0) {
      return;
    }
    this.cancelStroke();
    this.selecting = true;
    this.selectStartScreenX = x;
    this.selectStartScreenY = y;
    this.selectCurrentScreenX = x;
    this.selectCurrentScreenY = y;
    this.lassoPoints = [];
    if (this.selectionKind === 'lasso') {
      this.lassoPoints.push([x, y]);
    }
    this.clearBrushPreview();
  }

  private updateSelect(x: number, y: number): void {
    if (!this.selecting) {
      return;
    }
    this.selectCurrentScreenX = x;
    this.selectCurrentScreenY = y;
    if (this.selectionKind === 'lasso') {
      const pts = this.lassoPoints;
      const last = pts.length > 0 ? pts[pts.length - 1] : undefined;
      if (last === undefined || Math.abs(last[0] - x) + Math.abs(last[1] - y) > 2) {
        pts.push([x, y]);
      }
    }
    this.clearPreviewDirty();
    this.drawSelectPreview();
  }

  private cancelSelect(): void {
    this.selecting = false;
    this.lassoPoints = [];
    this.clearBrushPreview();
    this.drawSelectionFrame();
  }

  private shapeLabel(): string {
    if (this.shapeKind === 'ellipse') {
      return '椭圆';
    }
    if (this.shapeKind === 'line') {
      return '直线';
    }
    return '矩形';
  }

  private beginShape(x: number, y: number): void {
    if (this.docId === 0 || this.info === undefined || this.displayScale() <= 0) {
      return;
    }
    this.cancelStroke();
    this.cancelSelect();
    this.shaping = true;
    this.shapeStartScreenX = x;
    this.shapeStartScreenY = y;
    this.shapeCurrentScreenX = x;
    this.shapeCurrentScreenY = y;
    this.clearBrushPreview();
  }

  private updateShape(x: number, y: number): void {
    if (!this.shaping) {
      return;
    }
    this.shapeCurrentScreenX = x;
    this.shapeCurrentScreenY = y;
    this.clearPreviewDirty();
    this.drawShapePreview();
  }

  private cancelShape(): void {
    this.shaping = false;
    this.clearBrushPreview();
    this.drawSelectionFrame();
  }

  private drawShapePreview(): void {
    const scale = this.displayScale();
    if (scale <= 0) {
      return;
    }
    const ctx = this.brushCtx;
    let bx = 0;
    let by = 0;
    let bw = 0;
    let bh = 0;
    ctx.save();
    ctx.strokeStyle = COLOR_SHAPE_ACCENT;
    ctx.fillStyle = 'rgba(99, 102, 241, 0.18)';
    ctx.lineWidth = 1.5;
    if (this.shapeKind === 'line') {
      const lw = Math.max(1.5, this.shapeLineWidth * scale);
      ctx.lineWidth = lw;
      ctx.beginPath();
      ctx.moveTo(this.shapeStartScreenX, this.shapeStartScreenY);
      ctx.lineTo(this.shapeCurrentScreenX, this.shapeCurrentScreenY);
      ctx.stroke();
      const x0 = Math.min(this.shapeStartScreenX, this.shapeCurrentScreenX);
      const y0 = Math.min(this.shapeStartScreenY, this.shapeCurrentScreenY);
      bx = x0 - lw / 2;
      by = y0 - lw / 2;
      bw = Math.abs(this.shapeCurrentScreenX - this.shapeStartScreenX) + lw;
      bh = Math.abs(this.shapeCurrentScreenY - this.shapeStartScreenY) + lw;
    } else if (this.shapeKind === 'ellipse') {
      const x0 = Math.min(this.shapeStartScreenX, this.shapeCurrentScreenX);
      const y0 = Math.min(this.shapeStartScreenY, this.shapeCurrentScreenY);
      const w = Math.abs(this.shapeCurrentScreenX - this.shapeStartScreenX);
      const h = Math.abs(this.shapeCurrentScreenY - this.shapeStartScreenY);
      ctx.setLineDash([6, 4]);
      ctx.beginPath();
      ctx.ellipse(x0 + w / 2, y0 + h / 2, Math.max(0.5, w / 2), Math.max(0.5, h / 2), 0, 0, Math.PI * 2);
      ctx.fill();
      ctx.stroke();
      bx = x0;
      by = y0;
      bw = w;
      bh = h;
    } else {
      const x0 = Math.min(this.shapeStartScreenX, this.shapeCurrentScreenX);
      const y0 = Math.min(this.shapeStartScreenY, this.shapeCurrentScreenY);
      const w = Math.abs(this.shapeCurrentScreenX - this.shapeStartScreenX);
      const h = Math.abs(this.shapeCurrentScreenY - this.shapeStartScreenY);
      ctx.setLineDash([6, 4]);
      ctx.fillRect(x0, y0, w, h);
      ctx.strokeRect(x0, y0, w, h);
      bx = x0;
      by = y0;
      bw = w;
      bh = h;
    }
    ctx.restore();
    this.markPreviewDirty(bx, by, bw, bh);
  }

  private async commitShape(): Promise<void> {
    if (!this.shaping) {
      return;
    }
    this.shaping = false;
    const docId = this.docId;
    if (docId === 0 || this.info === undefined || this.displayScale() <= 0) {
      this.clearBrushPreview();
      this.drawSelectionFrame();
      return;
    }
    const moved = Math.abs(this.shapeCurrentScreenX - this.shapeStartScreenX) + Math.abs(this.shapeCurrentScreenY - this.shapeStartScreenY);
    if (moved < 4) {
      this.clearBrushPreview();
      this.drawSelectionFrame();
      return;
    }
    const startDocX = this.screenToDocX(this.shapeStartScreenX);
    const startDocY = this.screenToDocY(this.shapeStartScreenY);
    const endDocX = this.screenToDocX(this.shapeCurrentScreenX);
    const endDocY = this.screenToDocY(this.shapeCurrentScreenY);
    const fill = this.hexToRgba(this.shapeFillColor);
    fill[3] = this.shapeFillOpacity / 100;
    if (this.shapeKind === 'line' && fill[3] <= 0) {
      fill[3] = 1;
    }
    const stroke = this.hexToRgba(this.shapeStrokeColor);
    const params: ShapeParams = {
      kind: this.shapeKind,
      x: Math.min(startDocX, endDocX),
      y: Math.min(startDocY, endDocY),
      w: Math.abs(endDocX - startDocX),
      h: Math.abs(endDocY - startDocY),
      fromX: startDocX,
      fromY: startDocY,
      toX: endDocX,
      toY: endDocY,
      weight: this.shapeLineWidth,
      radius: this.shapeRadius,
      fill: fill,
      stroke: stroke,
      strokeWidth: this.shapeStrokeWidth,
      name: ''
    };
    const label = this.shapeLabel();
    try {
      const outcome = Engine.shapeCreate(docId, JSON.stringify(params));
      this.info = outcome.info;
      this.layers = flattenLayers(outcome.info.layers, 0);
      this.selectedLayerId = outcome.layerId;
      this.clearLayerThumbCache(outcome.layerId);
      this.loadLayerThumb(outcome.layerId);
      // 形状创建改变了文档内容：先同步版本号，预览缓存键才会正确失效（R8.2）。
      this.syncDocRevision();
      await this.refreshPreview();
      this.status = '已创建形状：' + label;
    } catch (err) {
      this.clearBrushPreview();
      this.drawSelectionFrame();
      this.status = '形状失败：' + this.messageOf(err as Object);
    }
  }

  private async commitText(x: number, y: number): Promise<void> {
    if (this.docId === 0 || this.info === undefined) {
      return;
    }
    if (this.textContent.trim().length === 0) {
      this.status = '文字：请输入内容';
      return;
    }
    const params: TextParams = {
      text: this.textContent,
      fontFamily: this.textFontFamily,
      size: this.textSize,
      x: this.screenToDocX(x),
      y: this.screenToDocY(y),
      color: this.hexToRgba(this.textColor),
      name: ''
    };
    try {
      const outcome = Engine.textCreate(this.docId, JSON.stringify(params));
      this.info = outcome.info;
      this.layers = flattenLayers(outcome.info.layers, 0);
      this.selectedLayerId = outcome.layerId;
      this.clearLayerThumbCache(outcome.layerId);
      this.loadLayerThumb(outcome.layerId);
      // 文字创建改变了文档内容：先同步版本号，预览缓存键才会正确失效（R8.2）。
      this.syncDocRevision();
      await this.refreshPreview();
      this.status = '已创建文字图层';
    } catch (err) {
      this.status = '文字失败：' + this.messageOf(err as Object);
    }
  }

  private async commitSelect(): Promise<void> {
    if (!this.selecting) {
      return;
    }
    this.selecting = false;
    const docId = this.docId;
    if (docId === 0 || this.info === undefined) {
      this.clearBrushPreview();
      return;
    }
    const kind = this.selectionKind;
    const scale = this.displayScale();
    if (scale <= 0) {
      this.clearBrushPreview();
      return;
    }
    let params: SelectParams;
    let shapePath: number[][] = [];
    if (kind === 'wand') {
      params = {
        kind: kind,
        x0: 0, y0: 0, x1: 0, y1: 0,
        mode: this.selectionMode,
        feather: this.selectionFeather,
        points: [],
        x: Math.round(this.screenToDocX(this.selectStartScreenX)),
        y: Math.round(this.screenToDocY(this.selectStartScreenY)),
        tolerance: this.wandTolerance * 2.55,
        contiguous: true
      };
    } else if (kind === 'lasso') {
      const pts = this.lassoPoints;
      if (pts.length < 3) {
        this.clearBrushPreview();
        return;
      }
      const docPts: number[][] = [];
      for (const p of pts) {
        docPts.push([this.screenToDocX(p[0]), this.screenToDocY(p[1])]);
      }
      shapePath = docPts;
      params = {
        kind: kind,
        x0: 0, y0: 0, x1: 0, y1: 0,
        mode: this.selectionMode,
        feather: this.selectionFeather,
        points: docPts,
        x: 0, y: 0,
        tolerance: 0,
        contiguous: true
      };
    } else {
      const moved = Math.abs(this.selectCurrentScreenX - this.selectStartScreenX) + Math.abs(this.selectCurrentScreenY - this.selectStartScreenY);
      if (moved < 4) {
        this.clearBrushPreview();
        return;
      }
      const x0 = this.screenToDocX(Math.min(this.selectStartScreenX, this.selectCurrentScreenX));
      const y0 = this.screenToDocY(Math.min(this.selectStartScreenY, this.selectCurrentScreenY));
      const x1 = this.screenToDocX(Math.max(this.selectStartScreenX, this.selectCurrentScreenX));
      const y1 = this.screenToDocY(Math.max(this.selectStartScreenY, this.selectCurrentScreenY));
      shapePath = [[x0, y0], [x1, y1]];
      params = {
        kind: kind,
        x0: x0, y0: y0, x1: x1, y1: y1,
        mode: this.selectionMode,
        feather: this.selectionFeather,
        points: [],
        x: 0, y: 0,
        tolerance: 0,
        contiguous: true
      };
    }
    try {
      const info = Engine.select(docId, JSON.stringify(params));
      const displayShape = this.selectionMode === 'replace' ? kind : 'bounds';
      this.applySelection(info, displayShape, shapePath);
      this.refreshInfoOnly();
      const modeText = this.selectionMode === 'add' ? '相加' : (this.selectionMode === 'subtract' ? '减去' : '新建');
      this.status = '已创建选区（' + modeText + '）';
    } catch (err) {
      this.clearBrushPreview();
      this.status = '选区失败：' + this.messageOf(err as Object);
    }
  }

  private applySelection(info: SelectionInfo, shape: string, path: number[][]): void {
    this.selectionActive = info.active;
    this.selX0 = info.x0;
    this.selY0 = info.y0;
    this.selX1 = info.x1;
    this.selY1 = info.y1;
    this.selectionShape = info.active ? shape : 'bounds';
    this.selectionPathDoc = path;
    this.selectionKey = info.x0 + ',' + info.y0 + ',' + info.x1 + ',' + info.y1;
    this.redrawCanvas();
  }

  private syncSelection(): void {
    if (this.docId === 0) {
      this.selectionActive = false;
      this.redrawCanvas();
      return;
    }
    try {
      const info = Engine.selectionInfo(this.docId);
      const key = info.x0 + ',' + info.y0 + ',' + info.x1 + ',' + info.y1;
      if (info.active && key === this.selectionKey) {
        this.applySelection(info, this.selectionShape, this.selectionPathDoc);
      } else {
        this.applySelection(info, 'bounds', []);
      }
    } catch (err) {
      this.selectionActive = false;
      this.redrawCanvas();
    }
  }

  private refreshInfoOnly(): void {
    if (this.docId === 0) {
      return;
    }
    try {
      this.info = Engine.info(this.docId);
    } catch (err) {
      // The status line keeps the previous error.
    }
  }

  private async runSelect(kind: string): Promise<void> {
    if (this.docId === 0) {
      promptAction.showToast({ message: '请先打开一张图片' });
      return;
    }
    try {
      const params: SelectParams = {
        kind: kind,
        x0: 0, y0: 0, x1: 0, y1: 0,
        mode: 'replace',
        feather: 0,
        points: [],
        x: 0, y: 0,
        tolerance: 0,
        contiguous: true
      };
      const info = Engine.select(this.docId, JSON.stringify(params));
      this.applySelection(info, 'bounds', []);
      this.refreshInfoOnly();
      if (kind === 'all') {
        this.status = '已全选';
      } else if (kind === 'none') {
        this.status = '已取消选区';
      } else {
        this.status = '已反选';
      }
    } catch (err) {
      this.status = '选区操作失败：' + this.messageOf(err as Object);
    }
  }

  private filterLabel(): string {
    switch (this.filterKind) {
      case 'gaussianBlur':
        return '高斯模糊';
      case 'motionBlur':
        return '动感模糊';
      case 'smartSharpen':
        return '智能锐化';
      case 'addNoise':
        return '添加杂色';
      case 'mosaic':
        return '马赛克';
      case 'emboss':
        return '浮雕';
      case 'twirl':
        return '旋转扭曲';
      case 'invert':
        return '反相';
      case 'desaturate':
        return '去色';
      default:
        return '查找边缘';
    }
  }

  private currentFilterArgs(): FilterArgs {
    let radius = 0;
    let angle = 0;
    let amount = 0;
    if (this.filterKind === 'gaussianBlur') {
      radius = this.filterBlurRadius;
    } else if (this.filterKind === 'smartSharpen') {
      radius = this.filterSharpenRadius;
      amount = this.filterSharpenAmount;
    } else if (this.filterKind === 'motionBlur') {
      angle = this.filterMotionAngle;
    } else if (this.filterKind === 'emboss') {
      angle = this.filterEmbossAngle;
      amount = this.filterEmbossAmount;
    } else if (this.filterKind === 'addNoise') {
      amount = this.filterNoiseAmount;
    } else if (this.filterKind === 'twirl') {
      angle = this.filterTwirlAngle;
    }
    return {
      filter: this.filterKind,
      radius: radius,
      angle: angle,
      distance: this.filterMotionDistance,
      amount: amount,
      cellSize: this.filterCellSize,
      height: this.filterEmbossHeight,
      reduceNoise: 10,
      distribution: 'gaussian',
      monochromatic: this.filterMono,
      seed: 12345
    };
  }

  private canFilter(): boolean {
    if (this.docId === 0 || this.selectedLayerId === 0) {
      return false;
    }
    for (const layer of this.layers) {
      if (layer.id === this.selectedLayerId) {
        return layer.kind === 'Pixel';
      }
    }
    return false;
  }

  private async applyFilter(): Promise<void> {
    if (this.docId === 0) {
      promptAction.showToast({ message: '请先打开一张图片' });
      return;
    }
    if (!this.canFilter()) {
      this.status = '滤镜：请先在图层列表选择一个像素图层';
      return;
    }
    const label = this.filterLabel();
    try {
      const info = Engine.filterApply(this.docId, this.selectedLayerId, JSON.stringify(this.currentFilterArgs()));
      this.info = info;
      this.layers = flattenLayers(info.layers, 0);
      // 滤镜改变了文档内容：先同步版本号，预览缓存键才会正确失效（R8.2）。
      this.syncDocRevision();
      await this.refreshPreview();
      this.status = '已应用滤镜：' + label;
    } catch (err) {
      this.status = '滤镜失败：' + this.messageOf(err as Object);
    }
  }

  private filterParamValue(name: string): number {
    switch (name) {
      case 'blurRadius':
        return this.filterBlurRadius;
      case 'motionAngle':
        return this.filterMotionAngle;
      case 'motionDistance':
        return this.filterMotionDistance;
      case 'sharpenAmount':
        return this.filterSharpenAmount;
      case 'sharpenRadius':
        return this.filterSharpenRadius;
      case 'noiseAmount':
        return this.filterNoiseAmount;
      case 'cellSize':
        return this.filterCellSize;
      case 'embossAngle':
        return this.filterEmbossAngle;
      case 'embossHeight':
        return this.filterEmbossHeight;
      case 'embossAmount':
        return this.filterEmbossAmount;
      case 'twirlAngle':
        return this.filterTwirlAngle;
      default:
        return 0;
    }
  }

  private filterParamText(name: string): string {
    const value = this.filterParamValue(name);
    return (Math.round(value * 10) / 10).toString();
  }

  private setFilterParam(name: string, value: number): void {
    if (name === 'blurRadius') {
      this.filterBlurRadius = value;
    } else if (name === 'motionAngle') {
      this.filterMotionAngle = value;
    } else if (name === 'motionDistance') {
      this.filterMotionDistance = value;
    } else if (name === 'sharpenAmount') {
      this.filterSharpenAmount = value;
    } else if (name === 'sharpenRadius') {
      this.filterSharpenRadius = value;
    } else if (name === 'noiseAmount') {
      this.filterNoiseAmount = value;
    } else if (name === 'cellSize') {
      this.filterCellSize = value;
    } else if (name === 'embossAngle') {
      this.filterEmbossAngle = value;
    } else if (name === 'embossHeight') {
      this.filterEmbossHeight = value;
    } else if (name === 'embossAmount') {
      this.filterEmbossAmount = value;
    } else if (name === 'twirlAngle') {
      this.filterTwirlAngle = value;
    }
  }

  private scheduleThumbRefresh(): void {
    if (this.selectedLayerId === 0) {
      return;
    }
    if (this.thumbTimer !== -1) {
      clearTimeout(this.thumbTimer);
    }
    this.thumbTimer = setTimeout(() => {
      this.thumbTimer = -1;
      if (this.selectedLayerId !== 0) {
        this.clearLayerThumbCache(this.selectedLayerId);
        this.loadLayerThumb(this.selectedLayerId);
      }
    }, 400);
  }

  private clearLayerThumbCache(layerId: number): void {
    const cached = this.layerThumbCache.get(layerId);
    if (cached === undefined) {
      return;
    }
    this.layerThumbCache.delete(layerId);
    if (this.layerThumb === cached) {
      this.layerThumb = undefined;
    }
    this.layerThumbVersion++;
    this.safeRelease(cached);
  }

  private clearAllLayerThumbs(): void {
    const cached: image.PixelMap[] = [];
    for (const thumb of this.layerThumbCache.values()) {
      cached.push(thumb);
    }
    this.thumbQueueToken++;
    this.layerThumbCache.clear();
    this.layerThumb = undefined;
    this.layerThumbVersion++;
    for (const thumb of cached) {
      this.safeRelease(thumb);
    }
  }

  private async ensureLayerThumb(layerId: number): Promise<boolean> {
    if (this.docId === 0) {
      return false;
    }
    if (this.layerThumbCache.has(layerId) || this.thumbPending.has(layerId)) {
      return false;
    }
    const docId = this.docId;
    this.thumbPending.add(layerId);
    try {
      const bytes = Engine.layerThumbnail(docId, layerId, 96);
      const view = new DataView(bytes);
      const width = view.getUint32(0, true);
      const height = view.getUint32(4, true);
      if (width === 0 || height === 0) {
        return false;
      }
      const options: image.InitializationOptions = {
        size: { width: width, height: height },
        pixelFormat: image.PixelMapFormat.RGBA_8888,
        alphaType: image.AlphaType.UNPREMUL
      };
      const next = await image.createPixelMap(bytes.slice(8), options);
      if (this.docId !== docId) {
        next.release();
        return false;
      }
      this.layerThumbCache.set(layerId, next);
      return true;
    } catch (err) {
      // Group / adjustment / empty layers legitimately have no thumbnail.
      return false;
    } finally {
      this.thumbPending.delete(layerId);
    }
  }

  private async loadLayerThumb(layerId: number): Promise<void> {
    const token = ++this.thumbToken;
    const created = await this.ensureLayerThumb(layerId);
    if (token !== this.thumbToken) {
      return;
    }
    const cached = this.layerThumbCache.get(layerId);
    if (cached !== undefined) {
      this.layerThumb = cached;
      if (created) {
        this.layerThumbVersion++;
      }
    }
  }

  private async loadAllLayerThumbs(): Promise<void> {
    if (this.docId === 0) {
      return;
    }
    const token = ++this.thumbQueueToken;
    const docId = this.docId;
    const pending: LayerInfo[] = [];
    for (const layer of this.layers) {
      if (pending.length >= 12) {
        break;
      }
      if (layer.kind === 'Pixel' || layer.kind === 'Shape' || layer.kind === 'Type') {
        pending.push(layer);
      }
    }
    const step = async (index: number): Promise<void> => {
      if (token !== this.thumbQueueToken || this.docId !== docId || index >= pending.length) {
        return;
      }
      const created = await this.ensureLayerThumb(pending[index].id);
      if (token !== this.thumbQueueToken || this.docId !== docId) {
        return;
      }
      if (created) {
        this.layerThumbVersion++;
      }
      setTimeout(() => {
        step(index + 1);
      }, 16);
    };
    setTimeout(() => {
      step(0);
    }, 0);
  }

  private selectLayer(layer: LayerInfo): void {
    this.selectedLayerId = layer.id;
    this.layerOpacity = Math.round(layer.opacity * 100);
    this.layerFillOpacity = Math.round((layer.fillOpacity ?? layer.opacity) * 100);
    this.layerBlend = layer.blend ?? 'Normal';
    this.layerNameDraft = layer.name;
    this.loadLayerThumb(layer.id);
    if (layer.kind === 'Adjustment') {
      try {
        this.adjust = Engine.adjustment(this.docId, layer.id);
      } catch (err) {
        this.adjust = emptyAdjustment();
      }
    } else {
      this.adjust = emptyAdjustment();
    }
    // 切换图层时同步加载该层的效果：失败只写状态栏，不抛到 UI 线程。
    this.loadFx(layer.id);
    // 同一时刻按新图层类型回填编辑分组（非文字/形状图层会自动清空分组，R5.4）。
    this.loadEdit(layer.id);
    // 切换图层时同步回填蒙版状态（C-MASK-UI@1 §D.1）。
    this.loadMask(layer.id);
    // 切层：上一层的变换草稿不能延续到新层（C-XFORM-UI@1 §3.5）。
    this.trResetDraft();
  }

  private async toggleVisible(layer: LayerInfo): Promise<void> {
    try {
      Engine.setLayerVisible(this.docId, layer.id, !layer.visible);
      await this.afterEdit();
    } catch (err) {
      this.status = this.messageOf(err as Object);
    }
  }

  private blendSuffix(mode: string): string {
    if (mode === '' || mode === 'Normal' || mode === 'PassThrough') {
      return '';
    }
    for (const option of BLEND_OPTIONS) {
      if (option.value === mode) {
        return ' · ' + option.label;
      }
    }
    return ' · ' + mode;
  }

  private async renameSelectedLayer(): Promise<void> {
    if (this.docId === 0 || this.selectedLayerId === 0) {
      return;
    }
    const name = this.layerNameDraft.trim();
    if (name.length === 0) {
      this.status = '图层名称不能为空';
      return;
    }
    try {
      Engine.renameLayer(this.docId, this.selectedLayerId, name);
      await this.afterEdit();
      this.status = '已重命名为：' + name;
    } catch (err) {
      this.status = '重命名失败：' + this.messageOf(err as Object);
    }
  }

  private async moveSelectedLayer(delta: number): Promise<void> {
    if (this.docId === 0 || this.selectedLayerId === 0) {
      return;
    }
    try {
      const info = Engine.moveLayer(this.docId, this.selectedLayerId, delta);
      this.info = info;
      this.layers = flattenLayers(info.layers, 0);
      await this.refreshPreview();
      this.status = delta > 1 ? '图层已置顶' : (delta === 1 ? '图层已上移' : (delta < -1 ? '图层已置底' : '图层已下移'));
    } catch (err) {
      this.status = '排序失败：' + this.messageOf(err as Object);
    }
  }

  private async duplicateSelectedLayer(): Promise<void> {
    if (this.docId === 0 || this.selectedLayerId === 0) {
      return;
    }
    try {
      const outcome = Engine.duplicateLayer(this.docId, this.selectedLayerId);
      this.info = outcome.info;
      this.layers = flattenLayers(outcome.info.layers, 0);
      this.selectedLayerId = outcome.layerId;
      this.clearLayerThumbCache(outcome.layerId);
      this.loadLayerThumb(outcome.layerId);
      for (const layer of this.layers) {
        if (layer.id === outcome.layerId) {
          this.layerNameDraft = layer.name;
          this.layerOpacity = Math.round(layer.opacity * 100);
          this.layerFillOpacity = Math.round((layer.fillOpacity ?? layer.opacity) * 100);
          this.layerBlend = layer.blend ?? 'Normal';
          break;
        }
      }
      await this.refreshPreview();
      this.status = '已复制图层';
    } catch (err) {
      this.status = '复制失败：' + this.messageOf(err as Object);
    }
  }

  private async mergeSelectedLayer(): Promise<void> {
    if (this.docId === 0 || this.selectedLayerId === 0) {
      return;
    }
    try {
      const outcome = Engine.mergeLayer(this.docId, this.selectedLayerId);
      this.info = outcome.info;
      this.layers = flattenLayers(outcome.info.layers, 0);
      this.selectedLayerId = outcome.layerId;
      this.clearLayerThumbCache(outcome.layerId);
      this.loadLayerThumb(outcome.layerId);
      for (const layer of this.layers) {
        if (layer.id === outcome.layerId) {
          this.layerNameDraft = layer.name;
          this.layerOpacity = Math.round(layer.opacity * 100);
          this.layerFillOpacity = Math.round((layer.fillOpacity ?? layer.opacity) * 100);
          this.layerBlend = layer.blend ?? 'Normal';
          break;
        }
      }
      await this.refreshPreview();
      this.status = '已向下合并图层';
    } catch (err) {
      this.status = '合并失败：' + this.messageOf(err as Object);
    }
  }

  private async setLayerBlendMode(mode: string): Promise<void> {
    if (this.docId === 0 || this.selectedLayerId === 0) {
      return;
    }
    this.layerBlend = mode;
    try {
      Engine.setLayerBlend(this.docId, this.selectedLayerId, mode);
      await this.afterEdit();
      this.status = '已切换混合模式';
    } catch (err) {
      this.status = '混合模式失败：' + this.messageOf(err as Object);
    }
  }

  // ===== 剪贴蒙版（C2）=====
  // 当前选中层的剪贴状态：按 id 在扁平列表查行，查不到（无选中 / 已折叠）返回 false。
  // 只读 this.layers，不缓存副本，保证 afterEdit 刷新列表后 chip 与图层行标记同步更新。
  private selectedLayerClipped(): boolean {
    const row = this.layerRowById(this.selectedLayerId);
    return row !== undefined && row.clipped;
  }

  // 切换剪贴蒙版：原生层负责一次 checkpoint，UI 侧成功后只走一次 afterEdit；
  // 失败仅写状态栏（不乐观更新本地草稿，避免面板与实际文档不一致）。
  private async toggleLayerClipped(): Promise<void> {
    if (this.docId === 0 || this.selectedLayerId === 0) {
      return;
    }
    const row = this.layerRowById(this.selectedLayerId);
    if (row === undefined) {
      return;
    }
    const next = !row.clipped;
    try {
      // 以原生返回的实际状态为准（同值空操作时也一致），文案不依赖请求值。
      const applied = Engine.layerClipped(this.docId, row.id, next);
      await this.afterEdit();
      this.status = applied ? '已启用剪贴蒙版' : '已取消剪贴蒙版';
    } catch (err) {
      this.status = '剪贴蒙版失败：' + this.messageOf(err as Object);
    }
  }

  private async commitLayerFillOpacity(value: number): Promise<void> {
    if (this.docId === 0 || this.selectedLayerId === 0) {
      return;
    }
    try {
      Engine.setLayerFillOpacity(this.docId, this.selectedLayerId, value / 100);
      await this.afterEdit();
      this.status = '填充不透明度：' + Math.round(value) + '%';
    } catch (err) {
      this.status = '填充不透明度失败：' + this.messageOf(err as Object);
    }
  }

  private async commitOpacity(value: number): Promise<void> {
    if (this.docId === 0 || this.selectedLayerId === 0) {
      return;
    }
    try {
      Engine.setLayerOpacity(this.docId, this.selectedLayerId, value / 100);
      await this.afterEdit();
    } catch (err) {
      this.status = this.messageOf(err as Object);
    }
  }

  private async removeSelected(): Promise<void> {
    if (this.docId === 0 || this.selectedLayerId === 0) {
      return;
    }
    try {
      const removed = this.selectedLayerId;
      Engine.deleteLayer(this.docId, removed);
      this.clearLayerThumbCache(removed);
      this.selectedLayerId = 0;
      this.adjust = emptyAdjustment();
      // 图层已删除：清空 FX 子面板数据与折叠状态。
      this.fxReset();
      // 已删除图层的可编辑参数不能残留（R4.5）。
      this.editReset();
      this.trResetDraft();
      await this.afterEdit();
    } catch (err) {
      this.status = this.messageOf(err as Object);
    }
  }

  // ===== 图层组（G2）=====
  // 数据流：建组 / 解组 → Engine.layerGroup / layerUngroup（原生层 checkpoint）→ afterEdit 重读并渲染；
  // 折叠开关 → Engine.layerExpanded（不 checkpoint，docRevision 不变）→ refreshLayersOnly 只重读图层列表。

  // 折叠 / 展开组：expanded 由原生层持有，UI 不维护本地镜像；调用后重读文档元数据，
  // 由 flattenLayers 依据新的 expanded 决定是否展开子层（G1 契约）。
  private async toggleGroupExpanded(layer: LayerInfo): Promise<void> {
    if (this.docId === 0 || !layer.isGroup) {
      return;
    }
    const expanded = !layer.expanded;
    try {
      Engine.layerExpanded(this.docId, layer.id, expanded);
      // layerExpanded 不 checkpoint：docRevision 不变，afterEdit 会命中「rev 未变化 → skip」分支而
      // 不刷新 this.layers，所以这里走 refreshLayersOnly（Engine.info + flattenLayers，与 afterEdit 内同一路径）。
      if (!this.refreshLayersOnly()) {
        return;
      }
      // 折叠后若选中层被藏进组内（已不在扁平列表），把选中切到该组本身：避免选中状态与列表脱节，
      // 也保证下一次「建组」能在 this.layers 里找到选中层的 depth。
      if (!expanded && this.layerRowById(this.selectedLayerId) === undefined) {
        const groupRow = this.layerRowById(layer.id);
        if (groupRow !== undefined) {
          this.selectLayer(groupRow);
        }
      }
      this.status = expanded ? '已展开组「' + layer.name + '」' : '已折叠组「' + layer.name + '」';
    } catch (err) {
      this.status = '折叠切换失败：' + this.messageOf(err as Object);
    }
  }

  // 只重读文档元数据并重建扁平图层列表：不渲染、不改渲染缓存（折叠不改变任何像素）。
  // 返回是否成功：调用方据此决定是否覆盖状态栏，避免刷新失败被后续成功文案掩盖。
  private refreshLayersOnly(): boolean {
    if (this.docId === 0) {
      return false;
    }
    try {
      const info = Engine.info(this.docId);
      this.info = info;
      this.layers = flattenLayers(info.layers, 0);
      return true;
    } catch (err) {
      this.status = '刷新图层列表失败：' + this.messageOf(err as Object);
      return false;
    }
  }

  // 按 id 在扁平列表找层；找不到返回 undefined（用于折叠后校验选中层是否仍可见）。
  private layerRowById(layerId: number): LayerInfo | undefined {
    if (layerId === 0) {
      return undefined;
    }
    for (const row of this.layers) {
      if (row.id === layerId) {
        return row;
      }
    }
    return undefined;
  }

  // 建组目标 id 列表（G2 §2）：selectedLayerId + 其「下方」所有同级层（depth 相同且位于其后）。
  // 扁平列表下标 0 为顶层，所以「下方」= 下标递增方向：
  // - 遇到 depth < baseDepth：已跳出当前父容器（父级的下一个兄弟），立即停止，禁止跨容器成组；
  // - depth > baseDepth：选中层（或已收集到的同级组的）子层，随父层整体进入新组，不单独列出；
  // - depth === baseDepth：同级层，收集（选中层本身固定排第一位）。
  private groupCandidateIds(): number[] {
    const ids: number[] = [];
    if (this.selectedLayerId === 0) {
      return ids;
    }
    let start = -1;
    for (let i = 0; i < this.layers.length; i++) {
      if (this.layers[i].id === this.selectedLayerId) {
        start = i;
        break;
      }
    }
    if (start < 0) {
      return ids;
    }
    const baseDepth = this.layers[start].depth;
    ids.push(this.layers[start].id);
    for (let i = start + 1; i < this.layers.length; i++) {
      const row = this.layers[i];
      if (row.depth < baseDepth) {
        break;
      }
      if (row.depth === baseDepth) {
        ids.push(row.id);
      }
    }
    return ids;
  }

  // 新组名：'组 N'，N 从「当前顶层组数 + 1」起；若同名已存在则继续递增，避免重名。
  private nextGroupName(): string {
    let index = this.topGroupCount() + 1;
    let name = '组 ' + index;
    while (this.layerNameExists(name)) {
      index++;
      name = '组 ' + index;
    }
    return name;
  }

  private topGroupCount(): number {
    let count = 0;
    for (const row of this.layers) {
      if (row.isGroup && row.depth === 0) {
        count++;
      }
    }
    return count;
  }

  private layerNameExists(name: string): boolean {
    for (const row of this.layers) {
      if (row.name === name) {
        return true;
      }
    }
    return false;
  }

  // 建组：一次 layerGroup = 一次 checkpoint，成功后只调用一次 afterEdit 刷新图层与画布。
  // 新建空白像素图层（layer.new.layer 的鸿蒙子集）：插到当前选中层之上，未选中则置顶。
  // 新层是空 Raster，选中它并重置文字/形状编辑与蒙版面板，避免沿用上一层的草稿。
  private async createLayer(): Promise<void> {
    if (this.docId === 0 || this.busy) {
      return;
    }
    try {
      const outcome = Engine.createLayer(this.docId, this.selectedLayerId, '');
      this.selectedLayerId = outcome.layerId;
      this.editReset();
      this.maskReset();
      await this.afterEdit();
      this.status = '已新建图层';
    } catch (err) {
      this.status = '新建图层失败：' + this.messageOf(err as Object);
    }
  }

  private async createLayerGroup(): Promise<void> {
    if (this.docId === 0 || this.selectedLayerId === 0) {
      this.status = '建组：请先在图层列表选中图层';
      return;
    }
    const ids = this.groupCandidateIds();
    if (ids.length === 0) {
      return;
    }
    try {
      Engine.layerGroup(this.docId, ids, this.nextGroupName());
      await this.afterEdit();
      this.status = '已创建组（' + ids.length + ' 个图层）';
    } catch (err) {
      this.status = '建组失败：' + this.messageOf(err as Object);
    }
  }

  // 解组：仅选中的层是组时可用；子层数量必须在 afterEdit 前取（刷新后该组已不存在）。
  private async ungroupSelected(): Promise<void> {
    if (this.docId === 0 || this.selectedLayerId === 0) {
      this.status = '解组：请先在图层列表选中一个组';
      return;
    }
    const groupRow = this.layerRowById(this.selectedLayerId);
    if (groupRow === undefined || !groupRow.isGroup) {
      this.status = '解组：当前选中的图层不是组';
      return;
    }
    const childCount = groupRow.children.length;
    try {
      Engine.layerUngroup(this.docId, groupRow.id);
      // 组图层已被删除，原 selectedLayerId 失效：清空选中并重置编辑/蒙版状态，
      // 否则面板会残留一个已不存在的图层名（曾观察到解组后仍显示「组 1」）。
      this.selectedLayerId = 0;
      this.editReset();
      this.maskReset();
      await this.afterEdit();
      this.status = '已解散组（' + childCount + ' 个图层）';
    } catch (err) {
      this.status = '解组失败：' + this.messageOf(err as Object);
    }
  }

  // 建组可用条件：已打开文档 + 选中了图层 + 列表中确实有图层（G2 §4）。
  private groupEnabled(): boolean {
    return this.docId !== 0 && this.selectedLayerId !== 0 && this.layers.length > 0;
  }

  // 解组可用条件：在「建组可用」之上，选中层 isGroup 为 true（G2 §3）。
  private ungroupEnabled(): boolean {
    if (!this.groupEnabled()) {
      return false;
    }
    const row = this.layerRowById(this.selectedLayerId);
    return row !== undefined && row.isGroup;
  }

  // ===== 图层多选与批量操作（S1）=====
  // 数据流：入口按钮切换 multiSelectMode → 点击图层行切换 multiSelected（不调 selectLayer）
  // → 批量按钮调用既有 Engine.deleteLayer / duplicateLayer / layerGroup → 成功后只调一次 afterEdit。

  private isMultiSelected(layerId: number): boolean {
    return this.multiSelected.indexOf(layerId) >= 0;
  }

  // 行内选中态（背景 / 左边条 / 名称色共用）：多选模式以勾选为准，不再显示单选高亮，
  // 避免同一行里出现两种截然不同的选中态造成混淆。
  private layerRowSelected(layerId: number): boolean {
    if (this.multiSelectMode) {
      return this.isMultiSelected(layerId);
    }
    return layerId === this.selectedLayerId;
  }

  // 行背景：多选选中用 #33445c（区别于单选 #2a3d55），未选中与既有底色一致。
  private layerRowBackground(layerId: number): string {
    if (this.multiSelectMode) {
      return this.isMultiSelected(layerId) ? '#33445c' : '#1d2834';
    }
    return layerId === this.selectedLayerId ? '#2a3d55' : '#1d2834';
  }

  // 行左边条宽度：选中行 3、未选中 0；颜色始终是 COLOR_LAYER_ACCENT。
  private layerRowAccentWidth(layerId: number): number {
    return this.layerRowSelected(layerId) ? 3 : 0;
  }

  // 进入 / 退出多选：两种切换都清空已选，避免沿用上一轮选择造成误删。
  // 退出后 selectedLayerId 保持不变，单选面板与图层行高亮自然恢复。
  private toggleMultiSelectMode(): void {
    this.multiSelectMode = !this.multiSelectMode;
    this.multiSelected = [];
    this.status = this.multiSelectMode ? '多选模式：点击图层行勾选，可批量删除/复制/建组' : '已退出多选';
  }

  // 勾选 / 取消勾选单行：数组整体重新赋值，确保 @State 观测到变化并让 ForEach 重建该行。
  private toggleMultiSelected(layerId: number): void {
    const next: number[] = [];
    let removed = false;
    for (const id of this.multiSelected) {
      if (id === layerId) {
        removed = true;
      } else {
        next.push(id);
      }
    }
    if (!removed) {
      next.push(layerId);
    }
    this.multiSelected = next;
  }

  private multiAllSelected(): boolean {
    if (this.layers.length === 0) {
      return false;
    }
    for (const row of this.layers) {
      if (!this.isMultiSelected(row.id)) {
        return false;
      }
    }
    return true;
  }

  // 全选当前扁平列表里的行 / 清空已选。折叠组的子层不在扁平列表内，因此不参与全选。
  private toggleMultiSelectAll(): void {
    if (this.multiAllSelected()) {
      this.multiSelected = [];
      return;
    }
    const ids: number[] = [];
    for (const row of this.layers) {
      ids.push(row.id);
    }
    this.multiSelected = ids;
  }

  // 批量成功后的统一收尾：退出多选并清空已选（退出后恢复单选，selectedLayerId 不变）。
  private exitMultiSelect(): void {
    this.multiSelectMode = false;
    this.multiSelected = [];
  }

  // 复制 id 列表：批量操作遍历的是快照，避免执行过程中状态变化影响循环范围。
  private copyIds(ids: number[]): number[] {
    const out: number[] = [];
    for (const id of ids) {
      out.push(id);
    }
    return out;
  }

  // 判断某层是否有「也被选中的祖先」：批量删除父组时子层随父组一并消失，
  // 再单独删该子层必然报错，所以删除前先剔除这类后代，只对选中集里的根层调用 deleteLayer。
  private hasSelectedAncestor(layerId: number): boolean {
    const info = this.info;
    if (info === undefined) {
      return false;
    }
    return this.findSelectedAncestor(info.layers, layerId, false);
  }

  private findSelectedAncestor(nodes: LayerInfo[], layerId: number, ancestorSelected: boolean): boolean {
    for (const node of nodes) {
      if (node.id === layerId) {
        return ancestorSelected;
      }
      if (this.findSelectedAncestor(node.children, layerId, ancestorSelected || this.isMultiSelected(node.id))) {
        return true;
      }
    }
    return false;
  }

  // 批量删除（D2）：一次 layerDeleteMany = 一次 checkpoint，撤销可整批恢复；
  // 不再逐个 deleteLayer（旧实现每层一次 checkpoint，撤销一次只能恢复一层）。
  // 提交前仍沿用既有「剔除被勾选祖先的后代」逻辑，只把选中集里的根层交给原生，
  // 与原生内部的后代剔除互为兜底；deleted 为原生返回的真实删除数，不再自行重复计数。
  // deleted === 0 时不刷新、不清空多选，状态栏给出原因，便于用户保留选择重试。
  private async deleteMultiSelected(): Promise<void> {
    if (this.docId === 0 || this.multiSelected.length === 0) {
      return;
    }
    const targets: number[] = [];
    for (const id of this.copyIds(this.multiSelected)) {
      if (!this.hasSelectedAncestor(id)) {
        targets.push(id);
      }
    }
    if (targets.length === 0) {
      // 防御分支：每次勾选至少有一个根层，正常不会出现；避免把空数组交给原生。
      this.status = '批量删除失败：没有可删除的图层';
      return;
    }
    // 单选层是否也在本次勾选内：删除前记录，避免刷新后按列表反查不到而误判（折叠组内的层不在扁平列表）。
    const selectedWillBeRemoved = this.selectedLayerId !== 0 && this.isMultiSelected(this.selectedLayerId);
    try {
      const deleted: number = Engine.layerDeleteMany(this.docId, targets);
      if (deleted === 0) {
        // 一层都没删掉：文档未变，不调 afterEdit，也不清空多选。
        this.status = '批量删除失败：没有可删除的图层';
        return;
      }
      for (const layerId of targets) {
        this.clearLayerThumbCache(layerId);
      }
      // 单选层也在本次删除内：先清空单选再刷新，避免 afterEdit 按已不存在的图层回填蒙版/缩略图。
      // 选中面板以 selectedLayerId === 0 整体不渲染，因此不需要额外清理 FX / 编辑 / 蒙版草稿
      // （下一次 selectLayer 会按新图层重新载入），也就不触碰这些既有子系统的逻辑。
      if (selectedWillBeRemoved) {
        this.selectedLayerId = 0;
        this.adjust = emptyAdjustment();
      }
      await this.afterEdit();
      this.exitMultiSelect();
      this.status = '已删除 ' + deleted + ' 个图层';
    } catch (err) {
      this.status = '批量删除失败：' + this.messageOf(err as Object);
    }
  }

  // 批量复制：一次 duplicateLayerMany = 一次 checkpoint；失败保留多选。
  private async duplicateMultiSelected(): Promise<void> {
    if (this.docId === 0 || this.multiSelected.length === 0) {
      return;
    }
    try {
      const ids = this.copyIds(this.multiSelected);
      const outcome = Engine.duplicateLayerMany(this.docId, JSON.stringify(ids));
      for (const id of outcome.layerIds) {
        // 新副本的缩略图缓存键从空开始，清一次防止复用旧图。
        this.clearLayerThumbCache(id);
      }
      await this.afterEdit();
      this.exitMultiSelect();
      // 新副本没有缩略图缓存：走既有分帧队列补齐（已缓存层会被跳过，不重复生成）。
      this.loadAllLayerThumbs();
      this.status = '已复制 ' + outcome.layerIds.length + ' 个图层';
    } catch (err) {
      this.status = '批量复制失败：' + this.messageOf(err as Object);
    }
  }

  // 批量建组：一次 layerGroup = 一次 checkpoint；成功后只调用一次 afterEdit。
  // 失败保留多选（不清空、不退出），便于用户调整选择后重试。
  private async groupMultiSelected(): Promise<void> {
    if (this.docId === 0 || this.multiSelected.length === 0) {
      return;
    }
    const ids = this.copyIds(this.multiSelected);
    try {
      Engine.layerGroup(this.docId, ids, this.nextGroupName());
      await this.afterEdit();
      this.exitMultiSelect();
      this.status = '已创建组（' + ids.length + ' 个图层）';
    } catch (err) {
      this.status = '批量建组失败：' + this.messageOf(err as Object);
    }
  }

  // ===== 图层二次编辑（文字 / 形状）=====
  // 数据流：loadEdit 从原生层读 LayerEditInfo → @State 草稿 → 分组控件 → applyEdit 单次提交 → afterEdit。
  // 拖动期间（beginMove / updateMove）只写普通字段与 status，严禁 afterEdit / renderPixels / preview / scheduleDetail。

  // 清空编辑草稿与进行中的移动状态：文档切换、删除图层、撤销/重做、离开页面时调用。
  private editReset(): void {
    this.editKind = '';
    this.editText = '';
    this.editSize = 48;
    this.editSizeBase = 48;
    this.editFontFamily = 'Inter';
    this.editColor = '#ffffff';
    this.editShapeKind = '';
    this.editFillColor = '#e5484d';
    this.editFillEditable = true;
    this.editStrokeColor = '#ffffff';
    this.editStrokeEditable = true;
    this.editStrokeWidth = 0;
    this.editStrokeWidthBase = 0;
    this.editPosX = 0;
    this.editPosY = 0;
    this.cancelMove();
  }

  // 按 id 查图层 kind；查不到返回 ''（用于类型门控，避免"切换图层后旧分组还在"的错配）。
  private layerKindById(layerId: number): string {
    for (const layer of this.layers) {
      if (layer.id === layerId) {
        return layer.kind;
      }
    }
    return '';
  }

  private selectedLayerKind(): string {
    if (this.selectedLayerId === 0) {
      return '';
    }
    return this.layerKindById(this.selectedLayerId);
  }

  // 读取单层可编辑参数并回填草稿。
  // 非 Type/Shape 图层不发引擎调用（R5.5 / R9.4）：避免选中像素/组图层时出现"读取失败"噪声。
  private loadEdit(layerId: number): void {
    if (this.docId === 0 || layerId === 0) {
      this.editReset();
      return;
    }
    const kind = this.layerKindById(layerId);
    if (kind !== 'Type' && kind !== 'Shape') {
      this.editReset();
      return;
    }
    try {
      this.fillEditDraft(Engine.layerEditGet(this.docId, layerId));
    } catch (err) {
      this.editReset();
      this.status = '读取图层参数失败：' + this.messageOf(err as Object);
    }
  }

  // 用引擎返回的 LayerEditInfo 回填全部草稿（纯赋值，不抛异常）。
  // 颜色字段是 [b,g,r,a]，一律经 rgbaToHex 转回 '#rrggbb'，不做通道交换（R7.1）。
  private fillEditDraft(edit: LayerEditInfo): void {
    this.editKind = edit.kind;
    this.editText = edit.text;
    this.editSize = edit.size;
    this.editSizeBase = edit.size;
    this.editFontFamily = edit.fontFamily === '' ? 'Inter' : edit.fontFamily;
    this.editColor = this.rgbaToHex(edit.color);
    this.editShapeKind = edit.shapeKind;
    this.editFillColor = this.rgbaToHex(edit.fillColor);
    this.editFillEditable = edit.fillEditable;
    this.editStrokeColor = this.rgbaToHex(edit.strokeColor);
    this.editStrokeEditable = edit.strokeEditable;
    this.editStrokeWidth = edit.strokeWidth;
    this.editStrokeWidthBase = edit.strokeWidth;
    this.editPosX = edit.x;
    this.editPosY = edit.y;
  }

  // 0..1 浮点分量 → 两位十六进制；NaN/Infinity/越界一律钳到 [0,1]，保证往返误差 ≤1/255。
  private channelHex(value: number): string {
    const finite = Number.isFinite(value) ? value : 0;
    const clamped = Math.min(1, Math.max(0, finite));
    const hex = Math.round(clamped * 255).toString(16);
    return hex.length < 2 ? '0' + hex : hex;
  }

  // [b, g, r, a]（引擎 BGR 顺序）→ '#rrggbb'；与 hexToRgba 互为逆运算，禁止任何通道交换（R7.1）。
  private rgbaToHex(color: number[]): string {
    const b = color.length > 0 ? color[0] : 0;
    const g = color.length > 1 ? color[1] : 0;
    const r = color.length > 2 ? color[2] : 0;
    return '#' + this.channelHex(r) + this.channelHex(g) + this.channelHex(b);
  }

  // 字体候选：内置 Inter + 已注册的系统字体族（可为空）。
  private editFontOptions(): string[] {
    const options: string[] = ['Inter'];
    if (this.textFontReady && this.textFontName !== '' && this.textFontName !== 'Inter') {
      options.push(this.textFontName);
    }
    return options;
  }

  // 分组可见性：必须"有选中层 + editKind 是 Type/Shape + editKind 与选中层真实 kind 一致"（R5.1 / R5.4）。
  private editGroupVisible(): boolean {
    if (this.selectedLayerId === 0 || (this.editKind !== 'Type' && this.editKind !== 'Shape')) {
      return false;
    }
    return this.editKind === this.selectedLayerKind();
  }

  // 切换工具时的入口提示；文案与 C-EDIT-UI@1 §F 一致。
  private editMoverHint(): string {
    if (this.selectedLayerId === 0) {
      return '移动：请先在图层列表选中文字或形状图层';
    }
    const kind = this.selectedLayerKind();
    if (kind !== 'Type' && kind !== 'Shape') {
      return '移动：当前选中的是 ' + kind + '，仅支持文字/形状图层';
    }
    return '移动：拖动后松手提交';
  }

  // 视觉（R5.3）：文字用青柠系、形状用靛蓝系，风格与 FX 子面板一致但配色可区分。
  private editAccent(): string {
    return this.editKind === 'Type' ? COLOR_TYPE_ACCENT : COLOR_SHAPE_ACCENT;
  }

  private editCardBg(): string {
    return this.editKind === 'Type' ? COLOR_TYPE_CARD : COLOR_SHAPE_CARD;
  }

  private editTitle(): string {
    return this.editKind === 'Type' ? '文字编辑' : '形状编辑';
  }

  private editTitleColor(): string {
    return this.editKind === 'Type' ? '#d9f99d' : '#c7d2fe';
  }

  private editMutedColor(): string {
    return this.editKind === 'Type' ? '#8fae63' : '#8a86c8';
  }

  // target 只接受常量 'text' / 'fill' / 'stroke'；选中态一律从这里读 this.* 状态（不传状态值给 Builder）。
  private editDotSelected(target: string, color: string): boolean {
    if (target === 'text') {
      return this.editColor === color;
    }
    if (target === 'fill') {
      return this.editFillColor === color;
    }
    if (target === 'stroke') {
      return this.editStrokeColor === color;
    }
    return false;
  }

  private editPickColor(target: string, color: string): void {
    if (target === 'text') {
      this.commitEditTextColor(color);
    } else if (target === 'fill') {
      this.commitEditFill(color);
    } else if (target === 'stroke') {
      this.commitEditStroke(color);
    }
  }

  // 色盘可用性：非纯色填充/描边置灰（R2.6）。
  private editPaletteEnabled(target: string): boolean {
    if (target === 'fill') {
      return this.editFillEditable;
    }
    if (target === 'stroke') {
      return this.editStrokeEditable;
    }
    return true;
  }

  // 统一提交：一次 apply = 一次 checkpoint（引擎侧保证）；成功后只调用一次 afterEdit（R8.2）。
  // 失败：引擎侧零副作用、画面不变；这里回读真实值并展示原生错误文本（R4.2 / R6.2）。
  private async applyEdit(patch: EditPatch, okText: string): Promise<void> {
    if (this.docId === 0 || this.selectedLayerId === 0) {
      this.status = '编辑：请先在图层列表选中文字或形状图层';
      return;
    }
    const layerId = this.selectedLayerId;
    try {
      // 诊断：记录提交补丁与 revision 变化，用于区分「重复提交同一值（无变化）」与
      // 「引擎已写回但渲染结果相同（如框文本溢出被裁切）」这两种都表现为画面不动的情形。
      const beforeRev = Engine.revision(this.docId);
      const payload = JSON.stringify(patch);
      const applied: LayerEditInfo = Engine.layerEditApply(this.docId, layerId, payload);
      const afterRev = Engine.revision(this.docId);
      this.perf('edit apply rev=' + beforeRev + '->' + afterRev + ' payload=' + payload);
      this.fillEditDraft(applied);
      await this.afterEdit();
      this.status = okText;
    } catch (err) {
      this.status = '编辑失败：' + this.messageOf(err as Object);
      this.loadEdit(layerId);
    }
  }

  // 文字内容只在点「应用」时提交，避免每敲一个字都 checkpoint（R1.2 / R1.6）。
  private async commitEditText(): Promise<void> {
    if (this.editText.trim() === '') {
      this.status = '文字内容不能为空';
      return;
    }
    await this.applyEdit({ text: this.editText }, '已更新文字内容');
  }

  // 字号/描边宽度滑块：与基线完全相同视为无变化（R4.3、§1.3 示例 1），不写文档。
  private async commitEditSize(value: number): Promise<void> {
    if (Math.abs(value - this.editSizeBase) < 0.01) {
      this.status = '字号未改变';
      return;
    }
    await this.applyEdit({ size: value }, '已更新字号 ' + Math.round(value) + 'pt');
  }

  private async commitEditTextColor(hex: string): Promise<void> {
    this.editColor = hex;
    await this.applyEdit({ color: this.hexToRgba(hex) }, '已更新文字颜色');
  }

  private async commitEditFont(family: string): Promise<void> {
    this.editFontFamily = family;
    await this.applyEdit({ fontFamily: family }, '已更新字体：' + family);
  }

  private async commitEditFill(hex: string): Promise<void> {
    if (!this.editFillEditable) {
      this.status = '渐变/图案填充暂不支持在此编辑';
      return;
    }
    this.editFillColor = hex;
    await this.applyEdit({ fillColor: this.hexToRgba(hex) }, '已更新形状填充');
  }

  private async commitEditStroke(hex: string): Promise<void> {
    if (!this.editStrokeEditable) {
      this.status = '该图层没有可编辑的描边';
      return;
    }
    // 当前无描边（宽度 0）时点击颜色默认给 1px，避免"点了没反应"（C-EDIT-UI@1 §B）。
    const width = this.editStrokeWidth > 0 ? this.editStrokeWidth : 1;
    this.editStrokeColor = hex;
    this.editStrokeWidth = width;
    await this.applyEdit({ strokeColor: this.hexToRgba(hex), strokeWidth: width }, '已更新描边颜色');
  }

  private async commitEditStrokeWidth(value: number): Promise<void> {
    if (Math.abs(value - this.editStrokeWidthBase) < 0.01) {
      this.status = '描边宽度未改变';
      return;
    }
    const okText = value <= 0 ? '已移除描边' : '已更新描边宽度 ' + Math.round(value) + 'px';
    await this.applyEdit({ strokeWidth: value, strokeColor: this.hexToRgba(this.editStrokeColor) }, okText);
  }

  // ----- 移动工具（C-EDIT-UI@1 §D）：Down 只记录起点，Move 只更新位移与状态栏，Up 单次提交 -----

  private beginMove(x: number, y: number): void {
    if (this.docId === 0 || this.info === undefined || this.displayScale() <= 0) {
      return;
    }
    if (this.selectedLayerId === 0) {
      this.status = '移动：请先在图层列表选中文字或形状图层';
      return;
    }
    const kind = this.selectedLayerKind();
    if (kind !== 'Type' && kind !== 'Shape') {
      this.status = '移动：当前选中的是 ' + kind + '，仅支持文字/形状图层';
      return;
    }
    this.moveStartScreenX = x;
    this.moveStartScreenY = y;
    this.moveCurrentScreenX = x;
    this.moveCurrentScreenY = y;
    this.moveStartDocX = this.screenToDocX(x);
    this.moveStartDocY = this.screenToDocY(y);
    this.moveDx = 0;
    this.moveDy = 0;
    this.movingLayer = true;
    this.status = '移动：拖动后松手提交';
  }

  // 只在拖动过程中更新位移与状态栏；不请求引擎渲染、不递增 revision（R3.2 / R8.1）。
  private updateMove(x: number, y: number): void {
    if (!this.movingLayer) {
      return;
    }
    this.moveCurrentScreenX = x;
    this.moveCurrentScreenY = y;
    this.moveDx = this.screenToDocX(x) - this.moveStartDocX;
    this.moveDy = this.screenToDocY(y) - this.moveStartDocY;
    this.status = '移动中 dx=' + Math.round(this.moveDx) + ' dy=' + Math.round(this.moveDy);
  }

  // 松手提交：屏幕累计位移 < 2px 或文档位移可忽略则丢弃（R3.5）。
  // 位移在提交前清零，提交失败也不残留"半提交"状态。
  private async commitMove(): Promise<void> {
    if (!this.movingLayer) {
      return;
    }
    this.movingLayer = false;
    const screenDistance = Math.abs(this.moveCurrentScreenX - this.moveStartScreenX) +
      Math.abs(this.moveCurrentScreenY - this.moveStartScreenY);
    const dx = this.moveDx;
    const dy = this.moveDy;
    this.moveDx = 0;
    this.moveDy = 0;
    if (screenDistance < 2 || (Math.abs(dx) < 0.01 && Math.abs(dy) < 0.01)) {
      this.status = '移动：位移太小，未提交';
      return;
    }
    await this.applyEdit({ dx: dx, dy: dy }, '已移动图层');
  }

  private cancelMove(): void {
    this.movingLayer = false;
    this.moveDx = 0;
    this.moveDy = 0;
  }

  // ===== 图层蒙版（C-MASK-UI@1）=====
  // 数据流：loadMask 从 Engine 读取 LayerMaskInfo → @State 草稿 → MaskCard 控件 → maskCommit 单次提交 → afterEdit。
  // 约束：所有成功路径只经 maskCommit 调用一次 afterEdit；滑块拖动期间只写 maskDensity / maskFeather 草稿。

  // 可挂蒙版的图层类型：像素 / 形状 / 文字（组 / 调整 / 填充 / 智能对象不出现蒙版分组，R1.5）。
  private maskSupported(): boolean {
    if (this.selectedLayerId === 0) {
      return false;
    }
    const kind = this.selectedLayerKind();
    return kind === 'Pixel' || kind === 'Shape' || kind === 'Type';
  }

  // 回落空状态：切层 / 撤销 / 重做 / 读取失败时调用（不写文档）。
  private maskReset(): void {
    this.maskInfo = { present: false, enabled: true, linked: true, density: 1, feather: 0 };
    this.maskDensity = 100;
    this.maskFeather = 0;
    this.maskOpen = true;
  }

  // 读取单层蒙版状态并回填草稿；失败静默回落（切层时的读取噪声不覆盖状态栏）。
  private loadMask(layerId: number): void {
    if (this.docId === 0 || layerId === 0) {
      this.maskReset();
      return;
    }
    try {
      const info = Engine.layerMaskGet(this.docId, layerId);
      this.maskInfo = info;
      this.maskDensity = Math.round(info.density * 100);
      this.maskFeather = Math.round(info.feather);
    } catch (err) {
      this.maskReset();
    }
  }

  // 唯一提交入口（C-MASK-UI@1 §C）：调用原生写操作 → afterEdit 单路径 → 回填 → 状态栏。
  // 失败时不调 afterEdit，只写错误前缀文案并按当前文档状态回填。
  private async maskCommit(op: () => LayerMaskInfo, okText: string): Promise<void> {
    if (this.docId === 0 || this.selectedLayerId === 0) {
      this.status = '蒙版：请先在图层列表选中图层';
      return;
    }
    const layerId = this.selectedLayerId;
    try {
      const startedAt = Date.now();
      const beforeRev = Engine.revision(this.docId);
      const next = op();
      const afterRev = Engine.revision(this.docId);
      this.maskInfo = next;
      this.maskDensity = Math.round(next.density * 100);
      this.maskFeather = Math.round(next.feather);
      this.perf('mask apply rev=' + beforeRev + '->' + afterRev +
        ' key=- hit=false calls=1 ms=' + (Date.now() - startedAt));
      await this.afterEdit();
      this.loadMask(layerId);
      this.status = okText;
    } catch (err) {
      this.status = '蒙版操作失败：' + this.messageOf(err as Object);
      this.loadMask(layerId);
    }
  }

  private async addMaskReveal(): Promise<void> {
    await this.maskCommit(() => Engine.layerMaskAdd(this.docId, this.selectedLayerId, 'reveal'),
      '已添加蒙版（显示全部）');
  }

  private async addMaskHide(): Promise<void> {
    await this.maskCommit(() => Engine.layerMaskAdd(this.docId, this.selectedLayerId, 'hide'),
      '已添加蒙版（隐藏全部）');
  }

  // 用当前选区生成蒙版；无选区时 UI 已禁用，这里再守一层（不发原生调用）。
  private async fillMaskReveal(): Promise<void> {
    if (!this.selectionActive) {
      this.status = '未选择区域：请先用选区工具框选';
      return;
    }
    await this.maskCommit(() => Engine.layerMaskFillFromSelection(this.docId, this.selectedLayerId, 'reveal'),
      '已用选区生成蒙版（选区内显示）');
  }

  private async fillMaskHide(): Promise<void> {
    if (!this.selectionActive) {
      this.status = '未选择区域：请先用选区工具框选';
      return;
    }
    await this.maskCommit(() => Engine.layerMaskFillFromSelection(this.docId, this.selectedLayerId, 'hide'),
      '已用选区生成蒙版（选区内隐藏）');
  }

  // 启用/停用只切 enabled；原生层对“无变化”不写文档（R3.4）。
  private async toggleMaskEnabled(): Promise<void> {
    const nextEnabled = !this.maskInfo.enabled;
    await this.maskCommit(
      () => Engine.layerMaskSet(this.docId, this.selectedLayerId, '{"enabled":' + (nextEnabled ? 'true' : 'false') + '}'),
      nextEnabled ? '已启用蒙版' : '已停用蒙版');
  }

  // 密度：草稿 0..100，提交前与回读值比较；相同则提示“密度未改变”且不发原生调用（R4.5）。
  private async commitMaskDensity(value: number): Promise<void> {
    const current = Math.round(this.maskInfo.density * 100);
    if (Math.abs(value - current) < 0.5) {
      this.status = '密度未改变';
      return;
    }
    await this.maskCommit(
      () => Engine.layerMaskSet(this.docId, this.selectedLayerId, '{"density":' + (value / 100) + '}'),
      '已更新蒙版密度');
  }

  private async commitMaskFeather(value: number): Promise<void> {
    const current = this.maskInfo.feather;
    if (Math.abs(value - current) < 0.5) {
      this.status = '羽化未改变';
      return;
    }
    await this.maskCommit(
      () => Engine.layerMaskSet(this.docId, this.selectedLayerId, '{"feather":' + value + '}'),
      '已更新蒙版羽化');
  }

  // 应用蒙版：把生效值（含密度/羽化）烘焙进像素并移除蒙版，仅像素层可用（R5）。
  private async applyMask(): Promise<void> {
    if (this.selectedLayerKind() !== 'Pixel') {
      this.status = '形状 / 文字图层需先栅格化（本轮不提供）';
      return;
    }
    await this.maskCommit(() => Engine.layerMaskApply(this.docId, this.selectedLayerId), '已应用蒙版');
  }

  private async removeMask(): Promise<void> {
    await this.maskCommit(() => Engine.layerMaskRemove(this.docId, this.selectedLayerId), '已删除蒙版');
  }

  // ===== 图层自由变换（C-XFORM-UI@1）=====
  // 数据流：滑块只写 5 个草稿 → 「应用」单次 Engine.layerTransform（原生层一次 checkpoint）
  // → afterEdit 单路径刷新 → 草稿归零 → 状态栏。失败只写状态栏且保留草稿，便于修正后重试。

  // 类型门控（§3.4）：Adjustment / Fill 不支持变换，分组整体显示为禁用。
  private trSupported(): boolean {
    if (this.selectedLayerId === 0) {
      return false;
    }
    const kind = this.selectedLayerKind();
    return kind !== 'Adjustment' && kind !== 'Fill';
  }

  // 草稿是否偏离默认值（100/100/0/0/0）：默认态「应用」禁用，点击也不调引擎（§3.4）。
  private trDirty(): boolean {
    return this.trScaleX !== 100 || this.trScaleY !== 100 || this.trAngle !== 0 ||
      this.trDx !== 0 || this.trDy !== 0;
  }

  // 只把 5 个滑块归零，绝不写文档（§3.3「重置草稿」）；切层/删除/撤销/重做/换文档/离开页面复用。
  private trResetDraft(): void {
    this.trScaleX = 100;
    this.trScaleY = 100;
    this.trAngle = 0;
    this.trDx = 0;
    this.trDy = 0;
  }

  // 内容包围盒摘要（§3.3）：选中层取自 this.layers 的扁平列表（layerRowById）。
  // LayerInfo 只有 id/name/kind 等，不含内容宽高，原生 bounds 也仅在提交后返回，
  // 因此无尺寸信息时按契约显示 —。
  private trBoundsLabel(): string {
    const row = this.layerRowById(this.selectedLayerId);
    if (row === undefined) {
      return '内容 —';
    }
    return '内容 —';
  }

  // 唯一提交入口（§3.4）：相对当前状态提交一次；成功后草稿归零，失败保留草稿。
  private async applyTransform(): Promise<void> {
    if (this.docId === 0 || this.selectedLayerId === 0) {
      this.status = '变换：请先在图层列表选中图层';
      return;
    }
    if (!this.trSupported()) {
      this.status = '该图层类型不支持变换';
      return;
    }
    if (!this.trDirty()) {
      return;
    }
    const layerId = this.selectedLayerId;
    try {
      Engine.layerTransform(this.docId, layerId, JSON.stringify({
        scaleX: this.trScaleX / 100,
        scaleY: this.trScaleY / 100,
        angle: this.trAngle,
        dx: this.trDx,
        dy: this.trDy
      }));
      await this.afterEdit();
      this.trResetDraft();
      this.status = '已应用变换';
    } catch (err) {
      this.status = '变换失败：' + this.messageOf(err as Object);
    }
  }

  // ===== 图层样式（FX）=====

  // 清空 FX 状态：打开新文档、删除图层时使用。
  private fxReset(): void {
    this.fxInfo = { enabled: true, items: [] };
    this.fxOpen = false;
    this.fxKind = 'dropShadow';
    this.fxSyncDraft();
  }

  // 从引擎读取指定图层的效果；失败时回落为空状态并写状态栏。
  private loadFx(layerId: number): void {
    if (this.docId === 0 || layerId === 0) {
      this.fxReset();
      return;
    }
    try {
      this.fxInfo = Engine.layerEffects(this.docId, layerId);
    } catch (err) {
      this.fxInfo = { enabled: true, items: [] };
      this.status = '读取图层样式失败：' + this.messageOf(err as Object);
    }
    this.fxSyncDraft();
  }

  // 返回同类实例中 index 最小的一个；没有则 undefined。
  private fxItem(kind: string): LayerEffectInfo | undefined {
    let found: LayerEffectInfo | undefined = undefined;
    for (const item of this.fxInfo.items) {
      if (item.kind !== kind) {
        continue;
      }
      if (found === undefined || item.index < found.index) {
        found = item;
      }
    }
    return found;
  }

  private fxCount(kind: string): number {
    let count = 0;
    for (const item of this.fxInfo.items) {
      if (item.kind === kind) {
        count = count + 1;
      }
    }
    return count;
  }

  // 引擎数值字段可能缺失（未支持类型只保证 index/kind/label/enabled），这里做防御式取值。
  private fxNumber(value: number, fallback: number): number {
    const raw: number | undefined = value;
    if (raw === undefined || Number.isNaN(raw)) {
      return fallback;
    }
    return raw;
  }

  private fxEnabledOf(kind: string): boolean {
    const item = this.fxItem(kind);
    return item !== undefined && item.enabled;
  }

  private fxKindLabel(kind: string): string {
    if (kind === 'dropShadow') {
      return '投影';
    }
    if (kind === 'stroke') {
      return '描边';
    }
    if (kind === 'outerGlow') {
      return '发光';
    }
    if (kind === 'innerShadow') {
      return '内阴影';
    }
    if (kind === 'innerGlow') {
      return '内发光';
    }
    if (kind === 'gradientOverlay') {
      return '渐变叠加';
    }
    if (kind === 'bevelEmboss') {
      return '斜面与浮雕';
    }
    return '样式';
  }

  // 分组标题：多实例时显示数量，例如「投影 ×2」。
  private fxGroupTitle(label: string, kind: string): string {
    const count = this.fxCount(kind);
    return count > 1 ? label + ' ×' + count : label;
  }

  // 七组各用一个强调色点 + 3px 左边条做区分（分组卡片底色仍按契约保持 #22303f / #2a3d55）。
  // 既有三组色值冻结不变，新增四组取 C-FX-UI@2 视觉值表。
  private fxGroupAccent(kind: string): string {
    if (kind === 'dropShadow') {
      return '#6ea8ff';
    }
    if (kind === 'stroke') {
      return '#5ad1c4';
    }
    if (kind === 'outerGlow') {
      return '#f7b955';
    }
    if (kind === 'innerShadow') {
      return FX_ACCENT_INNER_SHADOW;
    }
    if (kind === 'innerGlow') {
      return FX_ACCENT_INNER_GLOW;
    }
    if (kind === 'gradientOverlay') {
      return FX_ACCENT_GRADIENT;
    }
    return FX_ACCENT_BEVEL;
  }

  private fxDefaultOpacity(kind: string): number {
    return kind === 'stroke' ? 100 : 75;
  }

  private fxDefaultColor(kind: string): string {
    return kind === 'outerGlow' ? '#ffffff' : '#111827';
  }

  private fxDefaultBlur(kind: string): number {
    return kind === 'stroke' ? 3 : 5;
  }

  private fxHex2(value: number): string {
    const clamped = value < 0 ? 0 : (value > 255 ? 255 : value);
    const text = clamped.toString(16);
    return text.length === 1 ? '0' + text : text;
  }

  // 引擎颜色数组为 BGR（hexToRgba 的输出）；此处仅用于色板选中反演，提交时仍由 hexToRgba 产出 BGR。
  private fxHexFromColor(color: number[], fallback: string): string {
    const raw: number[] | undefined = color;
    if (raw === undefined || raw.length < 3) {
      return fallback;
    }
    const b = Math.round(this.fxNumber(raw[0], 0) * 255);
    const g = Math.round(this.fxNumber(raw[1], 0) * 255);
    const r = Math.round(this.fxNumber(raw[2], 0) * 255);
    return '#' + this.fxHex2(r) + this.fxHex2(g) + this.fxHex2(b);
  }

  private fxPositionOf(item: LayerEffectInfo): string {
    const value: string | undefined = item.position;
    if (value === undefined || value.length === 0) {
      return 'outside';
    }
    return value;
  }

  // 以 fxInfo 为准刷新草稿：无实例时回到该类型的默认值。
  // 新增四组的 18 个标量同样由这里回填（切换图层 loadFx → fxSyncDraft 即可全量恢复）。
  private fxSyncDraft(): void {
    const item = this.fxItem(this.fxKind);
    if (item === undefined) {
      this.fxOpacity = this.fxDefaultOpacity(this.fxKind);
      this.fxAngle = 120;
      this.fxDistance = 5;
      this.fxBlur = this.fxDefaultBlur(this.fxKind);
      this.fxSpread = 0;
      this.fxWidth = 3;
      this.fxPosition = 'outside';
      this.fxColor = this.fxDefaultColor(this.fxKind);
    } else {
      this.fxOpacity = Math.round(this.fxNumber(item.opacity, 0.75) * 100);
      this.fxAngle = Math.round(this.fxNumber(item.angle, 120));
      if (this.fxKind === 'gradientOverlay' && item.gradient !== undefined) {
        // 渐变的角度在响应里位于 gradient.angle（顶层 angle 不含），必须据此回填。
        this.fxAngle = Math.round(this.fxOptionalNumber(item.gradient.angle, 90));
      }
      this.fxDistance = Math.round(this.fxNumber(item.distance, 5));
      if (this.fxKind === 'stroke') {
        this.fxWidth = Math.round(this.fxNumber(item.size, 3));
        this.fxBlur = 5;
      } else {
        this.fxBlur = Math.round(this.fxNumber(item.size, 5));
        this.fxWidth = 3;
      }
      this.fxSpread = Math.round(this.fxNumber(item.spread, 0) * 100);
      this.fxPosition = this.fxPositionOf(item);
      this.fxColor = this.fxHexFromColor(item.color, this.fxDefaultColor(this.fxKind));
    }
    this.fxSyncGradientDraft(item);
    this.fxSyncGlowDraft(item);
    this.fxSyncBevelDraft(item);
  }

  // 渐变草稿回填：只编辑首尾色标颜色，位置（p）与中间色标一律原样保留（R3.4）。
  private fxSyncGradientDraft(item: LayerEffectInfo | undefined): void {
    if (item === undefined) {
      this.fxGradStartColor = '#000000';
      this.fxGradEndColor = '#ffffff';
      this.fxGradStyle = 'linear';
      this.fxGradScale = 100;
      this.fxGradReverse = false;
      this.fxGradAlign = true;
      this.fxGradDither = false;
      return;
    }
    const gradient: FxGradientInfo | undefined = item.gradient;
    const stops: FxGradientStop[] = gradient === undefined ? [] : gradient.stops;
    if (stops.length >= 2) {
      const last = stops.length - 1;
      this.fxGradStartColor = this.fxHexFromColor(stops[0].color, '#000000');
      this.fxGradEndColor = this.fxHexFromColor(stops[last].color, '#ffffff');
    } else {
      this.fxGradStartColor = '#000000';
      this.fxGradEndColor = '#ffffff';
    }
    this.fxGradStyle = gradient === undefined ? 'linear' : this.fxOptionalText(gradient.style, 'linear');
    this.fxGradScale = Math.round(this.fxOptionalNumber(gradient === undefined ? undefined : gradient.scale, 1) * 100);
    this.fxGradReverse = this.fxOptionalBool(gradient === undefined ? undefined : gradient.reverse, false);
    this.fxGradAlign = this.fxOptionalBool(gradient === undefined ? undefined : gradient.align, true);
    this.fxGradDither = this.fxOptionalBool(item.dither, false);
  }

  private fxSyncGlowDraft(item: LayerEffectInfo | undefined): void {
    this.fxGlowSource = item === undefined ? 'edge' : this.fxOptionalText(item.source, 'edge');
  }

  private fxSyncBevelDraft(item: LayerEffectInfo | undefined): void {
    this.fxBevelStyle = item === undefined ? 'innerBevel' : this.fxOptionalText(item.style, 'innerBevel');
    this.fxBevelTechnique = item === undefined ? 'smooth' : this.fxOptionalText(item.bevelTechnique, 'smooth');
    this.fxBevelDepth = Math.round(this.fxOptionalNumber(item === undefined ? undefined : item.depth, 1) * 100);
    this.fxBevelUp = item === undefined ? true : this.fxOptionalBool(item.up, true);
    this.fxBevelSoften = Math.round(this.fxOptionalNumber(item === undefined ? undefined : item.soften, 0));
    this.fxBevelAltitude = Math.round(this.fxOptionalNumber(item === undefined ? undefined : item.altitude, 30));
    const highlight: number[] | undefined = item === undefined ? undefined : item.highlightColor;
    this.fxBevelHighlightColor = highlight === undefined ? '#ffffff' : this.fxHexFromColor(highlight, '#ffffff');
    this.fxBevelHighlightOpacity = Math.round(this.fxOptionalNumber(item === undefined ? undefined : item.highlightOpacity, 0.75) * 100);
    const shadow: number[] | undefined = item === undefined ? undefined : item.shadowColor;
    this.fxBevelShadowColor = shadow === undefined ? '#000000' : this.fxHexFromColor(shadow, '#000000');
    this.fxBevelShadowOpacity = Math.round(this.fxOptionalNumber(item === undefined ? undefined : item.shadowOpacity, 0.75) * 100);
  }

  // 可选数值/布尔/文本字段的缺省取值（C-EFFECTS-API@2 的新字段在旧引擎响应中可能整体缺失）。
  private fxOptionalNumber(value: number | undefined, fallback: number): number {
    if (value === undefined) {
      return fallback;
    }
    return this.fxNumber(value, fallback);
  }

  private fxOptionalBool(value: boolean | undefined, fallback: boolean): boolean {
    return value === undefined ? fallback : value;
  }

  private fxOptionalText(value: string | undefined, fallback: string): string {
    if (value === undefined || value.length === 0) {
      return fallback;
    }
    return value;
  }

  private fxDraftValue(name: string): number {
    if (name === 'opacity') {
      return this.fxOpacity;
    }
    if (name === 'angle') {
      return this.fxAngle;
    }
    if (name === 'distance') {
      return this.fxDistance;
    }
    if (name === 'blur') {
      return this.fxBlur;
    }
    if (name === 'spread') {
      return this.fxSpread;
    }
    if (name === 'width') {
      return this.fxWidth;
    }
    if (name === 'gradScale') {
      return this.fxGradScale;
    }
    if (name === 'bevelDepth') {
      return this.fxBevelDepth;
    }
    if (name === 'bevelSoften') {
      return this.fxBevelSoften;
    }
    if (name === 'bevelAltitude') {
      return this.fxBevelAltitude;
    }
    if (name === 'highlightOpacity') {
      return this.fxBevelHighlightOpacity;
    }
    if (name === 'shadowOpacity') {
      return this.fxBevelShadowOpacity;
    }
    return 0;
  }

  private fxFormatValue(name: string, unit: string): string {
    const value = Math.round(this.fxDraftValue(name));
    if (unit === '%') {
      return value + '%';
    }
    if (unit === 'px') {
      return value + 'px';
    }
    if (unit === '°') {
      return value + '°';
    }
    return value.toString();
  }

  // 滑块拖动时只更新草稿；End/Click 才提交，避免拖动期间反复渲染。
  private fxOnSlider(name: string, value: number, mode: SliderChangeMode): void {
    const rounded = Math.round(value);
    if (name === 'opacity') {
      this.fxOpacity = rounded;
    } else if (name === 'angle') {
      this.fxAngle = rounded;
    } else if (name === 'distance') {
      this.fxDistance = rounded;
    } else if (name === 'blur') {
      this.fxBlur = rounded;
    } else if (name === 'spread') {
      this.fxSpread = rounded;
    } else if (name === 'width') {
      this.fxWidth = rounded;
    } else if (name === 'gradScale') {
      this.fxGradScale = rounded;
    } else if (name === 'bevelDepth') {
      this.fxBevelDepth = rounded;
    } else if (name === 'bevelSoften') {
      this.fxBevelSoften = rounded;
    } else if (name === 'bevelAltitude') {
      this.fxBevelAltitude = rounded;
    } else if (name === 'highlightOpacity') {
      this.fxBevelHighlightOpacity = rounded;
    } else if (name === 'shadowOpacity') {
      this.fxBevelShadowOpacity = rounded;
    }
    if (mode === SliderChangeMode.End || mode === SliderChangeMode.Click) {
      this.fxCommit(name, rounded);
    }
  }

  // 显式分支赋值：ArkTS 严格模式禁止按参数名做动态索引赋值。
  private fxAssign(params: FxUpdateParams, name: string, value: number): void {
    if (name === 'opacity') {
      params.opacity = value / 100;
    } else if (name === 'angle') {
      params.angle = value;
    } else if (name === 'distance') {
      params.distance = value;
    } else if (name === 'blur') {
      params.size = value;
    } else if (name === 'spread') {
      params.spread = value / 100;
    } else if (name === 'width') {
      params.size = value;
    } else if (name === 'gradScale') {
      // 显示值 10–150 %，引擎侧 1.0 = 100 %。
      params.gradientScale = value / 100;
    } else if (name === 'bevelDepth') {
      // 显示值 0–1000 %，引擎深度原生值域 0..=10（1.0 = 100 %）。
      params.depth = value / 100;
    } else if (name === 'bevelSoften') {
      params.soften = value;
    } else if (name === 'bevelAltitude') {
      params.altitude = value;
    } else if (name === 'highlightOpacity') {
      params.highlightOpacity = value / 100;
    } else if (name === 'shadowOpacity') {
      params.shadowOpacity = value / 100;
    }
  }

  private async fxCommit(name: string, value: number): Promise<void> {
    if (this.docId === 0 || this.selectedLayerId === 0) {
      return;
    }
    const item = this.fxItem(this.fxKind);
    let params: FxUpdateParams;
    if (item === undefined) {
      params = { op: 'add', kind: this.fxKind };
    } else {
      params = { op: 'update', index: item.index };
    }
    this.fxAssign(params, name, value);
    // 渐变叠加的任何参数提交都必须附带完整 stops：首尾用当前草稿，中间色标原样保留（R3.4）。
    if (this.fxKind === 'gradientOverlay') {
      params.gradientStops = this.fxStopsDraft();
    }
    await this.fxApply(params);
  }

  // 唯一的引擎写入口：成功用返回值刷新 fxInfo 与草稿，再通过 afterEdit 统一刷新元数据与画面。
  private async fxApply(params: FxUpdateParams): Promise<boolean> {
    if (this.docId === 0 || this.selectedLayerId === 0) {
      return false;
    }
    try {
      this.fxInfo = Engine.updateLayerEffect(this.docId, this.selectedLayerId, JSON.stringify(params));
      this.fxSyncDraft();
      await this.afterEdit();
      return true;
    } catch (err) {
      this.status = '图层样式失败：' + this.messageOf(err as Object);
      return false;
    }
  }

  private async fxAdd(kind: string): Promise<void> {
    const params: FxUpdateParams = { op: 'add', kind: kind };
    if (kind === 'gradientOverlay') {
      // 渐变类提交一律附带完整 stops；新建时取工厂默认（黑→白，位置 0/1）。
      const first: FxGradientStop = { p: 0, color: this.hexToRgba('#000000') };
      const last: FxGradientStop = { p: 1, color: this.hexToRgba('#ffffff') };
      const stops: FxGradientStop[] = [first, last];
      params.gradientStops = stops;
    }
    const ok = await this.fxApply(params);
    if (ok) {
      this.fxKind = kind;
      this.fxOpen = true;
      this.fxSyncDraft();
      this.status = '已添加' + this.fxKindLabel(kind);
    }
  }

  private async fxToggle(kind: string): Promise<void> {
    const item = this.fxItem(kind);
    if (item === undefined) {
      await this.fxAdd(kind);
      return;
    }
    const wasEnabled = item.enabled;
    const params: FxUpdateParams = { op: 'enable', index: item.index, enabled: !wasEnabled };
    const ok = await this.fxApply(params);
    if (ok) {
      this.status = this.fxKindLabel(kind) + (wasEnabled ? '已停用' : '已启用');
    }
  }

  private async fxRemove(kind: string): Promise<void> {
    const item = this.fxItem(kind);
    if (item === undefined) {
      return;
    }
    const params: FxUpdateParams = { op: 'remove', index: item.index };
    const ok = await this.fxApply(params);
    if (ok) {
      this.status = '已删除' + this.fxKindLabel(kind);
    }
  }

  private async fxMaster(enabled: boolean): Promise<void> {
    const params: FxUpdateParams = { op: 'master', enabled: enabled };
    const ok = await this.fxApply(params);
    if (ok) {
      this.status = '图层样式总开关：' + (enabled ? '开' : '关');
    }
  }

  private async fxClear(): Promise<void> {
    const params: FxUpdateParams = { op: 'clear' };
    const ok = await this.fxApply(params);
    if (ok) {
      this.status = '已清除全部图层样式';
    }
  }

  // 颜色直接经 hexToRgba 原样传入 color 字段，任何一层都不交换通道。
  private async fxSetColor(kind: string, hex: string): Promise<void> {
    this.fxColor = hex;
    const item = this.fxItem(kind);
    let params: FxUpdateParams;
    if (item === undefined) {
      params = { op: 'add', kind: kind, color: this.hexToRgba(hex) };
    } else {
      params = { op: 'update', index: item.index, color: this.hexToRgba(hex) };
    }
    await this.fxApply(params);
  }

  // 五个颜色槽的写入（slot: color | gradStart | gradEnd | highlight | shadow）。
  // 颜色一律由 hexToRgba 原样提交（BGR 直通），任何一层都不交换通道。
  private async fxSetColorSlot(slot: string, hex: string): Promise<void> {
    if (slot === 'color') {
      this.fxColor = hex;
    } else if (slot === 'gradStart') {
      this.fxGradStartColor = hex;
    } else if (slot === 'gradEnd') {
      this.fxGradEndColor = hex;
    } else if (slot === 'highlight') {
      this.fxBevelHighlightColor = hex;
    } else if (slot === 'shadow') {
      this.fxBevelShadowColor = hex;
    }
    const item = this.fxItem(this.fxKind);
    let params: FxUpdateParams;
    if (item === undefined) {
      params = { op: 'add', kind: this.fxKind };
    } else {
      params = { op: 'update', index: item.index };
    }
    if (slot === 'color') {
      params.color = this.hexToRgba(hex);
    } else if (slot === 'gradStart' || slot === 'gradEnd') {
      params.gradientStops = this.fxStopsDraft();
    } else if (slot === 'highlight') {
      params.highlightColor = this.hexToRgba(hex);
    } else if (slot === 'shadow') {
      params.shadowColor = this.hexToRgba(hex);
    }
    await this.fxApply(params);
  }

  // 指定颜色槽的当前值：@Builder 内通过本方法读 this.*，不通过 Builder 形参传状态。
  private fxSlotColor(slot: string): string {
    if (slot === 'color') {
      return this.fxColor;
    }
    if (slot === 'gradStart') {
      return this.fxGradStartColor;
    }
    if (slot === 'gradEnd') {
      return this.fxGradEndColor;
    }
    if (slot === 'highlight') {
      return this.fxBevelHighlightColor;
    }
    if (slot === 'shadow') {
      return this.fxBevelShadowColor;
    }
    return '#000000';
  }

  // 完整色标草稿：首项 = {原首项 p, 草稿起始色}，末项 = {原末项 p, 草稿结束色}，中间项原样保留。
  // 原 stops 不足 2 项时回落黑→白。只在提交时调用，Builder 重绘期间不调用。
  private fxStopsDraft(): FxGradientStop[] {
    const item = this.fxItem('gradientOverlay');
    const stops: FxGradientStop[] = [];
    const startColor: number[] = this.hexToRgba(this.fxGradStartColor);
    const endColor: number[] = this.hexToRgba(this.fxGradEndColor);
    const origin: FxGradientStop[] = (item === undefined || item.gradient === undefined) ? [] : item.gradient.stops;
    if (origin.length < 2) {
      const first: FxGradientStop = { p: 0, color: startColor };
      const last: FxGradientStop = { p: 1, color: endColor };
      stops.push(first);
      stops.push(last);
      return stops;
    }
    for (let i = 0; i < origin.length; i++) {
      const source = origin[i];
      let color: number[] = source.color;
      if (i === 0) {
        color = startColor;
      } else if (i === origin.length - 1) {
        color = endColor;
      }
      const stop: FxGradientStop = { p: source.p, color: color };
      stops.push(stop);
    }
    return stops;
  }

  // 字符串类 chip（渐变类型/斜面样式/斜面技术/内发光源/内发光技术）：点击即提交 1 次。
  private async fxSetChip(name: string, value: string): Promise<void> {
    if (name === 'gradientStyle') {
      this.fxGradStyle = value;
    } else if (name === 'bevelStyle') {
      this.fxBevelStyle = value;
    } else if (name === 'bevelTechnique') {
      this.fxBevelTechnique = value;
    } else if (name === 'glowSource') {
      this.fxGlowSource = value;
    }
    const item = this.fxItem(this.fxKind);
    let params: FxUpdateParams;
    if (item === undefined) {
      params = { op: 'add', kind: this.fxKind };
    } else {
      params = { op: 'update', index: item.index };
    }
    if (name === 'gradientStyle') {
      params.gradientStyle = value;
    } else if (name === 'bevelStyle') {
      params.bevelStyle = value;
    } else if (name === 'bevelTechnique') {
      params.bevelTechnique = value;
    } else if (name === 'glowSource') {
      params.source = value;
    } else if (name === 'technique') {
      params.technique = value;
    }
    if (this.fxKind === 'gradientOverlay') {
      params.gradientStops = this.fxStopsDraft();
    }
    await this.fxApply(params);
  }

  // 布尔类 chip（反向/对齐/抖动/方向上:下/抗锯齿）：点击即提交 1 次。
  private async fxSetToggle(name: string, value: boolean): Promise<void> {
    if (name === 'reverse') {
      this.fxGradReverse = value;
    } else if (name === 'align') {
      this.fxGradAlign = value;
    } else if (name === 'dither') {
      this.fxGradDither = value;
    } else if (name === 'bevelUp') {
      this.fxBevelUp = value;
    }
    const item = this.fxItem(this.fxKind);
    let params: FxUpdateParams;
    if (item === undefined) {
      params = { op: 'add', kind: this.fxKind };
    } else {
      params = { op: 'update', index: item.index };
    }
    if (name === 'reverse') {
      params.reverse = value;
    } else if (name === 'align') {
      params.align = value;
    } else if (name === 'dither') {
      params.dither = value;
    } else if (name === 'bevelUp') {
      params.up = value;
    } else if (name === 'antiAlias') {
      params.antiAlias = value;
    }
    if (this.fxKind === 'gradientOverlay') {
      params.gradientStops = this.fxStopsDraft();
    }
    await this.fxApply(params);
  }

  // chip 点击总入口：方向 chip 用字符串 up/down 表达布尔，其余转交 fxSetChip。
  private async fxSelectChip(name: string, value: string): Promise<void> {
    if (name === 'bevelUp') {
      await this.fxSetToggle('bevelUp', value === 'up');
      return;
    }
    await this.fxSetChip(name, value);
  }

  // chip 选中态：全部直接读 this.*（在 @Builder 内经由方法读取，避免形参传状态不刷新）。
  private fxChipSelected(name: string, value: string): boolean {
    if (name === 'gradientStyle') {
      return this.fxGradStyle === value;
    }
    if (name === 'bevelStyle') {
      return this.fxBevelStyle === value;
    }
    if (name === 'bevelTechnique') {
      return this.fxBevelTechnique === value;
    }
    if (name === 'glowSource') {
      return this.fxGlowSource === value;
    }
    if (name === 'bevelUp') {
      return this.fxBevelUp === (value === 'up');
    }
    if (name === 'technique') {
      return this.fxTechniqueOf(this.fxKind) === value;
    }
    return false;
  }

  private fxToggleValue(name: string): boolean {
    if (name === 'reverse') {
      return this.fxGradReverse;
    }
    if (name === 'align') {
      return this.fxGradAlign;
    }
    if (name === 'dither') {
      return this.fxGradDither;
    }
    if (name === 'bevelUp') {
      return this.fxBevelUp;
    }
    if (name === 'antiAlias') {
      return this.fxAntiAliasOf(this.fxKind);
    }
    return false;
  }

  // 内发光的技术/抗锯齿没有独立草稿：直接由 fxInfo 派生，避免引入第 19 个冗余状态。
  private fxTechniqueOf(kind: string): string {
    const item = this.fxItem(kind);
    if (item === undefined) {
      return 'softer';
    }
    return this.fxOptionalText(item.technique, 'softer');
  }

  private fxAntiAliasOf(kind: string): boolean {
    const item = this.fxItem(kind);
    if (item === undefined) {
      return false;
    }
    return this.fxOptionalBool(item.antiAlias, false);
  }

  private fxGradientStyleLabel(style: string): string {
    if (style === 'radial') {
      return '径向';
    }
    if (style === 'angle') {
      return '角度';
    }
    if (style === 'reflected') {
      return '对称';
    }
    if (style === 'diamond') {
      return '菱形';
    }
    return '线性';
  }

  private fxBevelStyleLabel(style: string): string {
    if (style === 'outerBevel') {
      return '外斜面';
    }
    if (style === 'emboss') {
      return '浮雕';
    }
    if (style === 'pillowEmboss') {
      return '枕状浮雕';
    }
    if (style === 'strokeEmboss') {
      return '描边浮雕';
    }
    return '内斜面';
  }

  private fxBevelTechniqueLabel(technique: string): string {
    if (technique === 'chiselHard') {
      return '硬性雕刻';
    }
    if (technique === 'chiselSoft') {
      return '软性雕刻';
    }
    return '平滑';
  }

  private fxGlowSourceLabel(source: string): string {
    return source === 'center' ? '中心' : '边缘';
  }

  private async fxSetPosition(position: string): Promise<void> {
    this.fxPosition = position;
    const item = this.fxItem(this.fxKind);
    let params: FxUpdateParams;
    if (item === undefined) {
      params = { op: 'add', kind: this.fxKind, position: position };
    } else {
      params = { op: 'update', index: item.index, position: position };
    }
    await this.fxApply(params);
  }

  private fxPositionLabel(position: string): string {
    if (position === 'inside') {
      return '内';
    }
    if (position === 'center') {
      return '中';
    }
    return '外';
  }

  // 非当前编辑分组的参数摘要：保持"有实例时显示其参数"的可读性，同时不引入第二份草稿。
  private fxSummaryLine(kind: string): string {
    const item = this.fxItem(kind);
    if (item === undefined) {
      return '';
    }
    const opacity = Math.round(this.fxNumber(item.opacity, 0.75) * 100);
    if (kind === 'dropShadow') {
      const angle = Math.round(this.fxNumber(item.angle, 120));
      const distance = Math.round(this.fxNumber(item.distance, 5));
      const blur = Math.round(this.fxNumber(item.size, 5));
      return '不透明度 ' + opacity + '% · 角度 ' + angle + '° · 距离 ' + distance + 'px · 模糊 ' + blur + 'px';
    }
    if (kind === 'stroke') {
      const width = Math.round(this.fxNumber(item.size, 3));
      return '宽度 ' + width + 'px · 位置 ' + this.fxPositionLabel(this.fxPositionOf(item)) + ' · 不透明度 ' + opacity + '%';
    }
    if (kind === 'innerShadow') {
      const shadowAngle = Math.round(this.fxNumber(item.angle, 120));
      const shadowDistance = Math.round(this.fxNumber(item.distance, 5));
      const shadowBlur = Math.round(this.fxNumber(item.size, 5));
      const shadowSpread = Math.round(this.fxNumber(item.spread, 0) * 100);
      return '不透明度 ' + opacity + '% · 角度 ' + shadowAngle + '° · 距离 ' + shadowDistance + 'px · 模糊 ' + shadowBlur + 'px · 扩展 ' + shadowSpread + '%';
    }
    if (kind === 'innerGlow') {
      const innerBlur = Math.round(this.fxNumber(item.size, 5));
      const innerSpread = Math.round(this.fxNumber(item.spread, 0) * 100);
      const source = this.fxOptionalText(item.source, 'edge');
      return '不透明度 ' + opacity + '% · 模糊 ' + innerBlur + 'px · 扩展 ' + innerSpread + '% · 源 ' + this.fxGlowSourceLabel(source);
    }
    if (kind === 'gradientOverlay') {
      const gradient: FxGradientInfo | undefined = item.gradient;
      const style = gradient === undefined ? 'linear' : this.fxOptionalText(gradient.style, 'linear');
      const angle = Math.round(this.fxOptionalNumber(gradient === undefined ? undefined : gradient.angle, 90));
      const scale = Math.round(this.fxOptionalNumber(gradient === undefined ? undefined : gradient.scale, 1) * 100);
      return '不透明度 ' + opacity + '% · ' + this.fxGradientStyleLabel(style) + ' · 角度 ' + angle + '° · 缩放 ' + scale + '%';
    }
    if (kind === 'bevelEmboss') {
      const style = this.fxOptionalText(item.style, 'innerBevel');
      const technique = this.fxOptionalText(item.bevelTechnique, 'smooth');
      const depth = Math.round(this.fxOptionalNumber(item.depth, 1) * 100);
      const size = Math.round(this.fxNumber(item.size, 5));
      const up = this.fxOptionalBool(item.up, true);
      return this.fxBevelStyleLabel(style) + ' · ' + this.fxBevelTechniqueLabel(technique) + ' · 深度 ' + depth + '% · 方向 ' + (up ? '上' : '下') + ' · 大小 ' + size + 'px';
    }
    const glowBlur = Math.round(this.fxNumber(item.size, 5));
    const spread = Math.round(this.fxNumber(item.spread, 0) * 100);
    return '不透明度 ' + opacity + '% · 模糊 ' + glowBlur + 'px · 扩展 ' + spread + '%';
  }

  private fxSelectKind(kind: string): void {
    if (this.fxKind === kind) {
      return;
    }
    this.fxKind = kind;
    this.fxSyncDraft();
  }

  // 未支持类型：只展示标签与说明，不提供参数编辑。
  private fxUnsupported(): LayerEffectInfo[] {
    const result: LayerEffectInfo[] = [];
    for (const item of this.fxInfo.items) {
      if (item.kind === 'unsupported') {
        result.push(item);
      }
    }
    return result;
  }

  private fxItemsSummary(): string {
    const parts: string[] = [];
    const shadow = this.fxCount('dropShadow');
    const stroke = this.fxCount('stroke');
    const glow = this.fxCount('outerGlow');
    const innerShadow = this.fxCount('innerShadow');
    const innerGlow = this.fxCount('innerGlow');
    const gradient = this.fxCount('gradientOverlay');
    const bevel = this.fxCount('bevelEmboss');
    if (shadow > 0) {
      parts.push(shadow > 1 ? '投影 ×' + shadow : '投影');
    }
    if (stroke > 0) {
      parts.push(stroke > 1 ? '描边 ×' + stroke : '描边');
    }
    if (glow > 0) {
      parts.push(glow > 1 ? '发光 ×' + glow : '发光');
    }
    if (innerShadow > 0) {
      parts.push(innerShadow > 1 ? '内阴影 ×' + innerShadow : '内阴影');
    }
    if (innerGlow > 0) {
      parts.push(innerGlow > 1 ? '内发光 ×' + innerGlow : '内发光');
    }
    if (gradient > 0) {
      parts.push(gradient > 1 ? '渐变叠加 ×' + gradient : '渐变叠加');
    }
    if (bevel > 0) {
      parts.push(bevel > 1 ? '斜面与浮雕 ×' + bevel : '斜面与浮雕');
    }
    return parts.length > 0 ? parts.join(' · ') : '无样式';
  }

  private defaultParams(kind: string): string {
    switch (kind) {
      case 'brightnessContrast':
        return '{"brightness":0,"contrast":0}';
      case 'exposure':
        return '{"exposure":0,"offset":0,"gamma":1}';
      case 'vibrance':
        return '{"vibrance":0,"saturation":0}';
      case 'hueSaturation':
        return '{"hue":0,"saturation":0,"lightness":0,"colorize":false}';
      case 'posterize':
        return '{"levels":4}';
      case 'threshold':
        return '{"level":0.5}';
      default:
        return '{}';
    }
  }

  private async addAdjustment(kind: string): Promise<void> {
    if (this.docId === 0) {
      promptAction.showToast({ message: '请先打开一张图片' });
      return;
    }
    try {
      Engine.addAdjustment(this.docId, kind, this.defaultParams(kind));
      await this.afterEdit();
      let newest = 0;
      for (const layer of this.layers) {
        if (layer.kind === 'Adjustment' && layer.id > newest) {
          newest = layer.id;
        }
      }
      for (const layer of this.layers) {
        if (layer.id === newest) {
          this.selectLayer(layer);
          break;
        }
      }
      this.status = '已添加调整图层';
    } catch (err) {
      this.status = this.messageOf(err as Object);
    }
  }

  private adjustWith(patch: AdjustPatch): AdjustmentInfo {
    const a = this.adjust;
    return {
      type: a.type,
      brightness: patch.brightness !== undefined ? patch.brightness : a.brightness,
      contrast: patch.contrast !== undefined ? patch.contrast : a.contrast,
      exposure: patch.exposure !== undefined ? patch.exposure : a.exposure,
      offset: patch.offset !== undefined ? patch.offset : a.offset,
      gamma: patch.gamma !== undefined ? patch.gamma : a.gamma,
      vibrance: patch.vibrance !== undefined ? patch.vibrance : a.vibrance,
      saturation: patch.saturation !== undefined ? patch.saturation : a.saturation,
      hue: patch.hue !== undefined ? patch.hue : a.hue,
      lightness: patch.lightness !== undefined ? patch.lightness : a.lightness,
      levels: patch.levels !== undefined ? patch.levels : a.levels,
      level: patch.level !== undefined ? patch.level : a.level,
      colorize: a.colorize
    };
  }

  private commitBrightnessContrast(): void {
    this.commitAdjustment(JSON.stringify({ brightness: this.adjust.brightness, contrast: this.adjust.contrast }));
  }

  private commitExposure(): void {
    this.commitAdjustment(JSON.stringify({ exposure: this.adjust.exposure, offset: this.adjust.offset, gamma: this.adjust.gamma }));
  }

  private commitVibrance(): void {
    this.commitAdjustment(JSON.stringify({ vibrance: this.adjust.vibrance, saturation: this.adjust.saturation }));
  }

  private commitHueSaturation(): void {
    this.commitAdjustment(JSON.stringify({ hue: this.adjust.hue, saturation: this.adjust.saturation, lightness: this.adjust.lightness, colorize: this.adjust.colorize }));
  }

  private commitPosterize(): void {
    this.commitAdjustment(JSON.stringify({ levels: this.adjust.levels }));
  }

  private commitThreshold(): void {
    this.commitAdjustment(JSON.stringify({ level: this.adjust.level }));
  }

  private commitAdjustment(params: string): void {
    if (this.docId === 0 || this.selectedLayerId === 0) {
      return;
    }
    try {
      Engine.updateAdjustment(this.docId, this.selectedLayerId, params);
      this.afterEdit();
    } catch (err) {
      this.status = this.messageOf(err as Object);
    }
  }

  private async undo(): Promise<void> {
    if (this.docId === 0) {
      return;
    }
    try {
      const info = Engine.undo(this.docId);
      // 撤销换了文档内容：版本号变化即缓存失效（R8.2）。
      this.syncDocRevision();
      this.resetRenderCache();
      this.info = info;
      // 文档被替换：裁剪框坐标对应旧画布尺寸，必须丢弃且不恢复视图快照（R1.5）。
      if (this.cropMode) {
        this.cropDiscard();
      }
      this.layers = flattenLayers(info.layers, 0);
      this.selectedLayerId = 0;
      this.adjust = emptyAdjustment();
      // 撤销后选中层已归零：清草稿与进行中的移动状态（R4.4）。
      this.editReset();
      this.maskReset();
      // 撤销后文档已变：变换草稿同样归零（C-XFORM-UI@1 §3.5）。
      this.trResetDraft();
      await this.refreshPreview();
      this.syncSelection();
      // 撤销可能改变画布尺寸（裁剪/画布尺寸调整）：回填宽高草稿，
      // 否则输入框仍显示撤销前的尺寸，用户直接点「应用」会把画布又改回去。
      this.syncCanvasDraft();
      // 视图辅助（V2）：参考线同样可撤销，撤销后重新回填。
      this.syncGuides();
      this.loadHistory();
      this.status = '已撤销';
    } catch (err) {
      this.status = this.messageOf(err as Object);
    }
  }

  private async redo(): Promise<void> {
    if (this.docId === 0) {
      return;
    }
    try {
      const info = Engine.redo(this.docId);
      // 重做换了文档内容：版本号变化即缓存失效（R8.2）。
      this.syncDocRevision();
      this.resetRenderCache();
      this.info = info;
      // 文档被替换：丢弃裁剪框且不恢复视图快照（R1.5）。
      if (this.cropMode) {
        this.cropDiscard();
      }
      this.layers = flattenLayers(info.layers, 0);
      this.selectedLayerId = 0;
      this.adjust = emptyAdjustment();
      // 重做后选中层已归零：清草稿与进行中的移动状态（R4.4）。
      this.editReset();
      this.maskReset();
      // 重做后文档已变：变换草稿同样归零（C-XFORM-UI@1 §3.5）。
      this.trResetDraft();
      await this.refreshPreview();
      this.syncSelection();
      // 重做同样可能改变画布尺寸，回填宽高草稿（理由见 undo）。
      this.syncCanvasDraft();
      // 视图辅助（V2）：重做后重新回填参考线。
      this.syncGuides();
      this.loadHistory();
      this.status = '已重做';
    } catch (err) {
      this.status = this.messageOf(err as Object);
    }
  }

  private async exportImage(): Promise<void> {
    if (this.docId === 0) {
      promptAction.showToast({ message: '请先打开一张图片' });
      return;
    }
    try {
      this.busy = true;
      const bytes = Engine.exportImage(this.docId, this.exportFormat, this.exportQuality);
      const options = new picker.DocumentSaveOptions();
      options.newFileNames = ['photocraft-' + Date.now() + '.' + this.exportFormat];
      const uris = await new picker.DocumentViewPicker(getContext(this)).save(options);
      if (uris.length === 0) {
        return;
      }
      const file = fileIo.openSync(uris[0], fileIo.OpenMode.READ_WRITE | fileIo.OpenMode.CREATE | fileIo.OpenMode.TRUNC);
      fileIo.writeSync(file.fd, bytes);
      fileIo.closeSync(file);
      this.status = '已导出 ' + this.exportFormat.toUpperCase();
      promptAction.showToast({ message: '导出成功' });
    } catch (err) {
      this.status = '导出失败：' + this.messageOf(err as Object);
      promptAction.showToast({ message: '导出失败：' + this.messageOf(err as Object) });
    } finally {
      this.busy = false;
    }
  }

  private formatLabel(format: string): string {
    return format.toUpperCase();
  }

  private barEnabled(kind: string): boolean {
    if (kind === 'open') {
      return true;
    }
    if (kind === 'undo') {
      return this.info?.canUndo === true;
    }
    if (kind === 'redo') {
      return this.info?.canRedo === true;
    }
    return this.docId !== 0;
  }

  @Builder
  BarButton(label: string, kind: string, action: () => void) {
    Button(label)
      .fontSize(13)
      .fontColor(this.barEnabled(kind) ? COLOR_TEXT : '#4d5663')
      .backgroundColor('#22262e')
      .height(32)
      .padding({ left: 12, right: 12 })
      .borderRadius(8)
      .enabled(this.barEnabled(kind))
      .onClick(() => {
        action();
      })
  }

  @Builder
  TopBar() {
    Row() {
      Text('PhotoCraft')
        .fontSize(17)
        .fontWeight(FontWeight.Bold)
        .fontColor(COLOR_TEXT)
      Text(this.info?.name ?? '图像编辑器')
        .fontSize(11)
        .fontColor(COLOR_MUTED)
        .margin({ left: 10 })
        .maxLines(1)
        .textOverflow({ overflow: TextOverflow.Ellipsis })
        .layoutWeight(1)
      Row({ space: 8 }) {
        this.BarButton('打开', 'open', () => {
          this.openFile();
        })
        this.BarButton('撤销', 'undo', () => {
          this.undo();
        })
        this.BarButton('重做', 'redo', () => {
          this.redo();
        })
        this.BarButton('导出', 'export', () => {
          this.exportImage();
        })
      }
    }
    .width('100%')
    .height(54)
    .padding({ left: 16, right: 16 })
    .backgroundColor(COLOR_BAR)
    .border({ width: { bottom: 1 }, color: '#232830' })
  }

  @Builder
  CanvasPane() {
    Stack({ alignContent: Alignment.TopStart }) {
      Column()
        .width('100%')
        .height('100%')
        .backgroundColor(COLOR_CANVAS)
      if (this.pixelMap !== undefined && this.displayWidth() > 0 && this.displayHeight() > 0) {
        Image(this.pixelMap as image.PixelMap)
          .objectFit(ImageFit.Fill)
          .interpolation(ImageInterpolation.Medium)
          .width(this.displayWidth())
          .height(this.displayHeight())
          .position({ x: this.displayLeft(), y: this.displayTop() })
      } else if (this.info === undefined) {
        Column({ space: 10 }) {
          Text('还没有打开图片')
            .fontSize(16)
            .fontColor(COLOR_MUTED)
          Text('支持 PNG · JPEG · WebP · TIFF · PSD / PSB')
            .fontSize(12)
            .fontColor('#5d6673')
          Button('打开图片')
            .fontSize(14)
            .height(38)
            .backgroundColor('#2b6cf6')
            .borderRadius(10)
            .onClick(() => {
              this.openFile();
            })
          Button('打开示例图')
            .fontSize(13)
            .height(34)
            .backgroundColor('#243040')
            .fontColor('#9fc4ff')
            .borderRadius(10)
            .onClick(() => {
              this.openSample();
            })
          // 新建空白文档（file.new）：宽高只写草稿，点击按钮才提交引擎。
          Row({ space: 8 }) {
            TextInput({ placeholder: '宽度', text: this.newDocW })
              .type(InputType.Number)
              .fontSize(13)
              .fontColor('#e6f0fa')
              .placeholderColor('#6f8aa6')
              .backgroundColor('#1b2430')
              .borderRadius(8)
              .height(34)
              .width(88)
              .enabled(!this.busy)
              .onChange((value: string) => {
                this.newDocW = value;
              })
            Text('×')
              .fontSize(13)
              .fontColor('#5d6673')
            TextInput({ placeholder: '高度', text: this.newDocH })
              .type(InputType.Number)
              .fontSize(13)
              .fontColor('#e6f0fa')
              .placeholderColor('#6f8aa6')
              .backgroundColor('#1b2430')
              .borderRadius(8)
              .height(34)
              .width(88)
              .enabled(!this.busy)
              .onChange((value: string) => {
                this.newDocH = value;
              })
            Button('新建文档')
              .fontSize(13)
              .height(34)
              .backgroundColor('#1f6f4a')
              .fontColor('#d7ffe9')
              .borderRadius(10)
              .enabled(!this.busy)
              .onClick(() => {
                this.newDocument();
              })
          }
        }
        .width('100%')
        .height('100%')
        .justifyContent(FlexAlign.Center)
      }
      Canvas(this.brushCtx)
        .width('100%')
        .height('100%')
        .hitTestBehavior(HitTestMode.None)
      // 裁剪框尺寸标签（C-CROPMODE-UI@1 §5）：ArkUI Text 而非 ctx.fillText，
      // 保证 uitest dumpLayout 可读到 "w × h"（T4 取证依赖）。
      if (this.cropMode) {
        Text(this.cropSizeLabel())
          .id('cropSizeTag')
          .fontSize(11)
          .fontColor('#ffffff')
          .padding({ left: 8, right: 8, top: 3, bottom: 3 })
          .backgroundColor('rgba(0,0,0,0.65)')
          .borderRadius(5)
          .hitTestBehavior(HitTestMode.None)
          .position({ x: this.cropLabelX(), y: this.cropLabelY() })
      }
      // 变换实时标签（C-TRANSFORM-GESTURE@1 §5）：缩放 / 旋转 / 偏移，位置跟随框。
      // 内容用内联表达式直接引用 5 个 @State，保证拖拽时 ArkUI 依赖收集成立、逐帧刷新。
      if (this.trMode) {
        Text('缩放 ' + Math.round(this.trPreviewScaleX) + '%' +
          ' / 旋转 ' + Math.round(this.trPreviewAngle) + '°' +
          ' / 偏移 ' + Math.round(this.trPreviewDx) + ',' + Math.round(this.trPreviewDy))
          .id('trSizeTag')
          .fontSize(11)
          .fontColor('#ffffff')
          .padding({ left: 8, right: 8, top: 3, bottom: 3 })
          .backgroundColor('rgba(0,0,0,0.65)')
          .borderRadius(5)
          .hitTestBehavior(HitTestMode.None)
          .position({ x: this.trLabelX(), y: this.trLabelY() })
      }
      Row({ space: 8 }) {
        Text('−')
          .fontSize(18)
          .fontColor(COLOR_TEXT)
          .width(32)
          .height(32)
          .textAlign(TextAlign.Center)
          .backgroundColor('#1c2026')
          .borderRadius(8)
          .onClick(() => {
            this.setZoom(this.zoom / 1.25, this.viewportW / 2, this.viewportH / 2);
          })
        Text(this.displayScale() > 0 ? Math.round(this.displayScale() * 100) + '%' : '100%')
          .fontSize(12)
          .fontColor(COLOR_TEXT)
          .width(54)
          .textAlign(TextAlign.Center)
        Text('+')
          .fontSize(18)
          .fontColor(COLOR_TEXT)
          .width(32)
          .height(32)
          .textAlign(TextAlign.Center)
          .backgroundColor('#1c2026')
          .borderRadius(8)
          .onClick(() => {
            this.setZoom(this.zoom * 1.25, this.viewportW / 2, this.viewportH / 2);
          })
        Text('适应')
          .fontSize(12)
          .fontColor(COLOR_TEXT)
          .height(32)
          .padding({ left: 10, right: 10 })
          .backgroundColor('#1c2026')
          .borderRadius(8)
          .onClick(() => {
            this.resetView();
            if (this.usingDetail) {
              this.refreshPreview();
            } else {
              this.scheduleDetail();
            }
          })
        Text('1:1')
          .fontSize(12)
          .fontColor(COLOR_TEXT)
          .height(32)
          .padding({ left: 10, right: 10 })
          .backgroundColor('#1c2026')
          .borderRadius(8)
          .onClick(() => {
            const fit = this.fitScale();
            if (fit > 0) {
              this.setZoom(1 / fit, this.viewportW / 2, this.viewportH / 2);
            }
          })
      }
      .padding(6)
      .backgroundColor('#161a20')
      .borderRadius(10)
      .position({ x: 12, y: 12 })
      .onClick(() => {
        // 工具条自身消费点击：避免空白区域（padding/间距）的点击穿透到下层画布，
        // 否则画笔工具下点「适应」「1:1」会额外落下一笔并让文档版本号 +1。
      })
    }
    .width('100%')
    .height('100%')
    .clip(true)
    .onAreaChange((oldValue: Area, newValue: Area) => {
      this.onViewportChanged(Number(newValue.width), Number(newValue.height));
      this.redrawCanvas();
    })
    .onTouch((event: TouchEvent) => {
      // 悬浮缩放工具条覆盖的左上角区域：整段手势都不交给画布。
      // 仅给工具条加 onClick 不能阻断父容器 onTouch 分发，必须在画布 onTouch 入口拦掉。
      if (event.type === TouchType.Down) {
        const startTouch = event.touches.length > 0 ? event.touches[0] : undefined;
        this.gestureInToolbar = startTouch !== undefined &&
          startTouch.x <= ZOOM_BAR_RIGHT && startTouch.y <= ZOOM_BAR_BOTTOM;
      }
      if (this.gestureInToolbar) {
        if (event.type === TouchType.Up || event.type === TouchType.Cancel) {
          this.gestureInToolbar = false;
        }
        return;
      }
      // 交互式裁剪手柄（C-CROPMODE-UI@1 §3.1）：最先于所有工具分支消费触摸。
      // 位置约束：必须在工具条区域守卫之后（保留 R5.3 的工具条拦截），
      // 必须在 tool==='pan' 与 move/select/shape/text/brush 分支之前（R5.1）。
      if (this.cropMode) {
        const cropTouch = event.touches.length > 0 ? event.touches[0] : undefined;
        if (event.type === TouchType.Down) {
          if (cropTouch === undefined || event.touches.length > 1) {
            // 裁剪模式不支持双指缩放/拖拽：多指落下即放弃本次拖拽（不改变视图）。
            this.cropDragFallbackEnd();
            return;
          }
          this.cropBeginDrag(cropTouch.x, cropTouch.y);
        } else if (event.type === TouchType.Move) {
          if (cropTouch !== undefined && this.cropDragging) {
            this.cropUpdateDrag(cropTouch.x, cropTouch.y);
          }
        } else if (event.type === TouchType.Up) {
          this.cropEndDrag();
        } else if (event.type === TouchType.Cancel) {
          this.cropDragFallbackEnd();
        }
        return;
      }
      // 画布变换手势（C-TRANSFORM-GESTURE@1 §3）：与裁剪同构，整段手势在变换模式内消费。
      if (this.trMode) {
        const trTouch = event.touches.length > 0 ? event.touches[0] : undefined;
        if (event.type === TouchType.Down) {
          if (trTouch === undefined || event.touches.length > 1) {
            // 变换模式不支持双指缩放/拖拽：多指落下即放弃本次拖拽（不改变预览）。
            this.trEndDrag();
            return;
          }
          this.trBeginDrag(trTouch.x, trTouch.y);
        } else if (event.type === TouchType.Move) {
          if (trTouch !== undefined && this.trDragging) {
            this.trUpdateDrag(trTouch.x, trTouch.y);
          }
        } else if (event.type === TouchType.Up) {
          this.trEndDrag();
        } else if (event.type === TouchType.Cancel) {
          this.trEndDrag();
        }
        return;
      }
      if (this.tool === 'pan') {
        return;
      }
      // 移动工具：多指落下即取消（交给 PinchGesture，缩放保持可用）；
      // Move 阶段只算位移，绝不调用 afterEdit / renderPixels / preview / scheduleDetail（R3.2 / R8.1）。
      if (this.tool === 'move') {
        if (event.touches.length > 1) {
          this.cancelMove();
          return;
        }
        if (event.type === TouchType.Down) {
          const touch = event.touches[0];
          this.beginMove(touch.x, touch.y);
        } else if (event.type === TouchType.Move) {
          if (this.movingLayer) {
            const touch = event.touches[0];
            this.updateMove(touch.x, touch.y);
          }
        } else if (event.type === TouchType.Up) {
          this.commitMove();
        } else if (event.type === TouchType.Cancel) {
          this.cancelMove();
        }
        return;
      }
      if (this.tool === 'select') {
        if (event.touches.length > 1) {
          this.cancelSelect();
          return;
        }
        if (event.type === TouchType.Down) {
          const touch = event.touches[0];
          this.beginSelect(touch.x, touch.y);
        } else if (event.type === TouchType.Move) {
          if (this.selecting) {
            const touch = event.touches[0];
            this.updateSelect(touch.x, touch.y);
          }
        } else if (event.type === TouchType.Up) {
          this.commitSelect();
        } else if (event.type === TouchType.Cancel) {
          this.cancelSelect();
        }
        return;
      }
      if (this.tool === 'shape') {
        if (event.touches.length > 1) {
          this.cancelShape();
          return;
        }
        if (event.type === TouchType.Down) {
          const touch = event.touches[0];
          this.beginShape(touch.x, touch.y);
        } else if (event.type === TouchType.Move) {
          if (this.shaping) {
            const touch = event.touches[0];
            this.updateShape(touch.x, touch.y);
          }
        } else if (event.type === TouchType.Up) {
          this.commitShape();
        } else if (event.type === TouchType.Cancel) {
          this.cancelShape();
        }
        return;
      }
      if (this.tool === 'text') {
        if (event.type === TouchType.Down) {
          this.textMultiTouch = event.touches.length > 1;
          if (event.touches.length > 0) {
            this.textTapX = event.touches[0].x;
            this.textTapY = event.touches[0].y;
          }
        } else if (event.type === TouchType.Up) {
          // 注意：Up 事件的 event.touches 在注入式触摸或多指残留时可能非空，
          // 不能据此判断是否落字（否则文字工具永远无法创建图层）。
          // 只要本次手势不是多点触摸，就用 Down 时记录的坐标提交一次。
          if (!this.textMultiTouch) {
            this.commitText(this.textTapX, this.textTapY);
          }
          this.textMultiTouch = false;
        } else if (event.type === TouchType.Cancel) {
          this.textMultiTouch = false;
        }
        return;
      }
      if (event.type === TouchType.Down) {

        if (event.touches.length > 1) {
          this.cancelStroke();
          return;
        }
        const touch = event.touches[0];
        this.beginStroke(touch.x, touch.y);
      } else if (event.type === TouchType.Move) {
        if (this.drawing && event.touches.length === 1) {
          const touch = event.touches[0];
          this.appendStroke(touch.x, touch.y);
        }
      } else if (event.type === TouchType.Up) {
        this.endStroke();
      } else if (event.type === TouchType.Cancel) {
        this.cancelStroke();
      }
    })
    .gesture(
      GestureGroup(GestureMode.Parallel,
        PanGesture({ fingers: 1, distance: 1 })
          .onActionStart((event: GestureEvent) => {
            if (this.trMode) { return; }
            if (this.cropMode) { return; }
            // 裁剪模式下 Pan 不写视图（R5.2）；首行守卫必须位于既有 tool 判定之前。
            if (this.tool !== 'pan' || this.pinchActive) {
              return;
            }
            this.panStartX = this.panX;
            this.panStartY = this.panY;
            this.gestureUpdates = 0;
          })
          .onActionUpdate((event: GestureEvent) => {
            if (this.trMode) { return; }
            if (this.cropMode) { return; }
            // 裁剪模式下 Pan 不修改 panX/panY（R5.2）。
            if (this.tool !== 'pan' || this.pinchActive || this.info === undefined) {
              return;
            }
            // 手势进行中只移动视图与重绘叠加层，绝不触发引擎渲染（R7.3）。
            this.panX = this.panStartX + event.offsetX;
            this.panY = this.panStartY + event.offsetY;
            this.noteGestureUpdate('pan');
            this.redrawCanvas();
          })
          .onActionEnd(() => {
            if (this.trMode) { return; }
            if (this.cropMode) { this.cropDragFallbackEnd(); return; }
            // Up 丢失时的幂等兜底：复位裁剪拖拽态（R5.5）。
            if (this.tool !== 'pan') {
              return;
            }
            this.endGesture('pan');
            this.scheduleDetail();
            this.redrawCanvas();
          })
          .onActionCancel(() => {
            if (this.trMode) { return; }
            if (this.cropMode) { this.cropDragFallbackEnd(); return; }
            // Cancel 同样幂等复位裁剪拖拽态（R5.5）。
            if (this.tool !== 'pan') {
              return;
            }
            this.endGesture('pan');
            this.scheduleDetail();
            this.redrawCanvas();
          }),
        PinchGesture({ fingers: 2 })
          .onActionStart((event: GestureEvent) => {
            if (this.trMode) { return; }
            if (this.cropMode) { return; }
            // 首行最关键：阻止 cropMode 下 pinchActive=true 与 cancelStroke/cancelSelect/cancelShape 副作用（R5.2）。
            this.cancelStroke();
            this.cancelSelect();
            this.cancelShape();
            this.textMultiTouch = true;
            this.pinchActive = true;
            this.pinchStartZoom = this.zoom;
            this.pinchStartPanX = this.panX;
            this.pinchStartPanY = this.panY;
            this.pinchFocusX = event.pinchCenterX;
            this.pinchFocusY = event.pinchCenterY;
            this.gestureUpdates = 0;
          })
          .onActionUpdate((event: GestureEvent) => {
            if (this.trMode) { return; }
            if (this.cropMode) { return; }
            // 裁剪模式下双指不修改 zoom/panX/panY（R5.2）。
            if (this.info === undefined || this.viewportW <= 0 || this.viewportH <= 0) {
              return;
            }
            const next = Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, this.pinchStartZoom * event.scale));
            const ratio = next / this.pinchStartZoom;
            const centerX = this.viewportW / 2;
            const centerY = this.viewportH / 2;
            // 手势进行中只调整缩放/平移并重绘叠加层，绝不触发引擎渲染（R7.3）。
            this.panX = (event.pinchCenterX - centerX) - (this.pinchFocusX - centerX - this.pinchStartPanX) * ratio;
            this.panY = (event.pinchCenterY - centerY) - (this.pinchFocusY - centerY - this.pinchStartPanY) * ratio;
            this.zoom = next;
            this.noteGestureUpdate('pinch');
            this.redrawCanvas();
          })
          .onActionEnd(() => {
            if (this.trMode) { return; }
            if (this.cropMode) { this.cropDragFallbackEnd(); return; }
            // 裁剪模式退出路径：保持 pinchActive=false 并幂等复位裁剪拖拽态（R5.5）。
            this.pinchActive = false;
            this.endGesture('pinch');
            this.scheduleDetail();
            this.redrawCanvas();
          })
          .onActionCancel(() => {
            if (this.trMode) { return; }
            if (this.cropMode) { this.cropDragFallbackEnd(); return; }
            // 同上（R5.5）。
            this.pinchActive = false;
            this.endGesture('pinch');
            this.scheduleDetail();
            this.redrawCanvas();
          })
      )
    )
  }

  @Builder
  BlendChip(label: string, value: string) {
    Text(label)
      .fontSize(10)
      .fontColor(this.layerBlend === value ? '#0b1220' : '#9fc4ff')
      .height(24)
      .padding({ left: 9, right: 9 })
      .backgroundColor(this.layerBlend === value ? COLOR_LAYER_ACCENT : '#22303f')
      .borderRadius(6)
      .onClick(() => {
        this.setLayerBlendMode(value);
      })
  }

  // 剪贴蒙版开关 chip（C2 §1）：无参 Builder，选中态与文案全部读 this.*，不把 @State 作为形参传入。
  // 视觉与 BlendChip 同构（高 24 / 左右内边距 9 / 圆角 6）：启用 COLOR_LAYER_ACCENT 亮蓝底 + 深色字，
  // 未启用 #22303f 静默底 + #55606d 灰字。
  @Builder
  ClipChip() {
    Text(this.selectedLayerClipped() ? '● 剪贴' : '○ 剪贴')
      .fontSize(10)
      .fontColor(this.selectedLayerClipped() ? '#0b1220' : '#55606d')
      .height(24)
      .padding({ left: 9, right: 9 })
      .backgroundColor(this.selectedLayerClipped() ? COLOR_LAYER_ACCENT : '#22303f')
      .borderRadius(6)
      .onClick(() => {
        this.toggleLayerClipped();
      })
  }

  // ===== 图层二次编辑分组（文字 / 形状）=====
  // 约束（C-EDIT-UI@1 §C，本项目已验证的 ArkUI 坑）：
  // 1) EditCard / TextEditGroup / ShapeEditGroup / EditGroupDot 一律无参，内部直接读 this.*；
  // 2) EditColorDot(color, target) / EditFontChip(family) / EditPaletteRow(label, target) 只接收
  //    字面常量或 ForEach 项值，选中态与提交值全部由 this.* 提供，绝不把 @State 值作为形参传入。

  // 编辑卡片骨架：左侧彩色边条 + 标题（'文字编辑'/'形状编辑'）+ 图层号与位置；正文按 editKind 二选一。
  @Builder
  EditCard() {
    Column() {
      Row({ space: 8 }) {
        Column()
          .width(4)
          .height(16)
          .backgroundColor(this.editAccent())
          .borderRadius(2)
        Text(this.editTitle())
          .fontSize(12)
          .fontWeight(FontWeight.Bold)
          .fontColor(this.editTitleColor())
        Text('图层 #' + this.selectedLayerId + ' · 位置 ' + Math.round(this.editPosX) + ', ' +
          Math.round(this.editPosY))
          .fontSize(9)
          .fontColor(this.editMutedColor())
          .layoutWeight(1)
          .maxLines(1)
          .textOverflow({ overflow: TextOverflow.Ellipsis })
      }
      .width('100%')

      if (this.editKind === 'Type') {
        this.TextEditGroup()
      } else {
        this.ShapeEditGroup()
      }
    }
    .width('100%')
    .padding(10)
    .backgroundColor(this.editCardBg())
    .borderRadius(10)
    .border({ width: { left: 4 }, color: this.editAccent() })
    .margin({ top: 10 })
  }

  // 分组色点（6×6，区分参数组）：颜色跟随当前分组类型，与 FX 的圆点写法一致。
  @Builder
  EditGroupDot() {
    Column()
      .width(6)
      .height(6)
      .backgroundColor(this.editAccent())
      .borderRadius(3)
  }

  // 文字编辑分组（控件顺序固定：内容/应用 → 字号 → 字体 → 颜色）。
  @Builder
  TextEditGroup() {
    Column({ space: 6 }) {
      Row({ space: 6 }) {
        this.EditGroupDot()
        TextInput({ placeholder: '文字内容', text: this.editText })
          .fontSize(11)
          .fontColor('#eaffc9')
          .height(30)
          .layoutWeight(1)
          .backgroundColor('#233313')
          .borderRadius(6)
          .onChange((value: string) => {
            // 只写草稿：内容必须点「应用」才提交（R1.2）。
            this.editText = value;
          })
        Button('应用')
          .fontSize(10)
          .height(28)
          .padding({ left: 10, right: 10 })
          .backgroundColor(COLOR_TYPE_ACCENT)
          .fontColor('#1b2a10')
          .borderRadius(6)
          .onClick(() => {
            this.commitEditText();
          })
      }
      .width('100%')
      .margin({ top: 8 })

      Row({ space: 6 }) {
        this.EditGroupDot()
        Text('字号')
          .fontSize(9)
          .fontColor(this.editMutedColor())
          .width(34)
        Slider({ value: this.editSize, min: 8, max: 400, step: 1 })
          .layoutWeight(1)
          .onChange((value: number, mode: SliderChangeMode) => {
            // 拖动中只更新草稿；End/Click 才提交一次（R1.3）。
            this.editSize = value;
            if (mode === SliderChangeMode.End || mode === SliderChangeMode.Click) {
              this.commitEditSize(value);
            }
          })
        Text(Math.round(this.editSize) + 'pt')
          .fontSize(9)
          .fontColor(this.editMutedColor())
          .width(38)
          .textAlign(TextAlign.End)
      }
      .width('100%')

      Row({ space: 6 }) {
        this.EditGroupDot()
        Text('字体')
          .fontSize(9)
          .fontColor(this.editMutedColor())
          .width(34)
        ForEach(this.editFontOptions(), (family: string) => {
          this.EditFontChip(family)
        }, (family: string) => 'edit-font-' + family)
      }
      .width('100%')

      this.EditPaletteRow('颜色', 'text')
    }
    .width('100%')
    .margin({ top: 8 })
  }

  // 形状编辑分组（控件顺序固定：填充 → 描边 → 线宽 → 位置只读提示）。
  @Builder
  ShapeEditGroup() {
    Column({ space: 6 }) {
      if (this.editFillEditable) {
        this.EditPaletteRow('填充', 'fill')
      } else {
        Text('渐变/图案填充暂不支持在此编辑')
          .fontSize(9)
          .fontColor(this.editMutedColor())
          .margin({ top: 8 })
      }

      if (this.editStrokeEditable) {
        this.EditPaletteRow('描边', 'stroke')
      } else {
        Text('该图层没有可编辑的描边')
          .fontSize(9)
          .fontColor(this.editMutedColor())
      }

      Row({ space: 6 }) {
        this.EditGroupDot()
        Text('线宽')
          .fontSize(9)
          .fontColor(this.editMutedColor())
          .width(34)
        Slider({ value: this.editStrokeWidth, min: 0, max: 50, step: 1 })
          .layoutWeight(1)
          .onChange((value: number, mode: SliderChangeMode) => {
            this.editStrokeWidth = value;
            if (mode === SliderChangeMode.End || mode === SliderChangeMode.Click) {
              this.commitEditStrokeWidth(value);
            }
          })
        Text(Math.round(this.editStrokeWidth) + 'px')
          .fontSize(9)
          .fontColor(this.editMutedColor())
          .width(38)
          .textAlign(TextAlign.End)
      }
      .width('100%')

      // 位置只读提示：拖动提交后由 applyEdit 回填刷新。
      Text('位置 ' + Math.round(this.editPosX) + ', ' + Math.round(this.editPosY))
        .fontSize(9)
        .fontColor(this.editMutedColor())
    }
    .width('100%')
    .margin({ top: 8 })
  }

  // 一行色盘：label 与 target 都是字面常量；选中态与提交值全部读 this.*（不传状态值）。
  @Builder
  EditPaletteRow(label: string, target: string) {
    Row({ space: 6 }) {
      this.EditGroupDot()
      Text(label)
        .fontSize(9)
        .fontColor(this.editMutedColor())
        .width(34)
      ForEach(BRUSH_COLORS, (color: string) => {
        this.EditColorDot(color, target)
      }, (color: string) => 'edit-' + target + '-' + color)
    }
    .width('100%')
    .enabled(this.editPaletteEnabled(target))
  }

  // 颜色圆点：color 是 ForEach 项常量、target 是字面常量（'text'|'fill'|'stroke'）。
  @Builder
  EditColorDot(color: string, target: string) {
    Column()
      .width(18)
      .height(18)
      .backgroundColor(color)
      .borderRadius(9)
      .border({ width: this.editDotSelected(target, color) ? 2 : 0, color: '#ffffff' })
      .onClick(() => {
        this.editPickColor(target, color);
      })
  }

  // 字体 chip：family 是 ForEach 项常量；选中态读 this.editFontFamily。
  @Builder
  EditFontChip(family: string) {
    Text(family)
      .fontSize(9)
      .fontColor(this.editFontFamily === family ? '#1b2a10' : '#c7f08c')
      .height(20)
      .padding({ left: 8, right: 8 })
      .backgroundColor(this.editFontFamily === family ? COLOR_TYPE_ACCENT : '#233313')
      .borderRadius(5)
      .onClick(() => {
        this.commitEditFont(family);
      })
  }

  // ===== 画布分组（C-CANVAS-UI@1）=====
  // 放置：LayersCard() 标题行之后、图层滚动列表之前；不新增顶层卡片，不改 Panel() 卡片顺序。
  // 刷新顺序（契约 §「成功后刷新顺序」）：成功且 changed 时
  // resetView → clearAllLayerThumbs → afterEdit → syncSelection → syncCanvasDraft → 状态栏；
  // changed=false 时不重置视图、不清缓存，只提示「无需调整」；失败时只写 status（R6.4）。

  // 宽高草稿回填：从 this.info 取当前文档尺寸（打开/切换文档、展开分组、画布操作成功后调用）。
  // 只写本地草稿，不提交、不调用引擎 —— 保证折叠行/宫格点击零引擎调用。
  private syncCanvasDraft(): void {
    const info = this.info;
    if (info === undefined) {
      this.canvasW = '';
      this.canvasH = '';
      return;
    }
    this.canvasW = info.width.toString();
    this.canvasH = info.height.toString();
  }

  // 宽/高草稿解析：仅接受 >= 1 的整数；空串、非数字、小数一律返回 0（按钮启用条件用它）。
  private canvasDimension(text: string): number {
    const trimmed = text.trim();
    if (trimmed.length === 0) {
      return 0;
    }
    const value = Number(trimmed);
    if (!Number.isFinite(value) || Math.floor(value) !== value || value < 1) {
      return 0;
    }
    return value;
  }

  // 「裁剪到选区」启用条件（契约 §控件表）：有选区 + 非忙 + 有文档。
  private canvasCropEnabled(): boolean {
    return this.docId !== 0 && !this.busy && this.selectionActive;
  }

  // 「应用」启用条件：宽高均可解析为 >= 1 的整数且非忙（有文档由外层分组保证）。
  private canvasApplyEnabled(): boolean {
    if (this.docId === 0 || this.busy) {
      return false;
    }
    return this.canvasDimension(this.canvasW) >= 1 && this.canvasDimension(this.canvasH) >= 1;
  }

  // 折叠行摘要：始终读 this.info，裁剪/撤销后无需手动同步即可显示真实尺寸。
  private canvasSizeLabel(): string {
    const info = this.info;
    if (info === undefined) {
      return '未打开';
    }
    return info.width + ' × ' + info.height;
  }

  // 9 宫格锚点标签：索引 = row * 3 + col（行 上/中/下，列 左/中/右）。
  private canvasAnchorLabel(): string {
    if (this.canvasAnchor === 0) {
      return '左上';
    } else if (this.canvasAnchor === 1) {
      return '上';
    } else if (this.canvasAnchor === 2) {
      return '右上';
    } else if (this.canvasAnchor === 3) {
      return '左';
    } else if (this.canvasAnchor === 5) {
      return '右';
    } else if (this.canvasAnchor === 6) {
      return '左下';
    } else if (this.canvasAnchor === 7) {
      return '下';
    } else if (this.canvasAnchor === 8) {
      return '右下';
    }
    // 4（默认）与越界值都显示中心，与 canvasAnchor 初值一致。
    return '中心';
  }

  // 9 宫格点击：只改本地状态，不触发引擎调用（契约 §控件表）。
  private canvasPickAnchor(index: number): void {
    if (index < 0 || index > 8) {
      return;
    }
    this.canvasAnchor = index;
  }

  // ===== 视图辅助（V2 网格 / 参考线）=====
  // 网格与参考线都由 redrawCanvas 的叠加层绘制，绘制路径零引擎调用（R4.4）。
  // 开关/间距只改 @State 后重绘；参考线增删清与原点复位是文档写操作（调 guidesSet），
  // 但不改变像素，因此成功后不走 afterEdit（它按 revision 调度渲染），只重绘叠加层并写状态栏（R5）。

  // 参考线回填：打开/新建文档与撤销/重做后调用；失败静默置空（叠加层不是主流程）。
  private syncGuides(): void {
    if (this.docId === 0) {
      this.guidesH = [];
      this.guidesV = [];
    } else {
      try {
        const guides: GuidesInfo = Engine.guidesGet(this.docId);
        this.guidesH = guides.horizontal.slice();
        this.guidesV = guides.vertical.slice();
      } catch (err) {
        this.guidesH = [];
        this.guidesV = [];
      }
    }
    // 文档切换后可能带着另一份参考线：先清掉上一帧叠加层，再按新状态重绘。
    this.clearBrushPreview();
    this.redrawCanvas();
  }

  private viewGuideCountLabel(): string {
    return '水平 ' + this.guidesH.length + ' · 竖直 ' + this.guidesV.length;
  }

  private viewSummaryLabel(): string {
    return (this.showGrid ? '网格 ' + this.gridSize : '网格关') + ' · ' + this.viewGuideCountLabel();
  }

  private viewToggleGrid(): void {
    this.showGrid = !this.showGrid;
    // 关闭时先清掉上一层残留的网格线，再由 redrawCanvas 按新状态重绘（可能只剩参考线）。
    if (!this.showGrid) {
      this.clearBrushPreview();
    }
    this.redrawCanvas();
  }

  private viewPickGridSize(size: number): void {
    if (size < 1) {
      return;
    }
    this.gridSize = size;
    this.redrawCanvas();
  }

  private viewToggleGuides(): void {
    this.showGuides = !this.showGuides;
    if (!this.showGuides) {
      this.clearBrushPreview();
    }
    this.redrawCanvas();
  }

  private viewToggleRulers(): void {
    this.showRulers = !this.showRulers;
    // 关闭时先清掉上一帧的尺带残留，再由 redrawCanvas 按新状态重绘其余叠加层。
    if (!this.showRulers) {
      this.clearBrushPreview();
    }
    this.redrawCanvas();
  }

  private prefsSummaryLabel(): string {
    const zoom = this.defaultZoom === 'oneToOne' ? '1:1' : '适应';
    return '标尺' + (this.showRulers ? '开' : '关') + ' · 网格 ' + this.gridSize + ' · ' + zoom;
  }

  // 首选项读取（R2）：先全部读进局部变量，四项都成功后才落到 @State，
  // 任何一步失败都保持默认值——首选项不是主流程，不能因读不到而崩或半更新。
  private loadPrefs(): void {
    try {
      const store = preferences.getPreferencesSync(getContext(this), { name: PREFS_NAME });
      const storedRulers = store.getSync(PREF_KEY_RULERS, false) as boolean;
      const storedGridSize = store.getSync(PREF_KEY_GRID_SIZE, 64) as number;
      const storedGuides = store.getSync(PREF_KEY_GUIDES, true) as boolean;
      const zoomValue = store.getSync(PREF_KEY_DEFAULT_ZOOM, 'fit') as string;
      const storedZoom = zoomValue === 'oneToOne' ? 'oneToOne' : 'fit';
      this.showRulers = storedRulers;
      this.gridSize = storedGridSize;
      this.showGuides = storedGuides;
      this.defaultZoom = storedZoom;
      this.redrawCanvas();
    } catch (err) {
      // 读取失败：全部保持默认值，不改变任何 @State。
    }
  }

  // 首选项写入（R2）：putSync + flush 异步落盘；失败静默，当前会话状态不受影响。
  private savePrefs(): void {
    try {
      const store = preferences.getPreferencesSync(getContext(this), { name: PREFS_NAME });
      store.putSync(PREF_KEY_RULERS, this.showRulers);
      store.putSync(PREF_KEY_GRID_SIZE, this.gridSize);
      store.putSync(PREF_KEY_GUIDES, this.showGuides);
      store.putSync(PREF_KEY_DEFAULT_ZOOM, this.defaultZoom);
      store.flush().catch(() => {
        // 落盘失败：内存态已生效，下次启动回落到旧值即可。
      });
    } catch (err) {
      // 写入失败静默。
    }
  }

  // 打开文档后的初始缩放（R2）：'oneToOne' 走与「1:1」按钮相同的 setZoom 路径，
  // 其余模式沿用调用方已执行的 resetView（适应）。视图尺寸未就绪（fit=0）时不做处理。
  private applyDefaultZoom(): void {
    if (this.defaultZoom !== 'oneToOne') {
      return;
    }
    const fit = this.fitScale();
    if (fit > 0) {
      this.setZoom(1 / fit, this.viewportW / 2, this.viewportH / 2);
    }
  }

  // 参考线全量提交：成功后以引擎返回值回填，再只重绘叠加层（不调 afterEdit）。
  private applyGuides(patch: GuidesPatch, done: string): void {
    if (this.docId === 0) {
      return;
    }
    try {
      const guides = Engine.guidesSet(this.docId, JSON.stringify(patch));
      this.guidesH = guides.horizontal.slice();
      this.guidesV = guides.vertical.slice();
      // 参考线可能被删减（含全部清除）：先清掉上一帧的叠加层，再按新状态重绘。
      this.clearBrushPreview();
      this.redrawCanvas();
      // 参考线是文档写操作（引擎侧已 checkpoint），但不走 afterEdit（不改像素）。
      // 必须单独刷新 info 与历史，否则顶栏「撤销」仍按旧的 canUndo=false 保持禁用，
      // 用户无法撤销任何参考线操作（运行时验证 G7 实测的缺陷）。
      this.refreshInfoOnly();
      this.loadHistory();
      this.status = done;
    } catch (err) {
      this.status = '参考线更新失败：' + this.messageOf(err as Object);
    }
  }

  // 新增参考线取当前视图中心对应的文档坐标（与拖拽工具同一套 screenToDoc 换算）。
  private addHorizontalGuide(): void {
    if (this.docId === 0 || this.busy) {
      return;
    }
    const y = Math.round(this.screenToDocY(this.viewportH / 2));
    const horizontal = this.guidesH.slice();
    horizontal.push(y);
    this.applyGuides({ horizontal: horizontal, vertical: this.guidesV.slice() }, '已添加水平参考线 · y=' + y);
  }

  private addVerticalGuide(): void {
    if (this.docId === 0 || this.busy) {
      return;
    }
    const x = Math.round(this.screenToDocX(this.viewportW / 2));
    const vertical = this.guidesV.slice();
    vertical.push(x);
    this.applyGuides({ horizontal: this.guidesH.slice(), vertical: vertical }, '已添加竖直参考线 · x=' + x);
  }

  private clearGuides(): void {
    if (this.docId === 0 || this.busy) {
      return;
    }
    this.applyGuides({ horizontal: [], vertical: [] }, '已清除参考线');
  }

  // 标尺原点复位：只重置 origin，不动参考线。
  private resetGuideOrigin(): void {
    if (this.docId === 0 || this.busy) {
      return;
    }
    this.applyGuides({ origin: [0, 0] }, '标尺原点已复位');
  }

  // 「裁剪到选区」：矩形选区取精确边界，椭圆/套索/魔棒取其覆盖面包围盒（均由 Engine.selectionInfo 给出）。
  private async canvasCropToSelection(): Promise<void> {
    if (this.docId === 0 || this.busy) {
      return;
    }
    // 1) 以引擎实时选区为准：无选区时只给原因，不调用引擎（R1.4）。
    let selection: SelectionInfo | undefined = undefined;
    try {
      selection = Engine.selectionInfo(this.docId);
    } catch (err) {
      this.status = this.messageOf(err as Object);
      return;
    }
    if (selection === undefined) {
      return;
    }
    const box = selection;
    if (!box.active || box.x1 <= box.x0 || box.y1 <= box.y0) {
      this.status = '请先用选区工具创建选区';
      return;
    }
    const before = this.info;
    this.busy = true;
    try {
      // 2) 调引擎：失败抛异常 → catch 只写 status（不 resetView、不清缓存、不改 docRevision）。
      const next = Engine.documentCrop(this.docId, box.x0, box.y0, box.x1, box.y1);
      // 3) changed 由 UI 比较宽高判定（Engine 只返回 DocumentInfo，不维护第二份真相）。
      const changed = before === undefined || next.width !== before.width || next.height !== before.height;
      if (changed) {
        // 4) 重置视图 + 显式清图层缩略图（缩略图缓存键只有 layerId，不随 revision 失效）。
        this.resetView();
        this.clearAllLayerThumbs();
      }
      // 5) 复用 afterEdit：比较 revision → 拉 info、清 detailKey/detailPixels、重排图层并调度 detail/preview。
      await this.afterEdit();
      // 6) 选区已被引擎裁剪/平移到新画布：同步选区叠加层（不清空，R1.5）。
      this.syncSelection();
      // 7) 回填草稿：afterEdit 已把 this.info 换成新尺寸，因此这里读到的就是新宽高。
      this.syncCanvasDraft();
      // 8) 状态栏。
      if (changed) {
        this.status = '已裁剪到选区 · ' + next.width + ' × ' + next.height;
      } else {
        this.status = '无需调整 · 尺寸未变化';
      }
    } catch (err) {
      this.status = '裁剪失败：' + this.messageOf(err as Object);
    } finally {
      this.busy = false;
    }
  }

  // 「应用」画布尺寸：宽高 + 9 宫格锚点提交一次；成功后刷新顺序与裁剪完全一致。
  // 画布旋转/翻转（Image Rotation）：像素层完整支持；含文字/形状/智能对象时引擎明确拒绝。
  // 与裁剪互斥：进入前先丢弃裁剪框，避免旧框坐标对应旧画布尺寸。
  private async turnDocument(turn: number): Promise<void> {
    if (this.docId === 0 || this.busy) {
      return;
    }
    try {
      this.busy = true;
      if (this.cropMode) {
        this.cropDiscard();
      }
      Engine.documentTurn(this.docId, turn);
      await this.afterEdit();
      this.syncSelection();
      this.syncCanvasDraft();
      this.status = turn === 0 ? '已水平翻转'
        : (turn === 1 ? '已垂直翻转'
        : (turn === 2 ? '已旋转 180°'
        : (turn === 3 ? '已顺时针旋转 90°' : '已逆时针旋转 90°')));
    } catch (err) {
      this.status = '旋转/翻转失败：' + this.messageOf(err as Object);
    } finally {
      this.busy = false;
    }
  }

  private async canvasResizeApply(): Promise<void> {
    if (this.docId === 0 || this.busy) {
      return;
    }
    const width = this.canvasDimension(this.canvasW);
    const height = this.canvasDimension(this.canvasH);
    if (width < 1 || height < 1) {
      // 非法输入：不调用引擎（零副作用），只给原因（R6.1）。
      this.status = '请输入大于 0 的整数宽高';
      return;
    }
    const anchor = this.canvasAnchor;
    const before = this.info;
    this.busy = true;
    try {
      const next = Engine.documentResizeCanvas(this.docId, width, height, anchor);
      const changed = before === undefined || next.width !== before.width || next.height !== before.height;
      if (changed) {
        this.resetView();
        this.clearAllLayerThumbs();
      }
      await this.afterEdit();
      this.syncSelection();
      this.syncCanvasDraft();
      if (changed) {
        this.status = '画布已调整为 ' + next.width + ' × ' + next.height;
      } else {
        this.status = '无需调整 · 尺寸未变化';
      }
    } catch (err) {
      this.status = '画布调整失败：' + this.messageOf(err as Object);
    } finally {
      this.busy = false;
    }
  }

  // 9 宫格单元格：index 由字面常量传入（0..8），选中态读 this.canvasAnchor（不把状态作为形参传）。
  @Builder
  CanvasAnchorCell(index: number) {
    Text(this.canvasAnchor === index ? '●' : '○')
      .fontSize(12)
      .fontColor(this.canvasAnchor === index ? '#0b1220' : FX_TEXT_MUTED)
      .width(26)
      .height(24)
      .textAlign(TextAlign.Center)
      .backgroundColor(this.canvasAnchor === index ? COLOR_LAYER_ACCENT : FX_GROUP_BG)
      .borderRadius(5)
      // 启用条件（契约 §控件表）：有文档才可选锚点。
      .enabled(this.docId !== 0)
      .onClick(() => {
        this.canvasPickAnchor(index);
      })
  }

  // 历史记录面板（只读）：标题行 + 展开后的标签列表。
  // 撤销栈在上（最近一步高亮），重做栈在下；不提供点击跳转，避免多次 undo 触发多轮渲染。
  @Builder
  HistoryGroup() {
    Column() {
      Row({ space: 6 }) {
        Text(this.historyOpen ? '▾ 历史' : '▸ 历史')
          .fontSize(11)
          .fontColor(FX_TEXT)
        Text((this.historyUndo.length + this.historyRedo.length) + ' 步')
          .fontSize(10)
          .fontColor(FX_TEXT_MUTED)
          .layoutWeight(1)
          .textAlign(TextAlign.End)
      }
      .width('100%')
      .height(24)
      .onClick(() => {
        // 展开前拉取一次：覆盖打开/新建文档等未走 afterEdit 的路径。
        if (!this.historyOpen) {
          this.loadHistory();
        }
        this.historyOpen = !this.historyOpen;
      })

      if (this.historyOpen) {
        Column({ space: 2 }) {
          if (this.historyUndo.length === 0 && this.historyRedo.length === 0) {
            Text('暂无操作记录')
              .fontSize(10)
              .fontColor(FX_TEXT_MUTED)
          }
          ForEach(this.historyUndo.slice(0, 12), (label: string, index: number) => {
            Row({ space: 6 }) {
              Text('●')
                .fontSize(8)
                .fontColor(index === 0 ? COLOR_LAYER_ACCENT : FX_TEXT_MUTED)
              Text(label)
                .fontSize(10)
                .fontColor(index === 0 ? FX_TEXT : FX_TEXT_MUTED)
                .maxLines(1)
                .textOverflow({ overflow: TextOverflow.Ellipsis })
                .layoutWeight(1)
            }
            .width('100%')
            .height(18)
            .onClick(() => {
              this.jumpHistory('undo', index);
            })
          }, (label: string, index: number) => 'h-' + index + '-' + label)
          ForEach(this.historyRedo, (label: string, index: number) => {
            Row({ space: 6 }) {
              Text('○')
                .fontSize(8)
                .fontColor(FX_TEXT_MUTED)
              Text(label)
                .fontSize(10)
                .fontColor(FX_TEXT_MUTED)
                .maxLines(1)
                .textOverflow({ overflow: TextOverflow.Ellipsis })
                .layoutWeight(1)
            }
            .width('100%')
            .height(18)
            .onClick(() => {
              this.jumpHistory('redo', index);
            })
          }, (label: string, index: number) => 'r-' + index + '-' + label)
        }
        .width('100%')
        .padding({ top: 4 })
        .alignItems(HorizontalAlign.Start)
      }
    }
    .width('100%')
    .padding({ left: 8, right: 8, top: 6, bottom: 6 })
    .backgroundColor(FX_GROUP_BG)
    .borderRadius(8)
    .alignItems(HorizontalAlign.Start)
    .margin({ bottom: 6 })
  }

  // 画布分组：标题行（▸/▾ 画布 + 当前尺寸摘要）+ 展开后的裁剪/尺寸控件。
  // 视觉与「图层蒙版」「FX 图层样式」同构：卡底 FX_CARD_BG、组底 FX_GROUP_BG、左边条 4。
  @Builder
  CanvasGroup() {
    Column() {
      Row({ space: 8 }) {
        Column()
          .width(8)
          .height(16)
          .backgroundColor(COLOR_LAYER_ACCENT)
          .borderRadius(4)
        Row({ space: 6 }) {
          Text(this.canvasOpen ? '▾ 画布' : '▸ 画布')
            .fontSize(12)
            .fontWeight(FontWeight.Bold)
            .fontColor(FX_TEXT)
          Text(this.canvasSizeLabel())
            .fontSize(9)
            .fontColor(FX_TEXT_MUTED)
            .maxLines(1)
            .textOverflow({ overflow: TextOverflow.Ellipsis })
        }
        .layoutWeight(1)
        .onClick(() => {
          // 只改本地状态：不调用引擎、不 bump revision。
          this.canvasOpen = !this.canvasOpen;
          if (this.canvasOpen) {
            // 展开时按当前文档尺寸预填草稿（只读 this.info）。
            this.syncCanvasDraft();
          }
        })
      }
      .width('100%')

      if (this.canvasOpen) {
        Column({ space: 6 }) {
          // 裁剪到选区行：无选区时按钮禁用并逐字显示原因（R1.4，不存在点击无反馈的控件）。
          Row({ space: 8 }) {
            Button('裁剪到选区')
              .fontSize(10)
              .height(28)
              .padding({ left: 10, right: 10 })
              .backgroundColor(this.canvasCropEnabled() ? COLOR_LAYER_ACCENT : FX_GROUP_BG)
              .fontColor(this.canvasCropEnabled() ? '#0b1220' : FX_DISABLED)
              .borderRadius(6)
              .enabled(this.canvasCropEnabled())
              .onClick(() => {
                this.canvasCropToSelection();
              })
            Text(this.canvasCropEnabled() ? '裁剪为选区包围盒' : '请先用选区工具创建选区')
              .fontSize(9)
              .fontColor(FX_TEXT_MUTED)
              .layoutWeight(1)
              .maxLines(1)
              .textOverflow({ overflow: TextOverflow.Ellipsis })
          }
          .width('100%')

          // 交互式裁剪手柄入口（C-CROPMODE-UI@1 §6）：裁剪模式是画布态，不新增卡片、不新增 Builder。
          Row({ space: 8 }) {
            Button(this.cropMode ? '裁剪中…' : '裁剪手柄')
              .id('cropEnterBtn')
              .fontSize(10)
              .height(28)
              .padding({ left: 10, right: 10 })
              .backgroundColor((this.cropEnabled() && !this.cropMode) ? COLOR_LAYER_ACCENT : FX_GROUP_BG)
              .fontColor((this.cropEnabled() && !this.cropMode) ? '#0b1220' : FX_DISABLED)
              .borderRadius(6)
              .enabled(this.cropEnabled() && !this.cropMode)
              .onClick(() => {
                this.cropEnter();
              })
            if (this.cropMode) {
              Button('应用')
                .id('cropApplyBtn')
                .fontSize(10)
                .height(28)
                .padding({ left: 12, right: 12 })
                .backgroundColor(COLOR_LAYER_ACCENT)
                .fontColor('#0b1220')
                .borderRadius(6)
                .enabled(!this.busy)
                .onClick(() => {
                  this.cropApply();
                })
              Button('取消')
                .id('cropCancelBtn')
                .fontSize(10)
                .height(28)
                .padding({ left: 12, right: 12 })
                .backgroundColor(FX_GROUP_BG)
                .fontColor(FX_TEXT)
                .borderRadius(6)
                .onClick(() => {
                  this.cropCancel();
                })
            }
            Text(this.cropMode ? this.cropSizeLabel()
              : (this.cropEnabled() ? '在画布上拖出裁剪框' : (this.docId === 0 ? '请先打开图片' : '正在处理')))
              .fontSize(9)
              .fontColor(FX_TEXT_MUTED)
              .layoutWeight(1)
              .maxLines(1)
              .textOverflow({ overflow: TextOverflow.Ellipsis })
          }
          .width('100%')

          // 画布旋转/翻转（Image Rotation）：像素层完整支持；含文字/形状/智能对象时引擎明确拒绝。
          Row({ space: 6 }) {
            Button('水平翻转')
              .fontSize(10)
              .height(28)
              .backgroundColor(FX_GROUP_BG)
              .fontColor(FX_TEXT)
              .borderRadius(6)
              .layoutWeight(1)
              .enabled(this.docId !== 0 && !this.busy)
              .onClick(() => {
                this.turnDocument(0);
              })
            Button('垂直翻转')
              .fontSize(10)
              .height(28)
              .backgroundColor(FX_GROUP_BG)
              .fontColor(FX_TEXT)
              .borderRadius(6)
              .layoutWeight(1)
              .enabled(this.docId !== 0 && !this.busy)
              .onClick(() => {
                this.turnDocument(1);
              })
          }
          .width('100%')
          Row({ space: 6 }) {
            Button('逆时针 90°')
              .fontSize(10)
              .height(28)
              .backgroundColor(FX_GROUP_BG)
              .fontColor(FX_TEXT)
              .borderRadius(6)
              .layoutWeight(1)
              .enabled(this.docId !== 0 && !this.busy)
              .onClick(() => {
                this.turnDocument(4);
              })
            Button('顺时针 90°')
              .fontSize(10)
              .height(28)
              .backgroundColor(FX_GROUP_BG)
              .fontColor(FX_TEXT)
              .borderRadius(6)
              .layoutWeight(1)
              .enabled(this.docId !== 0 && !this.busy)
              .onClick(() => {
                this.turnDocument(3);
              })
            Button('180°')
              .fontSize(10)
              .height(28)
              .backgroundColor(FX_GROUP_BG)
              .fontColor(FX_TEXT)
              .borderRadius(6)
              .layoutWeight(1)
              .enabled(this.docId !== 0 && !this.busy)
              .onClick(() => {
                this.turnDocument(2);
              })
          }
          .width('100%')

          // 宽 / 高：onChange 只写草稿，绝不提交引擎（提交只在「应用」）。
          Row({ space: 6 }) {
            Text('宽')
              .fontSize(9)
              .fontColor(FX_TEXT_MUTED)
              .width(16)
            TextInput({ placeholder: '宽度', text: this.canvasW })
              .type(InputType.Number)
              .fontSize(11)
              .fontColor('#e6f0fa')
              .placeholderColor('#6f8aa6')
              .backgroundColor(FX_GROUP_BG)
              .borderRadius(6)
              .height(30)
              .layoutWeight(1)
              .enabled(this.docId !== 0)
              .onChange((value: string) => {
                this.canvasW = value;
              })
            Text('高')
              .fontSize(9)
              .fontColor(FX_TEXT_MUTED)
              .width(16)
            TextInput({ placeholder: '高度', text: this.canvasH })
              .type(InputType.Number)
              .fontSize(11)
              .fontColor('#e6f0fa')
              .placeholderColor('#6f8aa6')
              .backgroundColor(FX_GROUP_BG)
              .borderRadius(6)
              .height(30)
              .layoutWeight(1)
              .enabled(this.docId !== 0)
              .onChange((value: string) => {
                this.canvasH = value;
              })
          }
          .width('100%')

          // 锚点 9 宫格 + 应用：静态 9 个按钮（不用 ForEach，避免 key 缺状态导致选中态不刷新）。
          Row({ space: 8 }) {
            Column({ space: 4 }) {
              Row({ space: 4 }) {
                this.CanvasAnchorCell(0)
                this.CanvasAnchorCell(1)
                this.CanvasAnchorCell(2)
              }

              Row({ space: 4 }) {
                this.CanvasAnchorCell(3)
                this.CanvasAnchorCell(4)
                this.CanvasAnchorCell(5)
              }

              Row({ space: 4 }) {
                this.CanvasAnchorCell(6)
                this.CanvasAnchorCell(7)
                this.CanvasAnchorCell(8)
              }
            }

            Column({ space: 4 }) {
              Button('应用')
                .fontSize(10)
                .height(28)
                .width('100%')
                .padding({ left: 10, right: 10 })
                .backgroundColor(this.canvasApplyEnabled() ? COLOR_LAYER_ACCENT : FX_GROUP_BG)
                .fontColor(this.canvasApplyEnabled() ? '#0b1220' : FX_DISABLED)
                .borderRadius(6)
                .enabled(this.canvasApplyEnabled())
                .onClick(() => {
                  this.canvasResizeApply();
                })
              Text('锚点：' + this.canvasAnchorLabel())
                .fontSize(9)
                .fontColor(FX_TEXT_MUTED)
                .maxLines(1)
                .textOverflow({ overflow: TextOverflow.Ellipsis })
            }
            .alignItems(HorizontalAlign.Start)
            .layoutWeight(1)
          }
          .width('100%')
        }
        .width('100%')
        .margin({ top: 8 })
      }
    }
    .width('100%')
    .padding(10)
    .backgroundColor(FX_CARD_BG)
    .borderRadius(10)
    .border({ width: { left: 4 }, color: COLOR_LAYER_ACCENT })
    .margin({ top: 10 })
  }

  // 视图辅助分组（V2）：网格开关/间距、参考线开关与增删清、标尺原点复位。
  // 与「画布」分组同构（卡底 FX_CARD_BG、左边条 4、组底 FX_GROUP_BG），无参 Builder：
  // 标题、摘要与选中态一律直接读 this.*，不把 @State 作为形参传入（避免重组不刷新）。
  @Builder
  ViewGroup() {
    Column() {
      Row({ space: 8 }) {
        Column()
          .width(8)
          .height(16)
          .backgroundColor(COLOR_LAYER_ACCENT)
          .borderRadius(4)
        Row({ space: 6 }) {
          Text(this.viewOpen ? '▾ 视图' : '▸ 视图')
            .fontSize(12)
            .fontWeight(FontWeight.Bold)
            .fontColor(FX_TEXT)
          Text(this.viewSummaryLabel())
            .fontSize(9)
            .fontColor(FX_TEXT_MUTED)
            .maxLines(1)
            .textOverflow({ overflow: TextOverflow.Ellipsis })
        }
        .layoutWeight(1)
        .onClick(() => {
          // 只改本地状态：不调用引擎、不 bump revision。
          this.viewOpen = !this.viewOpen;
        })
      }
      .width('100%')

      if (this.viewOpen) {
        Column({ space: 6 }) {
          // 网格开关 + 间距单选：两者都只改本地状态后重绘叠加层（零引擎调用）。
          Row({ space: 6 }) {
            Text(this.showGrid ? '● 网格' : '○ 网格')
              .fontSize(10)
              .fontColor(this.showGrid ? '#0b1220' : '#9fc4ff')
              .height(24)
              .padding({ left: 9, right: 9 })
              .backgroundColor(this.showGrid ? COLOR_LAYER_ACCENT : FX_GROUP_BG)
              .borderRadius(6)
              .onClick(() => {
                this.viewToggleGrid();
              })
            Text('间距')
              .fontSize(9)
              .fontColor(FX_TEXT_MUTED)
            Text('32')
              .fontSize(10)
              .fontColor(this.gridSize === 32 ? '#0b1220' : FX_TEXT_MUTED)
              .height(24)
              .padding({ left: 9, right: 9 })
              .backgroundColor(this.gridSize === 32 ? COLOR_LAYER_ACCENT : FX_GROUP_BG)
              .borderRadius(6)
              .onClick(() => {
                this.viewPickGridSize(32);
              })
            Text('64')
              .fontSize(10)
              .fontColor(this.gridSize === 64 ? '#0b1220' : FX_TEXT_MUTED)
              .height(24)
              .padding({ left: 9, right: 9 })
              .backgroundColor(this.gridSize === 64 ? COLOR_LAYER_ACCENT : FX_GROUP_BG)
              .borderRadius(6)
              .onClick(() => {
                this.viewPickGridSize(64);
              })
            Text('128')
              .fontSize(10)
              .fontColor(this.gridSize === 128 ? '#0b1220' : FX_TEXT_MUTED)
              .height(24)
              .padding({ left: 9, right: 9 })
              .backgroundColor(this.gridSize === 128 ? COLOR_LAYER_ACCENT : FX_GROUP_BG)
              .borderRadius(6)
              .onClick(() => {
                this.viewPickGridSize(128);
              })
          }
          .width('100%')

          // 参考线开关 + 条数摘要：开关同样只重绘叠加层。
          Row({ space: 6 }) {
            Text(this.showGuides ? '● 参考线' : '○ 参考线')
              .fontSize(10)
              .fontColor(this.showGuides ? '#0b1220' : '#9fc4ff')
              .height(24)
              .padding({ left: 9, right: 9 })
              .backgroundColor(this.showGuides ? COLOR_LAYER_ACCENT : FX_GROUP_BG)
              .borderRadius(6)
              .onClick(() => {
                this.viewToggleGuides();
              })
            Text(this.viewGuideCountLabel())
              .fontSize(9)
              .fontColor(FX_TEXT_MUTED)
              .layoutWeight(1)
              .maxLines(1)
              .textOverflow({ overflow: TextOverflow.Ellipsis })
          }
          .width('100%')

          // 标尺开关：只改本地状态并重绘叠加层，零引擎调用、零 afterEdit。
          Row({ space: 6 }) {
            Text(this.showRulers ? '● 标尺' : '○ 标尺')
              .fontSize(10)
              .fontColor(this.showRulers ? '#0b1220' : '#9fc4ff')
              .height(24)
              .padding({ left: 9, right: 9 })
              .backgroundColor(this.showRulers ? COLOR_LAYER_ACCENT : FX_GROUP_BG)
              .borderRadius(6)
              .onClick(() => {
                this.viewToggleRulers();
              })
            Text('上 / 左刻度条 · 文档像素')
              .fontSize(9)
              .fontColor(FX_TEXT_MUTED)
              .layoutWeight(1)
              .maxLines(1)
              .textOverflow({ overflow: TextOverflow.Ellipsis })
          }
          .width('100%')

          // 增删清与原点复位是文档写操作：成功后只重绘叠加层，不调 afterEdit。
          Row({ space: 6 }) {
            Button('添加水平线')
              .fontSize(10)
              .height(28)
              .backgroundColor(FX_GROUP_BG)
              .fontColor(FX_TEXT)
              .borderRadius(6)
              .layoutWeight(1)
              .enabled(this.docId !== 0 && !this.busy)
              .onClick(() => {
                this.addHorizontalGuide();
              })
            Button('添加竖线')
              .fontSize(10)
              .height(28)
              .backgroundColor(FX_GROUP_BG)
              .fontColor(FX_TEXT)
              .borderRadius(6)
              .layoutWeight(1)
              .enabled(this.docId !== 0 && !this.busy)
              .onClick(() => {
                this.addVerticalGuide();
              })
          }
          .width('100%')
          Row({ space: 6 }) {
            Button('清除参考线')
              .fontSize(10)
              .height(28)
              .backgroundColor(FX_GROUP_BG)
              .fontColor(FX_TEXT)
              .borderRadius(6)
              .layoutWeight(1)
              .enabled(this.docId !== 0 && !this.busy)
              .onClick(() => {
                this.clearGuides();
              })
            Button('原点复位')
              .fontSize(10)
              .height(28)
              .backgroundColor(FX_GROUP_BG)
              .fontColor(FX_TEXT)
              .borderRadius(6)
              .layoutWeight(1)
              .enabled(this.docId !== 0 && !this.busy)
              .onClick(() => {
                this.resetGuideOrigin();
              })
          }
          .width('100%')
        }
        .width('100%')
        .margin({ top: 8 })
        .alignItems(HorizontalAlign.Start)
      }
    }
    .width('100%')
    .padding(10)
    .backgroundColor(FX_CARD_BG)
    .borderRadius(10)
    .border({ width: { left: 4 }, color: COLOR_LAYER_ACCENT })
    .margin({ top: 10 })
  }

  // 首选项分组（R2）：与「视图」分组同构的折叠卡片，插在视图分组之后、历史之前。
  // 标尺/网格间距/参考线三项切换后立即重绘叠加层并落盘；「打开时缩放」只落盘、不改当前视图。
  @Builder
  PreferencesGroup() {
    Column() {
      Row({ space: 8 }) {
        Column()
          .width(8)
          .height(16)
          .backgroundColor(COLOR_LAYER_ACCENT)
          .borderRadius(4)
        Row({ space: 6 }) {
          Text(this.prefsOpen ? '▾ 首选项' : '▸ 首选项')
            .fontSize(12)
            .fontWeight(FontWeight.Bold)
            .fontColor(FX_TEXT)
          Text(this.prefsSummaryLabel())
            .fontSize(9)
            .fontColor(FX_TEXT_MUTED)
            .maxLines(1)
            .textOverflow({ overflow: TextOverflow.Ellipsis })
        }
        .layoutWeight(1)
        .onClick(() => {
          // 只改本地折叠态：不调用引擎、不 bump revision。
          this.prefsOpen = !this.prefsOpen;
        })
      }
      .width('100%')

      if (this.prefsOpen) {
        Column({ space: 6 }) {
          // 标尺显示：复用视图分组的同一开关路径（内部已 redrawCanvas），再落盘。
          Row({ space: 6 }) {
            Text(this.showRulers ? '● 标尺' : '○ 标尺')
              .fontSize(10)
              .fontColor(this.showRulers ? '#0b1220' : '#9fc4ff')
              .height(24)
              .padding({ left: 9, right: 9 })
              .backgroundColor(this.showRulers ? COLOR_LAYER_ACCENT : FX_GROUP_BG)
              .borderRadius(6)
              .onClick(() => {
                this.viewToggleRulers();
                this.savePrefs();
              })
            Text('启动时恢复标尺')
              .fontSize(9)
              .fontColor(FX_TEXT_MUTED)
              .layoutWeight(1)
              .maxLines(1)
              .textOverflow({ overflow: TextOverflow.Ellipsis })
          }
          .width('100%')

          // 网格默认间距：三档单选，改动立即重绘叠加层并落盘。
          Row({ space: 6 }) {
            Text('网格间距')
              .fontSize(9)
              .fontColor(FX_TEXT_MUTED)
            Text('32')
              .fontSize(10)
              .fontColor(this.gridSize === 32 ? '#0b1220' : FX_TEXT_MUTED)
              .height(24)
              .padding({ left: 9, right: 9 })
              .backgroundColor(this.gridSize === 32 ? COLOR_LAYER_ACCENT : FX_GROUP_BG)
              .borderRadius(6)
              .onClick(() => {
                this.viewPickGridSize(32);
                this.savePrefs();
              })
            Text('64')
              .fontSize(10)
              .fontColor(this.gridSize === 64 ? '#0b1220' : FX_TEXT_MUTED)
              .height(24)
              .padding({ left: 9, right: 9 })
              .backgroundColor(this.gridSize === 64 ? COLOR_LAYER_ACCENT : FX_GROUP_BG)
              .borderRadius(6)
              .onClick(() => {
                this.viewPickGridSize(64);
                this.savePrefs();
              })
            Text('128')
              .fontSize(10)
              .fontColor(this.gridSize === 128 ? '#0b1220' : FX_TEXT_MUTED)
              .height(24)
              .padding({ left: 9, right: 9 })
              .backgroundColor(this.gridSize === 128 ? COLOR_LAYER_ACCENT : FX_GROUP_BG)
              .borderRadius(6)
              .onClick(() => {
                this.viewPickGridSize(128);
                this.savePrefs();
              })
          }
          .width('100%')

          // 参考线默认显示：同标尺，先走现有开关路径再落盘。
          Row({ space: 6 }) {
            Text(this.showGuides ? '● 参考线' : '○ 参考线')
              .fontSize(10)
              .fontColor(this.showGuides ? '#0b1220' : '#9fc4ff')
              .height(24)
              .padding({ left: 9, right: 9 })
              .backgroundColor(this.showGuides ? COLOR_LAYER_ACCENT : FX_GROUP_BG)
              .borderRadius(6)
              .onClick(() => {
                this.viewToggleGuides();
                this.savePrefs();
              })
            Text('启动时恢复参考线')
              .fontSize(9)
              .fontColor(FX_TEXT_MUTED)
              .layoutWeight(1)
              .maxLines(1)
              .textOverflow({ overflow: TextOverflow.Ellipsis })
          }
          .width('100%')

          // 打开文档时的初始缩放：只改首选项，当前视图保持不变。
          Row({ space: 6 }) {
            Text('打开时缩放')
              .fontSize(9)
              .fontColor(FX_TEXT_MUTED)
            Text('适应')
              .fontSize(10)
              .fontColor(this.defaultZoom === 'fit' ? '#0b1220' : FX_TEXT_MUTED)
              .height(24)
              .padding({ left: 9, right: 9 })
              .backgroundColor(this.defaultZoom === 'fit' ? COLOR_LAYER_ACCENT : FX_GROUP_BG)
              .borderRadius(6)
              .onClick(() => {
                this.defaultZoom = 'fit';
                this.savePrefs();
              })
            Text('1:1')
              .fontSize(10)
              .fontColor(this.defaultZoom === 'oneToOne' ? '#0b1220' : FX_TEXT_MUTED)
              .height(24)
              .padding({ left: 9, right: 9 })
              .backgroundColor(this.defaultZoom === 'oneToOne' ? COLOR_LAYER_ACCENT : FX_GROUP_BG)
              .borderRadius(6)
              .onClick(() => {
                this.defaultZoom = 'oneToOne';
                this.savePrefs();
              })
          }
          .width('100%')

          Text('首选项会在下次启动时保持')
            .fontSize(9)
            .fontColor(FX_TEXT_MUTED)
            .width('100%')
        }
        .width('100%')
        .margin({ top: 8 })
        .alignItems(HorizontalAlign.Start)
      }
    }
    .width('100%')
    .padding(10)
    .backgroundColor(FX_CARD_BG)
    .borderRadius(10)
    .border({ width: { left: 4 }, color: COLOR_LAYER_ACCENT })
    .margin({ top: 10 })
  }

  @Builder
  LayersCard() {
    Column() {
      Row({ space: 8 }) {
        Column()
          .width(4)
          .height(18)
          .backgroundColor(COLOR_LAYER_ACCENT)
          .borderRadius(2)
        Text('图层')
          .fontSize(14)
          .fontWeight(FontWeight.Bold)
          .fontColor('#dcecff')
        Text(this.layers.length + ' 个图层')
          .fontSize(10)
          .fontColor('#6f8aa6')
          .layoutWeight(1)
      }
      .width('100%')
      .margin({ bottom: 10 })

      // 画布分组（C-CANVAS-UI@1 §A）：插在图层卡片标题行之后、图层滚动列表之前；
      // 不新增顶层卡片，不改变 Panel() 的 8 个卡片调用顺序。
      this.CanvasGroup()
      // 视图辅助分组（V2）：紧随「画布」分组，同属图层卡片内的折叠区块。
      this.ViewGroup()
      // 首选项分组（R2）：紧随「视图」分组，同属图层卡片内的折叠区块。
      this.PreferencesGroup()
      // 历史记录：紧随画布分组，同属图层卡片内的折叠区块。
      this.HistoryGroup()

      Scroll() {
        Column({ space: 4 }) {
          ForEach(this.layers, (layer: LayerInfo) => {
            Row({ space: 8 }) {
              // 多选勾选标记（S1 §3）：位于行最左侧（折叠符号之前），仅多选模式出现。
              // 不绑定 onClick：整行点击统一由行容器接管，避免行内再出现一个点击热区。
              if (this.multiSelectMode) {
                Text(this.isMultiSelected(layer.id) ? '☑' : '☐')
                  .fontSize(13)
                  .fontColor(this.isMultiSelected(layer.id) ? COLOR_LAYER_ACCENT : '#55606d')
                  .width(18)
                  .textAlign(TextAlign.Center)
              }
              // 组折叠开关（G2 §1）：仅组行出现，位于缩略图左侧；点击只切换折叠，不改变选中。
              if (layer.isGroup) {
                Text(layer.expanded ? '▾' : '▸')
                  .fontSize(13)
                  .fontColor(COLOR_LAYER_ACCENT)
                  .width(18)
                  .textAlign(TextAlign.Center)
                  .onClick(() => {
                    this.toggleGroupExpanded(layer);
                  })
              } else {
                // 非组行占位：宽度与折叠符号一致，保证所有行缩略图左边缘对齐。
                Column()
                  .width(18)
              }
              if (this.layerThumbVersion >= 0 && this.layerThumbCache.has(layer.id)) {
                Image(this.layerThumbCache.get(layer.id) as image.PixelMap)
                  .width(44)
                  .height(44)
                  .borderRadius(3)
                  .backgroundColor('#0f1720')
                  .border({ width: 1, color: '#2a3d55' })
                  .objectFit(ImageFit.Contain)
              } else {
                Column()
                  .width(44)
                  .height(44)
                  .borderRadius(3)
                  .backgroundColor('#1a2530')
                  .border({ width: 1, color: '#2a3d55' })
              }
              Text(layer.visible ? '●' : '○')
                .fontSize(15)
                .fontColor(layer.visible ? '#7ee787' : '#55606d')
                .width(26)
                .textAlign(TextAlign.Center)
                .onClick(() => {
                  this.toggleVisible(layer);
                })
              Column({ space: 2 }) {
                // 名称行（C2 §2）：剪贴层在名称前加 ↳ 标记（10 号 / #9fb3c8）。
                // 标记固定在名称之前，名称用 layoutWeight(1) 占满剩余宽度并保持 ellipsis；
                // 标记字号小于名称字号，行高仍由 13 号名称决定，52 的行高不变。
                Row({ space: 4 }) {
                  if (layer.clipped) {
                    Text('↳')
                      .fontSize(10)
                      .fontColor('#9fb3c8')
                  }
                  Text(layer.name)
                    .fontSize(13)
                    .fontColor(this.layerRowSelected(layer.id) ? '#ffffff' : '#c6d4e4')
                    .maxLines(1)
                    .textOverflow({ overflow: TextOverflow.Ellipsis })
                    .layoutWeight(1)
                }
                .width('100%')
                Text(layer.kind + this.blendSuffix(layer.blend))
                  .fontSize(9)
                  .fontColor(layer.kind === 'Adjustment' ? '#a98bfa' : '#66798f')
                  .maxLines(1)
                  .textOverflow({ overflow: TextOverflow.Ellipsis })
              }
              .alignItems(HorizontalAlign.Start)
              .layoutWeight(1)
              if (layer.opacity < 0.999) {
                Text(Math.round(layer.opacity * 100) + '%')
                  .fontSize(10)
                  .fontColor('#7f8b9a')
              }
            }
            .width('100%')
            .height(52)
            .padding({ left: 6 + layer.depth * 14, right: 10 })
            .backgroundColor(this.layerRowBackground(layer.id))
            .borderRadius(8)
            .border({ width: { left: this.layerRowAccentWidth(layer.id) }, color: COLOR_LAYER_ACCENT })
            .onClick(() => {
              // 多选模式下整行点击只切换勾选，不改变单选（S1 §3）；
              // 退出多选后恢复原有单选行为。
              if (this.multiSelectMode) {
                this.toggleMultiSelected(layer.id);
              } else {
                this.selectLayer(layer);
              }
            })
          }, (layer: LayerInfo) => layer.id.toString() +
            (layer.visible ? ':v' : ':h') + (layer.expanded ? ':e' : ':c') +
            (layer.clipped ? ':k' : ':n') +
            ':' + layer.name + ':' + layer.blend +
            (this.multiSelected.indexOf(layer.id) >= 0 ? ':s' : ':u'))
        }
        .width('100%')
      }
      .constraintSize({ maxHeight: this.isWide ? 300 : 190 })
      .scrollBar(BarState.Auto)

      // 图层组操作（G2 §2 / §3 / §4）：位于图层列表下方，与「上移 / 下移 / 复制 / 合并 / 删除」
      // 同属图层按钮区域；始终渲染，仅按「是否有文档 + 是否选中层 + 是否有图层 + 选中层是否组」切换可用态。
      Row({ space: 6 }) {
        if (!this.multiSelectMode) {
          Button('新建')
            .fontSize(11)
            .height(28)
            .backgroundColor('#22303f')
            .fontColor(this.docId !== 0 && !this.busy ? '#9fc4ff' : '#55606d')
            .borderRadius(6)
            .layoutWeight(1)
            .enabled(this.docId !== 0 && !this.busy)
            .onClick(() => {
              this.createLayer();
            })
          Button('建组')
            .fontSize(11)
            .height(28)
            .backgroundColor('#22303f')
            .fontColor(this.groupEnabled() ? '#9fc4ff' : '#55606d')
            .borderRadius(6)
            .layoutWeight(1)
            .enabled(this.groupEnabled())
            .onClick(() => {
              this.createLayerGroup();
            })
          Button('解组')
            .fontSize(11)
            .height(28)
            .backgroundColor('#22303f')
            .fontColor(this.ungroupEnabled() ? '#9fc4ff' : '#55606d')
            .borderRadius(6)
            .layoutWeight(1)
            .enabled(this.ungroupEnabled())
            .onClick(() => {
              this.ungroupSelected();
            })
        } else {
          // 多选模式：单层「建组 / 解组」让位给全选入口，避免与下方批量「建组 N」语义混淆。
          Button(this.multiAllSelected() ? '清空' : '全选')
            .fontSize(11)
            .height(28)
            .backgroundColor('#22303f')
            .fontColor('#9fc4ff')
            .borderRadius(6)
            .layoutWeight(1)
            .onClick(() => {
              this.toggleMultiSelectAll();
            })
        }
        Button(this.multiSelectMode ? '完成' : '多选')
          .fontSize(11)
          .height(28)
          .backgroundColor(this.multiSelectMode ? '#2b6cf6' : '#22303f')
          .fontColor(this.multiSelectMode ? '#ffffff' : '#9fc4ff')
          .borderRadius(6)
          .layoutWeight(1)
          .onClick(() => {
            this.toggleMultiSelectMode();
          })
      }
      .width('100%')
      .margin({ top: 8, bottom: 6 })

      // 批量操作（S1 §4）：仅多选模式且已勾选时渲染；未进入多选或未勾选时整行不出现（不做禁用占位）。
      if (this.multiSelectMode && this.multiSelected.length > 0) {
        Row({ space: 6 }) {
          Button('删除 ' + this.multiSelected.length)
            .fontSize(11)
            .height(28)
            .backgroundColor('#3a2026')
            .fontColor('#ff8b8b')
            .borderRadius(6)
            .layoutWeight(1)
            .onClick(() => {
              this.deleteMultiSelected();
            })
          Button('复制 ' + this.multiSelected.length)
            .fontSize(11)
            .height(28)
            .backgroundColor('#22303f')
            .fontColor('#9fc4ff')
            .borderRadius(6)
            .layoutWeight(1)
            .onClick(() => {
              this.duplicateMultiSelected();
            })
          Button('建组 ' + this.multiSelected.length)
            .fontSize(11)
            .height(28)
            .backgroundColor('#22303f')
            .fontColor('#9fc4ff')
            .borderRadius(6)
            .layoutWeight(1)
            .onClick(() => {
              this.groupMultiSelected();
            })
        }
        .width('100%')
        .margin({ bottom: 6 })
      }

      if (this.selectedLayerId !== 0) {
        Row({ space: 6 }) {
          if (this.layerThumb !== undefined) {
            Image(this.layerThumb as image.PixelMap)
              .width(88)
              .height(88)
              .borderRadius(4)
              .backgroundColor('#0f1720')
              .border({ width: 1, color: COLOR_LAYER_ACCENT })
              .objectFit(ImageFit.Contain)
          } else {
            Column()
              .width(88)
              .height(88)
              .borderRadius(4)
              .backgroundColor('#1a2530')
              .border({ width: 1, color: COLOR_LAYER_ACCENT })
          }
          TextInput({ placeholder: '图层名称', text: this.layerNameDraft })
            .fontSize(11)
            .fontColor('#e6f0fa')
            .placeholderColor('#6f8aa6')
            .backgroundColor('#22303f')
            .borderRadius(6)
            .height(30)
            .layoutWeight(1)
            .onChange((value: string) => {
              this.layerNameDraft = value;
            })
          Button('重命名')
            .fontSize(11)
            .height(30)
            .backgroundColor('#2b6cf6')
            .fontColor('#ffffff')
            .borderRadius(6)
            .onClick(() => {
              this.renameSelectedLayer();
            })
        }
        .width('100%')
        .margin({ bottom: 6 })

        Row({ space: 6 }) {
          Button('置顶')
            .fontSize(11)
            .height(28)
            .backgroundColor('#22303f')
            .fontColor('#9fc4ff')
            .borderRadius(6)
            .layoutWeight(1)
            .onClick(() => {
              this.moveSelectedLayer(9999);
            })
          Button('上移')
            .fontSize(11)
            .height(28)
            .backgroundColor('#22303f')
            .fontColor('#9fc4ff')
            .borderRadius(6)
            .layoutWeight(1)
            .onClick(() => {
              this.moveSelectedLayer(1);
            })
          Button('下移')
            .fontSize(11)
            .height(28)
            .backgroundColor('#22303f')
            .fontColor('#9fc4ff')
            .borderRadius(6)
            .layoutWeight(1)
            .onClick(() => {
              this.moveSelectedLayer(-1);
            })
          Button('置底')
            .fontSize(11)
            .height(28)
            .backgroundColor('#22303f')
            .fontColor('#9fc4ff')
            .borderRadius(6)
            .layoutWeight(1)
            .onClick(() => {
              this.moveSelectedLayer(-9999);
            })
          Button('复制')
            .fontSize(11)
            .height(28)
            .backgroundColor('#22303f')
            .fontColor('#9fc4ff')
            .borderRadius(6)
            .layoutWeight(1)
            .onClick(() => {
              this.duplicateSelectedLayer();
            })
          Button('合并')
            .fontSize(11)
            .height(28)
            .backgroundColor('#22303f')
            .fontColor('#9fc4ff')
            .borderRadius(6)
            .layoutWeight(1)
            .onClick(() => {
              this.mergeSelectedLayer();
            })
        }
        .width('100%')
        .margin({ bottom: 6 })

        Scroll() {
          Row({ space: 6 }) {
            ForEach(BLEND_OPTIONS, (option: BlendOption) => {
              this.BlendChip(option.label, option.value)
            }, (option: BlendOption) => option.value)
          }
        }
        .scrollable(ScrollDirection.Horizontal)
        .scrollBar(BarState.Off)
        .width('100%')
        .margin({ bottom: 6 })

        // 剪贴蒙版开关（C2 §1）：紧接混合模式 chip 行，仅作用于当前选中层。
        Row({ space: 6 }) {
          Text('剪贴蒙版')
            .fontSize(11)
            .fontColor('#9fb3c8')
            .width(58)
          this.ClipChip()
        }
        .width('100%')
        .margin({ bottom: 6 })

        Row({ space: 8 }) {
          Text('填充')
            .fontSize(11)
            .fontColor('#9fb3c8')
            .width(58)
          Slider({ value: this.layerFillOpacity, min: 0, max: 100, step: 1 })
            .layoutWeight(1)
            .onChange((value: number, mode: SliderChangeMode) => {
              this.layerFillOpacity = value;
              if (mode === SliderChangeMode.End || mode === SliderChangeMode.Click) {
                this.commitLayerFillOpacity(value);
              }
            })
          Text(this.layerFillOpacity + '%')
            .fontSize(11)
            .fontColor('#9fb3c8')
            .width(38)
        }
        .width('100%')
        .margin({ bottom: 4 })

        Row({ space: 8 }) {
          Text('不透明度')
            .fontSize(11)
            .fontColor('#9fb3c8')
            .width(58)
          Slider({ value: this.layerOpacity, min: 0, max: 100, step: 1 })
            .layoutWeight(1)
            .onChange((value: number, mode: SliderChangeMode) => {
              this.layerOpacity = value;
              if (mode === SliderChangeMode.End || mode === SliderChangeMode.Click) {
                this.commitOpacity(value);
              }
            })
          Text(this.layerOpacity + '%')
            .fontSize(11)
            .fontColor('#9fb3c8')
            .width(38)
          Button('删除')
            .fontSize(11)
            .height(26)
            .backgroundColor('#3a2026')
            .fontColor('#ff8b8b')
            .borderRadius(6)
            .onClick(() => {
              this.removeSelected();
            })
        }
        .width('100%')
        .margin({ top: 10 })

        // 编辑分组挂在图层卡片的选中分支内：不新增顶层卡片（R5.2）；
        // 位置固定在不透明度行之后、FX 子面板之前（C-EDIT-UI@1 §C）。
        if (this.selectedLayerId !== 0 && this.editGroupVisible()) {
          this.EditCard()
        }

        // 自由变换子分组：固定在「图层二次编辑」之后、「图层蒙版」之前（C-XFORM-UI@1 §3.1）。
        // 只要处于选中层分支就渲染；Adjustment / Fill 由分组内部置为禁用并给文案（§3.4）。
        this.TransformGroup()

        // 蒙版子分组：仅像素/形状/文字图层出现，固定位于「图层二次编辑」分组之后、「FX 图层样式」分组之前（C-MASK-UI@1 §A / R8.4）。
        if (this.maskSupported()) {
          this.MaskCard()
        }

        this.FxCard()
      }
    }
    .width('100%')
    .padding(12)
    .backgroundColor(COLOR_LAYER_CARD)
    .borderRadius(14)
    .border({ width: { left: 4 }, color: COLOR_LAYER_ACCENT })
  }

  // 图层蒙版子分组（C-MASK-UI@1 §E）：无参 Builder，正文直接读 this.*；
  // 外层样式与 FX 卡片同构（宽 100% / 内边距 10 / 圆角 10 / 左边条 4 / 上外边距 10）。
  @Builder
  MaskSelectionRow() {
    Column({ space: 6 }) {
      Row({ space: 8 }) {
        Button('选区内显示')
          .fontSize(10)
          .height(28)
          .padding({ left: 10, right: 10 })
          .backgroundColor(COLOR_MASK_GROUP)
          .fontColor(this.selectionActive ? '#e2e8f0' : '#55606d')
          .borderRadius(6)
          .enabled(this.selectionActive)
          .onClick(() => {
            this.fillMaskReveal();
          })
        Button('选区内隐藏')
          .fontSize(10)
          .height(28)
          .padding({ left: 10, right: 10 })
          .backgroundColor(COLOR_MASK_GROUP)
          .fontColor(this.selectionActive ? '#e2e8f0' : '#55606d')
          .borderRadius(6)
          .enabled(this.selectionActive)
          .onClick(() => {
            this.fillMaskHide();
          })
      }
      .width('100%')
      if (!this.selectionActive) {
        // 无选区提示（C-MASK-UI@1 §F，逐字）：按钮禁用期间显示，避免用户误以为按钮无响应。
        Text('未选择区域：请先用选区工具框选')
          .fontSize(9)
          .fontColor(COLOR_MASK_MUTED)
          .width('100%')
      }
    }
    .width('100%')
  }

  @Builder
  MaskCard() {
    Column() {
      // 标题行：8×16 边条 + 折叠标题 + 状态摘要 + 右侧启用指示（只读指示，开关在状态行）。
      Row({ space: 8 }) {
        Column()
          .width(8)
          .height(16)
          .backgroundColor(COLOR_MASK_ACCENT)
          .borderRadius(4)
        Row({ space: 6 }) {
          Text(this.maskOpen ? '▾ 图层蒙版' : '▸ 图层蒙版')
            .fontSize(12)
            .fontWeight(FontWeight.Bold)
            .fontColor(COLOR_MASK_TEXT)
          Text(!this.maskInfo.present ? '未添加' :
            (this.maskInfo.enabled ?
              '已启用 · 密度 ' + this.maskDensity + '%' +
                (this.maskFeather > 0 ? ' · 羽化 ' + this.maskFeather : '') : '已停用'))
            .fontSize(9)
            .fontColor(COLOR_MASK_MUTED)
            .maxLines(1)
            .textOverflow({ overflow: TextOverflow.Ellipsis })
        }
        .layoutWeight(1)
        .onClick(() => {
          this.maskOpen = !this.maskOpen;
        })
        if (this.maskInfo.present) {
          Text(this.maskInfo.enabled ? '●' : '○')
            .fontSize(13)
            .fontColor(this.maskInfo.enabled ? '#7ee787' : '#55606d')
            .width(22)
            .textAlign(TextAlign.Center)
        }
      }
      .width('100%')

      if (this.maskOpen) {
        Column({ space: 6 }) {
          // D3：选区行按契约置于状态/添加行之前（恒显示，C-MASK-UI@1 §E/§G）。
          this.MaskSelectionRow()

          if (!this.maskInfo.present) {
            // 添加行：显示全部 / 隐藏全部（底 COLOR_MASK_GROUP，字 #e2e8f0）。
            Row({ space: 8 }) {
              Button('显示全部')
                .fontSize(10)
                .height(28)
                .padding({ left: 10, right: 10 })
                .backgroundColor(COLOR_MASK_GROUP)
                .fontColor('#e2e8f0')
                .borderRadius(6)
                .onClick(() => {
                  this.addMaskReveal();
                })
              Button('隐藏全部')
                .fontSize(10)
                .height(28)
                .padding({ left: 10, right: 10 })
                .backgroundColor(COLOR_MASK_GROUP)
                .fontColor('#e2e8f0')
                .borderRadius(6)
                .onClick(() => {
                  this.addMaskHide();
                })
            }
            .width('100%')

          } else {
            // 状态行：启用/停用开关 + 删除蒙版。
            Row({ space: 8 }) {
              Text(this.maskInfo.enabled ? '启用 ●' : '停用 ○')
                .fontSize(10)
                .fontColor(this.maskInfo.enabled ? '#7ee787' : '#55606d')
                .height(24)
                .padding({ left: 9, right: 9 })
                .backgroundColor(COLOR_MASK_GROUP)
                .borderRadius(5)
                .onClick(() => {
                  this.toggleMaskEnabled();
                })
              Text('删除蒙版')
                .fontSize(10)
                .fontColor(COLOR_MASK_DANGER)
                .height(24)
                .padding({ left: 9, right: 9 })
                .backgroundColor('#3a2026')
                .borderRadius(5)
                .layoutWeight(1)
                .textAlign(TextAlign.Center)
                .onClick(() => {
                  this.removeMask();
                })
            }
            .width('100%')

            // 密度行：拖动只写草稿；End/Click 才提交一次（R4.4）。
            Row({ space: 8 }) {
              Text('密度')
                .fontSize(9)
                .fontColor(COLOR_MASK_MUTED)
                .width(34)
              Slider({ value: this.maskDensity, min: 0, max: 100, step: 1 })
                .layoutWeight(1)
                .onChange((value: number, mode: SliderChangeMode) => {
                  this.maskDensity = value;
                  if (mode === SliderChangeMode.End || mode === SliderChangeMode.Click) {
                    this.commitMaskDensity(value);
                  }
                })
              Text(this.maskDensity + '%')
                .fontSize(9)
                .fontColor(COLOR_MASK_MUTED)
                .width(38)
                .textAlign(TextAlign.End)
            }
            .width('100%')

            // 羽化行：同理只在 End/Click 提交。
            Row({ space: 8 }) {
              Text('羽化')
                .fontSize(9)
                .fontColor(COLOR_MASK_MUTED)
                .width(34)
              Slider({ value: this.maskFeather, min: 0, max: 250, step: 1 })
                .layoutWeight(1)
                .onChange((value: number, mode: SliderChangeMode) => {
                  this.maskFeather = value;
                  if (mode === SliderChangeMode.End || mode === SliderChangeMode.Click) {
                    this.commitMaskFeather(value);
                  }
                })
              Text(Math.round(this.maskFeather) + ' px')
                .fontSize(9)
                .fontColor(COLOR_MASK_MUTED)
                .width(38)
                .textAlign(TextAlign.End)
            }
            .width('100%')

            // 应用行：仅像素图层可点；形状/文字禁用并显示 §F 文案。
            Row({ space: 6 }) {
              Button('应用蒙版')
                .fontSize(10)
                .height(28)
                .padding({ left: 10, right: 10 })
                .backgroundColor(COLOR_MASK_GROUP)
                .fontColor(this.selectedLayerKind() === 'Pixel' ? '#e2e8f0' : '#55606d')
                .borderRadius(6)
                .enabled(this.selectedLayerKind() === 'Pixel')
                .onClick(() => {
                  this.applyMask();
                })
              if (this.selectedLayerKind() !== 'Pixel') {
                Text('形状 / 文字图层需先栅格化（本轮不提供）')
                  .fontSize(9)
                  .fontColor(COLOR_MASK_MUTED)
                  .layoutWeight(1)
                  .maxLines(2)
              }
            }
            .width('100%')
          }

        }
        .width('100%')
        .margin({ top: 8 })
      }
    }
    .width('100%')
    .padding(10)
    .backgroundColor(COLOR_MASK_CARD)
    .borderRadius(10)
    .border({ width: { left: 4 }, color: COLOR_MASK_ACCENT })
    .margin({ top: 10 })
  }

  // 自由变换子分组（C-XFORM-UI@1 §3）：无参 Builder，正文直接读 this.*。
  // 滑块 onChange 只写草稿，唯一提交入口是「应用」（§3.4）；收起时只渲染标题行。
  @Builder
  TransformGroup() {
    Column() {
      Row({ space: 8 }) {
        Column()
          .width(4)
          .height(16)
          .backgroundColor(this.trSupported() ? COLOR_LAYER_ACCENT : FX_DISABLED)
          .borderRadius(2)
        Row({ space: 6 }) {
          Text(this.trOpen ? '▾ 变换' : '▸ 变换')
            .fontSize(12)
            .fontWeight(FontWeight.Bold)
            .fontColor(this.trSupported() ? FX_TEXT : FX_DISABLED)
          Text(this.trSupported() ? (this.trDirty() ? '草稿未应用' : '未修改') : '不支持')
            .fontSize(9)
            .fontColor(FX_TEXT_MUTED)
            .maxLines(1)
            .textOverflow({ overflow: TextOverflow.Ellipsis })
        }
        .layoutWeight(1)
        .onClick(() => {
          this.trOpen = !this.trOpen;
        })
      }
      .width('100%')

      if (this.trOpen) {
        Column({ space: 6 }) {
          if (!this.trSupported()) {
            Text('该图层类型不支持变换')
              .fontSize(9)
              .fontColor(FX_TEXT_MUTED)
              .width('100%')
          }

          Row({ space: 6 }) {
            Text('缩放 X')
              .fontSize(9)
              .fontColor(FX_TEXT_MUTED)
              .width(40)
            Slider({ value: this.trScaleX, min: 10, max: 400, step: 1 })
              .layoutWeight(1)
              .enabled(this.trSupported())
              .onChange((value: number, mode: SliderChangeMode) => {
                // 只写草稿：End/Click 也不提交，统一由「应用」提交一次（§3.4）。
                this.trScaleX = value;
              })
            Text(Math.round(this.trScaleX) + '%')
              .fontSize(9)
              .fontColor(FX_TEXT_MUTED)
              .width(38)
              .textAlign(TextAlign.End)
          }
          .width('100%')

          Row({ space: 6 }) {
            Text('缩放 Y')
              .fontSize(9)
              .fontColor(FX_TEXT_MUTED)
              .width(40)
            Slider({ value: this.trScaleY, min: 10, max: 400, step: 1 })
              .layoutWeight(1)
              .enabled(this.trSupported())
              .onChange((value: number, mode: SliderChangeMode) => {
                this.trScaleY = value;
              })
            Text(Math.round(this.trScaleY) + '%')
              .fontSize(9)
              .fontColor(FX_TEXT_MUTED)
              .width(38)
              .textAlign(TextAlign.End)
          }
          .width('100%')

          Row({ space: 6 }) {
            Text('旋转')
              .fontSize(9)
              .fontColor(FX_TEXT_MUTED)
              .width(40)
            Slider({ value: this.trAngle, min: -180, max: 180, step: 1 })
              .layoutWeight(1)
              .enabled(this.trSupported())
              .onChange((value: number, mode: SliderChangeMode) => {
                this.trAngle = value;
              })
            Text(Math.round(this.trAngle) + '°')
              .fontSize(9)
              .fontColor(FX_TEXT_MUTED)
              .width(38)
              .textAlign(TextAlign.End)
          }
          .width('100%')

          Row({ space: 6 }) {
            Text('偏移 X')
              .fontSize(9)
              .fontColor(FX_TEXT_MUTED)
              .width(40)
            Slider({ value: this.trDx, min: -200, max: 200, step: 1 })
              .layoutWeight(1)
              .enabled(this.trSupported())
              .onChange((value: number, mode: SliderChangeMode) => {
                this.trDx = value;
              })
            Text(Math.round(this.trDx) + ' px')
              .fontSize(9)
              .fontColor(FX_TEXT_MUTED)
              .width(38)
              .textAlign(TextAlign.End)
          }
          .width('100%')

          Row({ space: 6 }) {
            Text('偏移 Y')
              .fontSize(9)
              .fontColor(FX_TEXT_MUTED)
              .width(40)
            Slider({ value: this.trDy, min: -200, max: 200, step: 1 })
              .layoutWeight(1)
              .enabled(this.trSupported())
              .onChange((value: number, mode: SliderChangeMode) => {
                this.trDy = value;
              })
            Text(Math.round(this.trDy) + ' px')
              .fontSize(9)
              .fontColor(FX_TEXT_MUTED)
              .width(38)
              .textAlign(TextAlign.End)
          }
          .width('100%')

          // 变换模式专用行：应用 / 取消（不替换既有滑块「应用」按钮的语义）。
          if (this.trMode) {
            Row({ space: 6 }) {
              Button('应用变换')
                .fontSize(10)
                .height(28)
                .padding({ left: 10, right: 10 })
                .backgroundColor(COLOR_LAYER_ACCENT)
                .fontColor('#1b2a10')
                .borderRadius(6)
                .enabled(!this.busy)
                .onClick(() => {
                  this.trApply();
                })
              Button('取消变换')
                .fontSize(10)
                .height(28)
                .padding({ left: 10, right: 10 })
                .backgroundColor(FX_GROUP_BG)
                .fontColor(FX_TEXT)
                .borderRadius(6)
                .onClick(() => {
                  this.trCancel();
                })
              Text('拖框内平移 · 拖手柄缩放/旋转')
                .fontSize(9)
                .fontColor(FX_TEXT_MUTED)
                .layoutWeight(1)
                .textAlign(TextAlign.End)
            }
            .width('100%')
          }

          Row({ space: 6 }) {
            Button('画布变换')
              .fontSize(10)
              .height(28)
              .padding({ left: 10, right: 10 })
              .backgroundColor((this.trEnabled() && !this.trMode) ? COLOR_LAYER_ACCENT : FX_GROUP_BG)
              .fontColor((this.trEnabled() && !this.trMode) ? '#1b2a10' : FX_DISABLED)
              .borderRadius(6)
              .enabled(this.trEnabled() && !this.trMode)
              .onClick(() => {
                this.trEnter();
              })
            Button('应用')
              .fontSize(10)
              .height(28)
              .padding({ left: 10, right: 10 })
              .backgroundColor(this.trSupported() && this.trDirty() ? COLOR_LAYER_ACCENT : FX_GROUP_BG)
              .fontColor(this.trSupported() && this.trDirty() ? '#1b2a10' : FX_DISABLED)
              .borderRadius(6)
              .enabled(this.trSupported() && this.trDirty())
              .onClick(() => {
                this.applyTransform();
              })
            Button('重置草稿')
              .fontSize(10)
              .height(28)
              .padding({ left: 10, right: 10 })
              .backgroundColor(FX_GROUP_BG)
              .fontColor(this.trDirty() ? FX_TEXT : FX_DISABLED)
              .borderRadius(6)
              .enabled(this.trSupported())
              .onClick(() => {
                this.trResetDraft();
              })
            Text(this.trBoundsLabel())
              .fontSize(9)
              .fontColor(FX_TEXT_MUTED)
              .layoutWeight(1)
              .textAlign(TextAlign.End)
          }
          .width('100%')
          .margin({ top: 4 })
        }
        .width('100%')
        .margin({ top: 8 })
      }
    }
    .width('100%')
    .padding(10)
    .backgroundColor(FX_CARD_BG)
    .borderRadius(10)
    .border({ width: { left: 4 }, color: this.trSupported() ? COLOR_LAYER_ACCENT : FX_DISABLED })
    .margin({ top: 10 })
  }

  // FX 子面板标题行：左侧蓝色边条 + 「FX 图层样式」+ 实例摘要 + 总开关；收起时只渲染标题行。
  @Builder
  FxCard() {
    Column() {
      Row({ space: 8 }) {
        Column()
          .width(4)
          .height(16)
          .backgroundColor(COLOR_LAYER_ACCENT)
          .borderRadius(2)
        Row({ space: 6 }) {
          Text(this.fxOpen ? '▾ FX 图层样式' : '▸ FX 图层样式')
            .fontSize(12)
            .fontWeight(FontWeight.Bold)
            .fontColor(FX_TEXT)
          Text(this.fxItemsSummary())
            .fontSize(9)
            .fontColor(FX_TEXT_MUTED)
            .maxLines(1)
            .textOverflow({ overflow: TextOverflow.Ellipsis })
        }
        .layoutWeight(1)
        .onClick(() => {
          this.fxOpen = !this.fxOpen;
        })
        Text(this.fxInfo.enabled ? '总开关 ●' : '总开关 ○')
          .fontSize(9)
          .fontColor(this.fxInfo.enabled ? FX_ENABLED : FX_DISABLED)
          .height(20)
          .padding({ left: 8, right: 8 })
          .backgroundColor(FX_GROUP_BG)
          .borderRadius(5)
          .onClick(() => {
            this.fxMaster(!this.fxInfo.enabled);
          })
      }
      .width('100%')

      if (this.fxOpen) {
        Column({ space: 6 }) {
          Text('当前编辑：' + this.fxKindLabel(this.fxKind) + ' · 点击分组标题切换')
            .fontSize(9)
            .fontColor(FX_TEXT_MUTED)
            .maxLines(1)
            .textOverflow({ overflow: TextOverflow.Ellipsis })

          // 七组固定顺序：投影 / 描边 / 发光 / 内阴影 / 内发光 / 渐变叠加 / 斜面与浮雕。
          this.FxGroup('投影', 'dropShadow')
          this.FxGroup('描边', 'stroke')
          this.FxGroup('发光', 'outerGlow')
          this.FxGroup('内阴影', 'innerShadow')
          this.FxGroup('内发光', 'innerGlow')
          this.FxGroup('渐变叠加', 'gradientOverlay')
          this.FxGroup('斜面与浮雕', 'bevelEmboss')

          if (this.fxUnsupported().length > 0) {
            Column({ space: 4 }) {
              ForEach(this.fxUnsupported(), (item: LayerEffectInfo) => {
                Row({ space: 6 }) {
                  Text(item.label)
                    .fontSize(10)
                    .fontColor(FX_TEXT)
                  Text('暂不支持编辑')
                    .fontSize(9)
                    .fontColor(FX_TEXT_MUTED)
                }
                .width('100%')
              }, (item: LayerEffectInfo) => 'fx-unsupported-' + item.index)
            }
            .width('100%')
            .padding(8)
            .backgroundColor(FX_GROUP_BG)
            .borderRadius(8)
          }

          Text('清除 FX')
            .fontSize(9)
            .fontColor('#ff8b8b')
            .height(22)
            .padding({ left: 10, right: 10 })
            .backgroundColor('#3a2026')
            .borderRadius(5)
            .onClick(() => {
              this.fxClear();
            })
        }
        .width('100%')
        .margin({ top: 8 })
      }
    }
    .width('100%')
    .padding(10)
    .backgroundColor(FX_CARD_BG)
    .borderRadius(10)
    .border({ width: { left: 4 }, color: COLOR_LAYER_ACCENT })
    .margin({ top: 10 })
  }

  // 单个效果分组：标题（含实例数）、启用/删除或添加；当前编辑分组才展开参数控件。
  @Builder
  FxGroup(label: string, kind: string) {
    Column() {
      Row({ space: 6 }) {
        Column()
          .width(6)
          .height(6)
          .backgroundColor(this.fxGroupAccent(kind))
          .borderRadius(3)
        Text(this.fxGroupTitle(label, kind))
          .fontSize(11)
          .fontWeight(FontWeight.Medium)
          .fontColor(FX_TEXT)
          .layoutWeight(1)
          .onClick(() => {
            this.fxSelectKind(kind);
          })
        if (this.fxCount(kind) > 0) {
          Text(this.fxEnabledOf(kind) ? '●' : '○')
            .fontSize(13)
            .fontColor(this.fxEnabledOf(kind) ? FX_ENABLED : FX_DISABLED)
            .width(22)
            .textAlign(TextAlign.Center)
            .onClick(() => {
              this.fxToggle(kind);
            })
          Text('删除')
            .fontSize(9)
            .fontColor('#ff8b8b')
            .height(20)
            .padding({ left: 8, right: 8 })
            .backgroundColor('#3a2026')
            .borderRadius(5)
            .onClick(() => {
              this.fxRemove(kind);
            })
        } else {
          Text('添加')
            .fontSize(9)
            .fontColor('#0b1220')
            .height(20)
            .padding({ left: 9, right: 9 })
            .backgroundColor(FX_ENABLED)
            .borderRadius(5)
            .onClick(() => {
              this.fxAdd(kind);
            })
        }
      }
      .width('100%')

      if (this.fxKind === kind) {
        if (this.fxCount(kind) > 0) {
          Column({ space: 4 }) {
            if (kind === 'dropShadow' || kind === 'stroke' || kind === 'outerGlow') {
              // 既有三组：控件顺序与 C-FX-UI@1 完全一致（不改）。
              this.FxColorRow()
              this.FxSliderRow('不透明度', 'opacity', 0, 100, '%')
              if (kind === 'dropShadow') {
                this.FxSliderRow('角度', 'angle', -180, 180, '°')
                this.FxSliderRow('距离', 'distance', 0, 100, 'px')
                this.FxSliderRow('模糊', 'blur', 0, 250, 'px')
                this.FxSliderRow('扩展', 'spread', 0, 100, '%')
              } else if (kind === 'stroke') {
                this.FxSliderRow('宽度', 'width', 1, 100, 'px')
                this.FxPositionRow()
              } else {
                this.FxSliderRow('模糊', 'blur', 0, 250, 'px')
                this.FxSliderRow('扩展', 'spread', 0, 100, '%')
              }
            } else if (kind === 'innerShadow') {
              this.FxColorSlotRow('颜色', 'color')
              this.FxSliderRow('不透明度', 'opacity', 0, 100, '%')
              this.FxSliderRow('角度', 'angle', -180, 180, '°')
              this.FxSliderRow('距离', 'distance', 0, 100, 'px')
              this.FxSliderRow('扩展', 'spread', 0, 100, '%')
              this.FxSliderRow('模糊', 'blur', 0, 250, 'px')
              this.FxToggleRow('抗锯齿', 'antiAlias')
            } else if (kind === 'innerGlow') {
              this.FxColorSlotRow('颜色', 'color')
              this.FxSliderRow('不透明度', 'opacity', 0, 100, '%')
              this.FxSliderRow('扩展', 'spread', 0, 100, '%')
              this.FxSliderRow('模糊', 'blur', 0, 250, 'px')
              this.FxGlowTechniqueRow()
              this.FxGlowSourceRow()
              this.FxToggleRow('抗锯齿', 'antiAlias')
            } else if (kind === 'gradientOverlay') {
              this.FxColorSlotRow('起始色', 'gradStart')
              this.FxColorSlotRow('结束色', 'gradEnd')
              this.FxGradientStyleRow()
              this.FxSliderRow('角度', 'angle', 0, 360, '°')
              this.FxSliderRow('缩放', 'gradScale', 10, 150, '%')
              this.FxToggleRow('反向', 'reverse')
              this.FxToggleRow('对齐', 'align')
              this.FxToggleRow('抖动', 'dither')
              this.FxSliderRow('不透明度', 'opacity', 0, 100, '%')
            } else if (kind === 'bevelEmboss') {
              this.FxBevelStyleRow()
              this.FxBevelTechniqueRow()
              this.FxSliderRow('深度', 'bevelDepth', 0, 1000, '%')
              this.FxBevelDirectionRow()
              this.FxSliderRow('大小', 'blur', 0, 250, 'px')
              this.FxSliderRow('柔化', 'bevelSoften', 0, 100, 'px')
              this.FxSliderRow('角度', 'angle', -180, 180, '°')
              this.FxSliderRow('高度角', 'bevelAltitude', 0, 90, '°')
              this.FxColorSlotRow('高光色', 'highlight')
              this.FxSliderRow('高光', 'highlightOpacity', 0, 100, '%')
              this.FxColorSlotRow('阴影色', 'shadow')
              this.FxSliderRow('阴影', 'shadowOpacity', 0, 100, '%')
              this.FxToggleRow('抗锯齿', 'antiAlias')
            }
          }
          .width('100%')
          .margin({ top: 6 })
        } else {
          Text('尚无' + label + '效果，点击「添加」')
            .fontSize(9)
            .fontColor(FX_TEXT_MUTED)
            .margin({ top: 4 })
        }
      } else if (this.fxCount(kind) > 0) {
        Text(this.fxSummaryLine(kind))
          .fontSize(9)
          .fontColor(FX_TEXT_MUTED)
          .margin({ top: 4 })
          .maxLines(2)
          .textOverflow({ overflow: TextOverflow.Ellipsis })
          .onClick(() => {
            this.fxSelectKind(kind);
          })
      }
    }
    .width('100%')
    .padding(8)
    .backgroundColor(this.fxKind === kind ? FX_GROUP_ACTIVE_BG : FX_GROUP_BG)
    .borderRadius(8)
    // 3px 同色左边条：七组统一，既有三组只增加此边条，色点/背景/控件顺序不变。
    .border({ width: { left: 3 }, color: this.fxGroupAccent(kind) })
  }

  // 参数行：数值不走 Builder 参数，直接读组件状态，保证拖动/提交后都能刷新。
  @Builder
  FxSliderRow(label: string, name: string, min: number, max: number, unit: string) {
    Row({ space: 6 }) {
      Text(label)
        .fontSize(9)
        .fontColor(FX_TEXT_MUTED)
        .width(44)
      Slider({ value: this.fxDraftValue(name), min: min, max: max, step: 1 })
        .layoutWeight(1)
        .onChange((value: number, mode: SliderChangeMode) => {
          this.fxOnSlider(name, value, mode);
        })
      Text(this.fxFormatValue(name, unit))
        .fontSize(9)
        .fontColor(FX_TEXT_MUTED)
        .width(40)
        .textAlign(TextAlign.End)
    }
    .width('100%')
  }

  // 颜色色板复用 BRUSH_COLORS，选中色以边框高亮。
  @Builder
  FxColorRow() {
    Row({ space: 6 }) {
      Text('颜色')
        .fontSize(9)
        .fontColor(FX_TEXT_MUTED)
        .width(34)
      ForEach(BRUSH_COLORS, (color: string) => {
        Column()
          .width(18)
          .height(18)
          .backgroundColor(color)
          .borderRadius(9)
          .border({ width: this.fxColor === color ? 2 : 0, color: FX_TEXT })
          .onClick(() => {
            this.fxSetColor(this.fxKind, color);
          })
      }, (color: string) => 'fx-color-' + color)
    }
    .width('100%')
  }

  @Builder
  FxPositionRow() {
    Row({ space: 6 }) {
      Text('位置')
        .fontSize(9)
        .fontColor(FX_TEXT_MUTED)
        .width(34)
      this.FxPositionChip('外', 'outside')
      this.FxPositionChip('中', 'center')
      this.FxPositionChip('内', 'inside')
    }
    .width('100%')
  }

  @Builder
  FxPositionChip(label: string, value: string) {
    Text(label)
      .fontSize(9)
      .fontColor(this.fxPosition === value ? '#0b1220' : '#9fc4ff')
      .height(20)
      .padding({ left: 9, right: 9 })
      .backgroundColor(this.fxPosition === value ? COLOR_LAYER_ACCENT : FX_GROUP_ACTIVE_BG)
      .borderRadius(5)
      .onClick(() => {
        this.fxSetPosition(value);
      })
  }

  // 指定颜色槽的颜色行：label + slot 仅为标识常量，选中态与提交值都直接读 this.*（不传状态值）。
  @Builder
  FxColorSlotRow(label: string, slot: string) {
    Row({ space: 6 }) {
      Text(label)
        .fontSize(9)
        .fontColor(FX_TEXT_MUTED)
        .width(52)
      ForEach(BRUSH_COLORS, (color: string) => {
        Column()
          .width(18)
          .height(18)
          .backgroundColor(color)
          .borderRadius(9)
          .border({ width: this.fxSlotColor(slot) === color ? 2 : 0, color: FX_TEXT })
          .onClick(() => {
            this.fxSetColorSlot(slot, color);
          })
      }, (color: string) => 'fx-slot-' + slot + '-' + color)
    }
    .width('100%')
  }

  // 通用 chip：name/value 是标识常量；选中态由 fxChipSelected 内部读 this.* 状态。
  @Builder
  FxChip(label: string, name: string, value: string) {
    Text(label)
      .fontSize(9)
      .fontColor(this.fxChipSelected(name, value) ? '#0b1220' : '#9fc4ff')
      .height(20)
      .padding({ left: 6, right: 6 })
      .backgroundColor(this.fxChipSelected(name, value) ? COLOR_LAYER_ACCENT : FX_GROUP_ACTIVE_BG)
      .borderRadius(5)
      .onClick(() => {
        this.fxSelectChip(name, value);
      })
  }

  // 渐变类型：5 选（linear / radial / angle / reflected / diamond）。
  @Builder
  FxGradientStyleRow() {
    Row({ space: 4 }) {
      Text('类型')
        .fontSize(9)
        .fontColor(FX_TEXT_MUTED)
        .width(34)
      this.FxChip('线性', 'gradientStyle', 'linear')
      this.FxChip('径向', 'gradientStyle', 'radial')
      this.FxChip('角度', 'gradientStyle', 'angle')
      this.FxChip('对称', 'gradientStyle', 'reflected')
      this.FxChip('菱形', 'gradientStyle', 'diamond')
    }
    .width('100%')
  }

  // 斜面样式：5 选。
  @Builder
  FxBevelStyleRow() {
    Row({ space: 4 }) {
      Text('样式')
        .fontSize(9)
        .fontColor(FX_TEXT_MUTED)
        .width(34)
      this.FxChip('外斜面', 'bevelStyle', 'outerBevel')
      this.FxChip('内斜面', 'bevelStyle', 'innerBevel')
      this.FxChip('浮雕', 'bevelStyle', 'emboss')
      this.FxChip('枕状浮雕', 'bevelStyle', 'pillowEmboss')
      this.FxChip('描边浮雕', 'bevelStyle', 'strokeEmboss')
    }
    .width('100%')
  }

  // 斜面技术：3 选。
  @Builder
  FxBevelTechniqueRow() {
    Row({ space: 4 }) {
      Text('技术')
        .fontSize(9)
        .fontColor(FX_TEXT_MUTED)
        .width(34)
      this.FxChip('平滑', 'bevelTechnique', 'smooth')
      this.FxChip('硬性雕刻', 'bevelTechnique', 'chiselHard')
      this.FxChip('软性雕刻', 'bevelTechnique', 'chiselSoft')
    }
    .width('100%')
  }

  // 内发光技术：柔和 / 精确（选中态读 fxInfo 派生值）。
  @Builder
  FxGlowTechniqueRow() {
    Row({ space: 4 }) {
      Text('技术')
        .fontSize(9)
        .fontColor(FX_TEXT_MUTED)
        .width(34)
      this.FxChip('柔和', 'technique', 'softer')
      this.FxChip('精确', 'technique', 'precise')
    }
    .width('100%')
  }

  // 内发光源：边缘 / 中心。
  @Builder
  FxGlowSourceRow() {
    Row({ space: 4 }) {
      Text('源')
        .fontSize(9)
        .fontColor(FX_TEXT_MUTED)
        .width(34)
      this.FxChip('边缘', 'glowSource', 'edge')
      this.FxChip('中心', 'glowSource', 'center')
    }
    .width('100%')
  }

  // 斜面方向：上 / 下（互换高光与阴影位置）。
  @Builder
  FxBevelDirectionRow() {
    Row({ space: 4 }) {
      Text('方向')
        .fontSize(9)
        .fontColor(FX_TEXT_MUTED)
        .width(34)
      this.FxChip('上', 'bevelUp', 'up')
      this.FxChip('下', 'bevelUp', 'down')
    }
    .width('100%')
  }

  // 开/关 chip：name 为标识常量，当前态由 fxToggleValue 内部读 this.* 状态。
  @Builder
  FxToggleRow(label: string, name: string) {
    Row({ space: 6 }) {
      Text(label)
        .fontSize(9)
        .fontColor(FX_TEXT_MUTED)
        .width(52)
      Text(this.fxToggleValue(name) ? '开' : '关')
        .fontSize(9)
        .fontColor(this.fxToggleValue(name) ? '#0b1220' : '#9fc4ff')
        .height(20)
        .padding({ left: 10, right: 10 })
        .backgroundColor(this.fxToggleValue(name) ? COLOR_LAYER_ACCENT : FX_GROUP_ACTIVE_BG)
        .borderRadius(5)
        .onClick(() => {
          this.fxSetToggle(name, !this.fxToggleValue(name));
        })
    }
    .width('100%')
  }

  @Builder
  ToolChip(label: string, value: string) {
    Text(label)
      .fontSize(11)
      .fontColor(this.tool === value ? '#1a1206' : '#f3c98b')
      .height(28)
      .padding({ left: 12, right: 12 })
      .backgroundColor(this.tool === value ? COLOR_BRUSH_ACCENT : '#3d2a15')
      .borderRadius(7)
      .onClick(() => {
        this.selectTool(value);
      })
  }

  @Builder
  BrushCard() {
    Column() {
      Row({ space: 8 }) {
        Column()
          .width(4)
          .height(18)
          .backgroundColor(COLOR_BRUSH_ACCENT)
          .borderRadius(2)
        Text('画笔')
          .fontSize(14)
          .fontWeight(FontWeight.Bold)
          .fontColor('#ffe9c7')
        Text(this.tool === 'pan' ? '移动与缩放' : (this.tool === 'move' ? '移动图层' :
          (this.tool === 'eraser' ? '橡皮擦' : '绘制')))
          .fontSize(10)
          .fontColor('#b08a55')
          .layoutWeight(1)
      }
      .width('100%')
      .margin({ bottom: 10 })

      // 工具切换：4 个 chip 在窄面板下会超出宽度，用横向 Scroll 承接（不放宽既有 chip 尺寸）。
      Scroll() {
        Row({ space: 6 }) {
          this.ToolChip('移动', 'pan')
          this.ToolChip('移动图层', 'move')
          this.ToolChip('画笔', 'brush')
          this.ToolChip('橡皮', 'eraser')
        }
      }
      .scrollable(ScrollDirection.Horizontal)
      .scrollBar(BarState.Off)
      .width('100%')

      // 移动工具不落笔：不显示画笔参数，避免误导（画笔/橡皮仍走各自分支）。
      if (this.tool !== 'pan' && this.tool !== 'move') {
        Row({ space: 8 }) {
          Text('大小')
            .fontSize(11)
            .fontColor('#e8c48f')
            .width(40)
          Slider({ value: this.brushSize, min: 1, max: 200, step: 1 })
            .layoutWeight(1)
            .onChange((value: number, _mode: SliderChangeMode) => {
              this.brushSize = value;
            })
          Text(Math.round(this.brushSize).toString())
            .fontSize(11)
            .fontColor('#e8c48f')
            .width(34)
        }
        .width('100%')
        .margin({ top: 8 })

        Row({ space: 8 }) {
          Text('不透明')
            .fontSize(11)
            .fontColor('#e8c48f')
            .width(46)
          Slider({ value: this.brushOpacity, min: 1, max: 100, step: 1 })
            .layoutWeight(1)
            .onChange((value: number, _mode: SliderChangeMode) => {
              this.brushOpacity = value;
            })
          Text(Math.round(this.brushOpacity) + '%')
            .fontSize(11)
            .fontColor('#e8c48f')
            .width(38)
        }
        .width('100%')

        Row({ space: 8 }) {
          Text('硬度')
            .fontSize(11)
            .fontColor('#e8c48f')
            .width(40)
          Slider({ value: this.brushHardness, min: 0, max: 100, step: 1 })
            .layoutWeight(1)
            .onChange((value: number, _mode: SliderChangeMode) => {
              this.brushHardness = value;
            })
          Text(Math.round(this.brushHardness) + '%')
            .fontSize(11)
            .fontColor('#e8c48f')
            .width(38)
        }
        .width('100%')

        if (this.tool === 'brush') {
          Row({ space: 8 }) {
            Text('颜色')
              .fontSize(11)
              .fontColor('#e8c48f')
              .width(40)
            ForEach(BRUSH_COLORS, (color: string) => {
              Column()
                .width(24)
                .height(24)
                .backgroundColor(color)
                .borderRadius(12)
                .border({ width: this.brushColor === color ? 2 : 0, color: '#f8d9a0' })
                .onClick(() => {
                  this.brushColor = color;
                })
            }, (color: string) => color)
          }
          .width('100%')
          .margin({ top: 4 })
        }
      }
    }
    .width('100%')
    .padding(12)
    .backgroundColor(COLOR_BRUSH_CARD)
    .borderRadius(14)
    .border({ width: { left: 4 }, color: COLOR_BRUSH_ACCENT })
  }

  @Builder
  SelectChip(label: string, kind: string) {
    Text(label)
      .fontSize(11)
      .fontColor(this.tool === 'select' && this.selectionKind === kind ? '#062a26' : '#8ee9df')
      .height(28)
      .padding({ left: 10, right: 10 })
      .backgroundColor(this.tool === 'select' && this.selectionKind === kind ? COLOR_SELECT_ACCENT : '#15443f')
      .borderRadius(7)
      .onClick(() => {
        this.selectionKind = kind;
        this.selectTool('select');
      })
  }

  @Builder
  SelectModeChip(label: string, mode: string) {
    Text(label)
      .fontSize(10)
      .fontColor(this.selectionMode === mode ? '#062a26' : '#8ee9df')
      .height(24)
      .padding({ left: 10, right: 10 })
      .backgroundColor(this.selectionMode === mode ? COLOR_SELECT_ACCENT : '#103a36')
      .borderRadius(6)
      .onClick(() => {
        this.selectionMode = mode;
      })
  }

  @Builder
  SelectAction(label: string, kind: string) {
    Text(label)
      .fontSize(11)
      .fontColor('#bff5ee')
      .height(28)
      .padding({ left: 12, right: 12 })
      .backgroundColor('#15443f')
      .borderRadius(7)
      .onClick(() => {
        this.runSelect(kind);
      })
  }

  @Builder
  SelectCard() {
    Column() {
      Row({ space: 8 }) {
        Column()
          .width(4)
          .height(18)
          .backgroundColor(COLOR_SELECT_ACCENT)
          .borderRadius(2)
        Text('选区')
          .fontSize(14)
          .fontWeight(FontWeight.Bold)
          .fontColor('#d9fff9')
        Text(this.tool === 'select' ? '拖拽创建选区' : '选择工具后拖拽')
          .fontSize(10)
          .fontColor('#6fb8ae')
          .layoutWeight(1)
      }
      .width('100%')
      .margin({ bottom: 10 })

      Row({ space: 6 }) {
        this.SelectChip('矩形', 'rect')
        this.SelectChip('椭圆', 'ellipse')
        this.SelectChip('套索', 'lasso')
        this.SelectChip('魔棒', 'wand')
      }
      .width('100%')

      Row({ space: 6 }) {
        this.SelectModeChip('新建', 'replace')
        this.SelectModeChip('相加', 'add')
        this.SelectModeChip('减去', 'subtract')
      }
      .width('100%')
      .margin({ top: 6 })

      Row({ space: 8 }) {
        Text('羽化')
          .fontSize(11)
          .fontColor('#8ee9df')
          .width(40)
        Slider({ value: this.selectionFeather, min: 0, max: 100, step: 1 })
          .layoutWeight(1)
          .onChange((value: number, _mode: SliderChangeMode) => {
            this.selectionFeather = value;
          })
        Text(Math.round(this.selectionFeather) + 'px')
          .fontSize(11)
          .fontColor('#8ee9df')
          .width(38)
      }
      .width('100%')
      .margin({ top: 6 })

      if (this.selectionKind === 'wand' && this.tool === 'select') {
        Row({ space: 8 }) {
          Text('容差')
            .fontSize(11)
            .fontColor('#8ee9df')
            .width(40)
          Slider({ value: this.wandTolerance, min: 0, max: 100, step: 1 })
            .layoutWeight(1)
            .onChange((value: number, _mode: SliderChangeMode) => {
              this.wandTolerance = value;
            })
          Text(Math.round(this.wandTolerance).toString())
            .fontSize(11)
            .fontColor('#8ee9df')
            .width(34)
        }
        .width('100%')
        .margin({ top: 6 })
      }

      Row({ space: 6 }) {
        this.SelectAction('全选', 'all')
        this.SelectAction('取消', 'none')
        this.SelectAction('反选', 'invert')
      }
      .width('100%')
      .margin({ top: 8 })

      Text(this.selectionActive ? '选区 ' + Math.abs(this.selX1 - this.selX0) + ' × ' + Math.abs(this.selY1 - this.selY0) + ' px · 画笔与滤镜仅作用于选区内' : '未选择区域：效果作用于整幅图像')
        .fontSize(10)
        .fontColor('#6fb8ae')
        .margin({ top: 8 })
    }
    .width('100%')
    .padding(12)
    .backgroundColor(COLOR_SELECT_CARD)
    .borderRadius(14)
    .border({ width: { left: 4 }, color: COLOR_SELECT_ACCENT })
  }

  @Builder
  FilterChip(label: string, kind: string) {
    Text(label)
      .fontSize(10)
      .fontColor(this.filterKind === kind ? '#2a0b18' : '#f7b7d2')
      .height(26)
      .padding({ left: 8, right: 8 })
      .backgroundColor(this.filterKind === kind ? COLOR_FILTER_ACCENT : '#4a1a30')
      .borderRadius(6)
      .onClick(() => {
        this.filterKind = kind;
      })
  }

  @Builder
  FilterParam(label: string, name: string, min: number, max: number, step: number) {
    Row({ space: 8 }) {
      Text(label)
        .fontSize(11)
        .fontColor('#f0a8c6')
        .width(40)
      Slider({ value: this.filterParamValue(name), min: min, max: max, step: step })
        .layoutWeight(1)
        .onChange((value: number, _mode: SliderChangeMode) => {
          this.setFilterParam(name, value);
        })
      Text(this.filterParamText(name))
        .fontSize(11)
        .fontColor('#f0a8c6')
        .width(46)
    }
    .width('100%')
    .margin({ top: 6 })
  }

  @Builder
  FilterCard() {
    Column() {
      Row({ space: 8 }) {
        Column()
          .width(4)
          .height(18)
          .backgroundColor(COLOR_FILTER_ACCENT)
          .borderRadius(2)
        Text('滤镜')
          .fontSize(14)
          .fontWeight(FontWeight.Bold)
          .fontColor('#ffe4f0')
        Text(this.filterLabel())
          .fontSize(10)
          .fontColor('#c98aa8')
          .layoutWeight(1)
      }
      .width('100%')
      .margin({ bottom: 10 })

      Row({ space: 6 }) {
        this.FilterChip('模糊', 'gaussianBlur')
        this.FilterChip('动感', 'motionBlur')
        this.FilterChip('锐化', 'smartSharpen')
        this.FilterChip('杂色', 'addNoise')
        this.FilterChip('马赛克', 'mosaic')
      }
      .width('100%')

      Row({ space: 6 }) {
        this.FilterChip('浮雕', 'emboss')
        this.FilterChip('扭曲', 'twirl')
        this.FilterChip('反相', 'invert')
        this.FilterChip('去色', 'desaturate')
        this.FilterChip('查找边缘', 'findEdges')
      }
      .width('100%')
      .margin({ top: 6 })

      if (this.filterKind === 'gaussianBlur') {
        this.FilterParam('半径', 'blurRadius', 0.1, 50, 0.1)
      } else if (this.filterKind === 'motionBlur') {
        this.FilterParam('角度', 'motionAngle', -180, 180, 1)
        this.FilterParam('距离', 'motionDistance', 1, 200, 1)
      } else if (this.filterKind === 'smartSharpen') {
        this.FilterParam('数量', 'sharpenAmount', 1, 300, 1)
        this.FilterParam('半径', 'sharpenRadius', 0.1, 20, 0.1)
      } else if (this.filterKind === 'addNoise') {
        this.FilterParam('数量', 'noiseAmount', 0, 100, 1)
      } else if (this.filterKind === 'mosaic') {
        this.FilterParam('单元格', 'cellSize', 2, 100, 1)
      } else if (this.filterKind === 'emboss') {
        this.FilterParam('角度', 'embossAngle', -180, 180, 1)
        this.FilterParam('高度', 'embossHeight', 1, 20, 1)
        this.FilterParam('数量', 'embossAmount', 1, 500, 1)
      } else if (this.filterKind === 'twirl') {
        this.FilterParam('角度', 'twirlAngle', -999, 999, 1)
      } else {
        Text('该滤镜没有可调参数')
          .fontSize(10)
          .fontColor('#a97189')
          .margin({ top: 8 })
      }

      Button('应用滤镜')
        .fontSize(13)
        .height(34)
        .width('100%')
        .backgroundColor(this.canFilter() ? COLOR_FILTER_ACCENT : '#4a1a30')
        .fontColor(this.canFilter() ? '#2a0b18' : '#9d6b80')
        .borderRadius(9)
        .enabled(this.canFilter())
        .margin({ top: 10 })
        .onClick(() => {
          this.applyFilter();
        })

      Text(this.selectedLayerId === 0 ? '请先在图层列表选择一个像素图层' : '破坏性滤镜，可用撤销恢复；有选区时只作用于选区内')
        .fontSize(10)
        .fontColor('#c98aa8')
        .margin({ top: 8 })
    }
    .width('100%')
    .padding(12)
    .backgroundColor(COLOR_FILTER_CARD)
    .borderRadius(14)
    .border({ width: { left: 4 }, color: COLOR_FILTER_ACCENT })
  }

  @Builder
  ShapeChip(label: string, kind: string) {
    Text(label)
      .fontSize(11)
      .fontColor(this.tool === 'shape' && this.shapeKind === kind ? '#eef0ff' : '#b8bcff')
      .height(28)
      .padding({ left: 12, right: 12 })
      .backgroundColor(this.tool === 'shape' && this.shapeKind === kind ? COLOR_SHAPE_ACCENT : '#2a2a5c')
      .borderRadius(7)
      .onClick(() => {
        this.shapeKind = kind;
        this.selectTool('shape');
      })
  }

  @Builder
  ShapeColorDot(color: string, target: string) {
    Column()
      .width(22)
      .height(22)
      .backgroundColor(color)
      .borderRadius(11)
      .border({
        width: (target === 'fill' ? this.shapeFillColor : this.shapeStrokeColor) === color ? 2 : 0,
        color: '#c7d2fe'
      })
      .onClick(() => {
        if (target === 'fill') {
          this.shapeFillColor = color;
        } else {
          this.shapeStrokeColor = color;
        }
      })
  }

  @Builder
  ShapeCard() {
    Column() {
      Row({ space: 8 }) {
        Column()
          .width(4)
          .height(18)
          .backgroundColor(COLOR_SHAPE_ACCENT)
          .borderRadius(2)
        Text('形状')
          .fontSize(14)
          .fontWeight(FontWeight.Bold)
          .fontColor('#eef0ff')
        Text(this.shapeLabel())
          .fontSize(10)
          .fontColor('#9da4e8')
          .layoutWeight(1)
      }
      .width('100%')
      .margin({ bottom: 10 })

      Row({ space: 6 }) {
        this.ShapeChip('矩形', 'rect')
        this.ShapeChip('椭圆', 'ellipse')
        this.ShapeChip('直线', 'line')
      }
      .width('100%')

      Row({ space: 8 }) {
        Text('填充')
          .fontSize(11)
          .fontColor('#b8bcff')
          .width(40)
        ForEach(BRUSH_COLORS, (color: string) => {
          this.ShapeColorDot(color, 'fill')
        }, (color: string) => 'fill-' + color)
      }
      .width('100%')
      .margin({ top: 8 })

      Row({ space: 8 }) {
        Text('透明度')
          .fontSize(11)
          .fontColor('#b8bcff')
          .width(46)
        Slider({ value: this.shapeFillOpacity, min: 0, max: 100, step: 1 })
          .layoutWeight(1)
          .onChange((value: number, _mode: SliderChangeMode) => {
            this.shapeFillOpacity = value;
          })
        Text(Math.round(this.shapeFillOpacity) + '%')
          .fontSize(11)
          .fontColor('#b8bcff')
          .width(38)
      }
      .width('100%')

      Row({ space: 8 }) {
        Text('描边')
          .fontSize(11)
          .fontColor('#b8bcff')
          .width(40)
        ForEach(BRUSH_COLORS, (color: string) => {
          this.ShapeColorDot(color, 'stroke')
        }, (color: string) => 'stroke-' + color)
      }
      .width('100%')
      .margin({ top: 4 })

      Row({ space: 8 }) {
        Text('描边宽')
          .fontSize(11)
          .fontColor('#b8bcff')
          .width(46)
        Slider({ value: this.shapeStrokeWidth, min: 0, max: 50, step: 1 })
          .layoutWeight(1)
          .onChange((value: number, _mode: SliderChangeMode) => {
            this.shapeStrokeWidth = value;
          })
        Text(Math.round(this.shapeStrokeWidth) + 'px')
          .fontSize(11)
          .fontColor('#b8bcff')
          .width(38)
      }
      .width('100%')

      if (this.shapeKind === 'rect') {
        Row({ space: 8 }) {
          Text('圆角')
            .fontSize(11)
            .fontColor('#b8bcff')
            .width(40)
          Slider({ value: this.shapeRadius, min: 0, max: 200, step: 1 })
            .layoutWeight(1)
            .onChange((value: number, _mode: SliderChangeMode) => {
              this.shapeRadius = value;
            })
          Text(Math.round(this.shapeRadius) + 'px')
            .fontSize(11)
            .fontColor('#b8bcff')
            .width(38)
        }
        .width('100%')
      }

      if (this.shapeKind === 'line') {
        Row({ space: 8 }) {
          Text('线宽')
            .fontSize(11)
            .fontColor('#b8bcff')
            .width(40)
          Slider({ value: this.shapeLineWidth, min: 1, max: 100, step: 1 })
            .layoutWeight(1)
            .onChange((value: number, _mode: SliderChangeMode) => {
              this.shapeLineWidth = value;
            })
          Text(Math.round(this.shapeLineWidth) + 'px')
            .fontSize(11)
            .fontColor('#b8bcff')
            .width(38)
        }
        .width('100%')
      }

      Text(this.tool === 'shape' ? '在画布拖拽即可创建形状图层' : '选择上方形状后，在画布拖拽创建')
        .fontSize(10)
        .fontColor('#8f97dd')
        .margin({ top: 8 })
    }
    .width('100%')
    .padding(12)
    .backgroundColor(COLOR_SHAPE_CARD)
    .borderRadius(14)
    .border({ width: { left: 4 }, color: COLOR_SHAPE_ACCENT })
  }

  @Builder
  TypeCard() {
    Column() {
      Row({ space: 8 }) {
        Column()
          .width(4)
          .height(18)
          .backgroundColor(COLOR_TYPE_ACCENT)
          .borderRadius(2)
        Text('文字')
          .fontSize(14)
          .fontWeight(FontWeight.Bold)
          .fontColor('#efffe0')
        Text(this.textFontReady ? '系统字体' : '内置字体')
          .fontSize(10)
          .fontColor('#a9c98a')
          .layoutWeight(1)
      }
      .width('100%')
      .margin({ bottom: 10 })

      TextInput({ placeholder: '输入文字', text: this.textContent })
        .fontSize(13)
        .fontColor('#f4ffe8')
        .placeholderColor('#7e9a63')
        .backgroundColor('#243318')
        .borderRadius(8)
        .height(38)
        .onChange((value: string) => {
          this.textContent = value;
        })

      Row({ space: 8 }) {
        Text('字号')
          .fontSize(11)
          .fontColor('#c6e3a3')
          .width(40)
        Slider({ value: this.textSize, min: 8, max: 400, step: 1 })
          .layoutWeight(1)
          .onChange((value: number, _mode: SliderChangeMode) => {
            this.textSize = value;
          })
        Text(Math.round(this.textSize) + 'pt')
          .fontSize(11)
          .fontColor('#c6e3a3')
          .width(40)
      }
      .width('100%')
      .margin({ top: 8 })

      Row({ space: 8 }) {
        Text('颜色')
          .fontSize(11)
          .fontColor('#c6e3a3')
          .width(40)
        ForEach(BRUSH_COLORS, (color: string) => {
          Column()
            .width(22)
            .height(22)
            .backgroundColor(color)
            .borderRadius(11)
            .border({ width: this.textColor === color ? 2 : 0, color: '#d9f99d' })
            .onClick(() => {
              this.textColor = color;
            })
        }, (color: string) => 'text-' + color)
      }
      .width('100%')
      .margin({ top: 4 })

      Text(this.tool === 'text' ? '点击画布放置文字（点击点为基线起点）' : '点击下方按钮启用文字工具')
        .fontSize(10)
        .fontColor('#8fb26b')
        .margin({ top: 8 })

      Button('文字工具')
        .fontSize(13)
        .height(34)
        .width('100%')
        .backgroundColor(this.tool === 'text' ? COLOR_TYPE_ACCENT : '#2f3d12')
        .fontColor(this.tool === 'text' ? '#121a08' : '#a9c98a')
        .borderRadius(9)
        .margin({ top: 8 })
        .onClick(() => {
          this.selectTool('text');
        })
    }
    .width('100%')
    .padding(12)
    .backgroundColor(COLOR_TYPE_CARD)
    .borderRadius(14)
    .border({ width: { left: 4 }, color: COLOR_TYPE_ACCENT })
  }

  @Builder
  AdjustmentCard() {
    Column() {
      Row({ space: 8 }) {
        Column()
          .width(4)
          .height(18)
          .backgroundColor(COLOR_ADJUST_ACCENT)
          .borderRadius(2)
        Text('调整')
          .fontSize(14)
          .fontWeight(FontWeight.Bold)
          .fontColor('#eee6ff')
        Text(this.adjust.type.length > 0 ? '当前图层' : '新建调整图层')
          .fontSize(10)
          .fontColor('#8d7bb5')
          .layoutWeight(1)
      }
      .width('100%')
      .margin({ bottom: 10 })

      if (this.adjust.type === 'brightnessContrast') {
        Row({ space: 8 }) {
          Text('亮度').fontSize(11).fontColor('#b9a9e0').width(52)
          Slider({ value: this.adjust.brightness, min: -100, max: 100, step: 1 })
            .layoutWeight(1)
            .onChange((value: number, mode: SliderChangeMode) => {
              this.adjust = this.adjustWith({ brightness: value });
              if (mode === SliderChangeMode.End || mode === SliderChangeMode.Click) {
                this.commitBrightnessContrast();
              }
            })
          Text(Math.round(this.adjust.brightness).toString()).fontSize(11).fontColor('#b9a9e0').width(34)
        }.width('100%')
        Row({ space: 8 }) {
          Text('对比度').fontSize(11).fontColor('#b9a9e0').width(52)
          Slider({ value: this.adjust.contrast, min: -100, max: 100, step: 1 })
            .layoutWeight(1)
            .onChange((value: number, mode: SliderChangeMode) => {
              this.adjust = this.adjustWith({ contrast: value });
              if (mode === SliderChangeMode.End || mode === SliderChangeMode.Click) {
                this.commitBrightnessContrast();
              }
            })
          Text(Math.round(this.adjust.contrast).toString()).fontSize(11).fontColor('#b9a9e0').width(34)
        }.width('100%')
      } else if (this.adjust.type === 'exposure') {
        Row({ space: 8 }) {
          Text('曝光').fontSize(11).fontColor('#b9a9e0').width(52)
          Slider({ value: this.adjust.exposure, min: -5, max: 5, step: 0.1 })
            .layoutWeight(1)
            .onChange((value: number, mode: SliderChangeMode) => {
              this.adjust = this.adjustWith({ exposure: value });
              if (mode === SliderChangeMode.End || mode === SliderChangeMode.Click) {
                this.commitExposure();
              }
            })
          Text(this.adjust.exposure.toFixed(1)).fontSize(11).fontColor('#b9a9e0').width(34)
        }.width('100%')
        Row({ space: 8 }) {
          Text('偏移').fontSize(11).fontColor('#b9a9e0').width(52)
          Slider({ value: this.adjust.offset, min: -0.5, max: 0.5, step: 0.01 })
            .layoutWeight(1)
            .onChange((value: number, mode: SliderChangeMode) => {
              this.adjust = this.adjustWith({ offset: value });
              if (mode === SliderChangeMode.End || mode === SliderChangeMode.Click) {
                this.commitExposure();
              }
            })
          Text(this.adjust.offset.toFixed(2)).fontSize(11).fontColor('#b9a9e0').width(34)
        }.width('100%')
        Row({ space: 8 }) {
          Text('伽马').fontSize(11).fontColor('#b9a9e0').width(52)
          Slider({ value: this.adjust.gamma, min: 0.2, max: 3, step: 0.01 })
            .layoutWeight(1)
            .onChange((value: number, mode: SliderChangeMode) => {
              this.adjust = this.adjustWith({ gamma: value });
              if (mode === SliderChangeMode.End || mode === SliderChangeMode.Click) {
                this.commitExposure();
              }
            })
          Text(this.adjust.gamma.toFixed(2)).fontSize(11).fontColor('#b9a9e0').width(34)
        }.width('100%')
      } else if (this.adjust.type === 'vibrance') {
        Row({ space: 8 }) {
          Text('自然饱和').fontSize(11).fontColor('#b9a9e0').width(58)
          Slider({ value: this.adjust.vibrance, min: -100, max: 100, step: 1 })
            .layoutWeight(1)
            .onChange((value: number, mode: SliderChangeMode) => {
              this.adjust = this.adjustWith({ vibrance: value });
              if (mode === SliderChangeMode.End || mode === SliderChangeMode.Click) {
                this.commitVibrance();
              }
            })
          Text(Math.round(this.adjust.vibrance).toString()).fontSize(11).fontColor('#b9a9e0').width(34)
        }.width('100%')
        Row({ space: 8 }) {
          Text('饱和度').fontSize(11).fontColor('#b9a9e0').width(58)
          Slider({ value: this.adjust.saturation, min: -100, max: 100, step: 1 })
            .layoutWeight(1)
            .onChange((value: number, mode: SliderChangeMode) => {
              this.adjust = this.adjustWith({ saturation: value });
              if (mode === SliderChangeMode.End || mode === SliderChangeMode.Click) {
                this.commitVibrance();
              }
            })
          Text(Math.round(this.adjust.saturation).toString()).fontSize(11).fontColor('#b9a9e0').width(34)
        }.width('100%')
      } else if (this.adjust.type === 'hueSaturation') {
        Row({ space: 8 }) {
          Text('色相').fontSize(11).fontColor('#b9a9e0').width(52)
          Slider({ value: this.adjust.hue, min: -180, max: 180, step: 1 })
            .layoutWeight(1)
            .onChange((value: number, mode: SliderChangeMode) => {
              this.adjust = this.adjustWith({ hue: value });
              if (mode === SliderChangeMode.End || mode === SliderChangeMode.Click) {
                this.commitHueSaturation();
              }
            })
          Text(Math.round(this.adjust.hue).toString()).fontSize(11).fontColor('#b9a9e0').width(34)
        }.width('100%')
        Row({ space: 8 }) {
          Text('饱和度').fontSize(11).fontColor('#b9a9e0').width(52)
          Slider({ value: this.adjust.saturation, min: -100, max: 100, step: 1 })
            .layoutWeight(1)
            .onChange((value: number, mode: SliderChangeMode) => {
              this.adjust = this.adjustWith({ saturation: value });
              if (mode === SliderChangeMode.End || mode === SliderChangeMode.Click) {
                this.commitHueSaturation();
              }
            })
          Text(Math.round(this.adjust.saturation).toString()).fontSize(11).fontColor('#b9a9e0').width(34)
        }.width('100%')
        Row({ space: 8 }) {
          Text('明度').fontSize(11).fontColor('#b9a9e0').width(52)
          Slider({ value: this.adjust.lightness, min: -100, max: 100, step: 1 })
            .layoutWeight(1)
            .onChange((value: number, mode: SliderChangeMode) => {
              this.adjust = this.adjustWith({ lightness: value });
              if (mode === SliderChangeMode.End || mode === SliderChangeMode.Click) {
                this.commitHueSaturation();
              }
            })
          Text(Math.round(this.adjust.lightness).toString()).fontSize(11).fontColor('#b9a9e0').width(34)
        }.width('100%')
      } else if (this.adjust.type === 'posterize') {
        Row({ space: 8 }) {
          Text('色阶').fontSize(11).fontColor('#b9a9e0').width(52)
          Slider({ value: this.adjust.levels, min: 2, max: 64, step: 1 })
            .layoutWeight(1)
            .onChange((value: number, mode: SliderChangeMode) => {
              this.adjust = this.adjustWith({ levels: value });
              if (mode === SliderChangeMode.End || mode === SliderChangeMode.Click) {
                this.commitPosterize();
              }
            })
          Text(Math.round(this.adjust.levels).toString()).fontSize(11).fontColor('#b9a9e0').width(34)
        }.width('100%')
      } else if (this.adjust.type === 'threshold') {
        Row({ space: 8 }) {
          Text('阈值').fontSize(11).fontColor('#b9a9e0').width(52)
          Slider({ value: this.adjust.level, min: 0, max: 1, step: 0.01 })
            .layoutWeight(1)
            .onChange((value: number, mode: SliderChangeMode) => {
              this.adjust = this.adjustWith({ level: value });
              if (mode === SliderChangeMode.End || mode === SliderChangeMode.Click) {
                this.commitThreshold();
              }
            })
          Text(this.adjust.level.toFixed(2)).fontSize(11).fontColor('#b9a9e0').width(34)
        }.width('100%')
      } else if (this.adjust.type === 'blackWhite' || this.adjust.type === 'invert') {
        Text('该调整已应用，无需额外参数')
          .fontSize(11)
          .fontColor('#8d7bb5')
          .margin({ top: 2, bottom: 6 })
      } else {
        Text('选择下方按钮新建调整图层，或在上方图层列表选中一个调整图层')
          .fontSize(11)
          .fontColor('#8d7bb5')
          .margin({ bottom: 8 })
      }

      Flex({ wrap: FlexWrap.Wrap }) {
        this.AdjustButton('亮度/对比', 'brightnessContrast')
        this.AdjustButton('曝光', 'exposure')
        this.AdjustButton('自然饱和', 'vibrance')
        this.AdjustButton('色相/饱和', 'hueSaturation')
        this.AdjustButton('黑白', 'blackWhite')
        this.AdjustButton('反相', 'invert')
        this.AdjustButton('色调分离', 'posterize')
        this.AdjustButton('阈值', 'threshold')
      }
      .width('100%')
      .margin({ top: 8 })
    }
    .width('100%')
    .padding(12)
    .backgroundColor(COLOR_ADJUST_CARD)
    .borderRadius(14)
    .border({ width: { left: 4 }, color: COLOR_ADJUST_ACCENT })
  }

  @Builder
  AdjustButton(label: string, kind: string) {
    Button(label)
      .fontSize(11)
      .height(30)
      .fontColor('#e4d9ff')
      .backgroundColor('#3a3050')
      .borderRadius(8)
      .margin({ right: 6, bottom: 6 })
      .padding({ left: 10, right: 10 })
      .onClick(() => {
        this.addAdjustment(kind);
      })
  }

  @Builder
  ExportCard() {
    Column() {
      Row({ space: 8 }) {
        Column()
          .width(4)
          .height(18)
          .backgroundColor(COLOR_EXPORT_ACCENT)
          .borderRadius(2)
        Text('导出')
          .fontSize(14)
          .fontWeight(FontWeight.Bold)
          .fontColor('#d8ffe9')
        Text(this.imageSizeLabel())
          .fontSize(10)
          .fontColor('#6f9e86')
          .layoutWeight(1)
      }
      .width('100%')
      .margin({ bottom: 10 })

      Row({ space: 8 }) {
        ForEach(['png', 'jpeg', 'webp'], (format: string) => {
          Text(this.formatLabel(format))
            .fontSize(11)
            .fontColor(this.exportFormat === format ? '#0d1a14' : '#9fd8bb')
            .height(28)
            .padding({ left: 12, right: 12 })
            .backgroundColor(this.exportFormat === format ? COLOR_EXPORT_ACCENT : '#22352b')
            .borderRadius(7)
            .onClick(() => {
              this.exportFormat = format;
            })
        }, (format: string) => format)
        Blank()
        Button('导出文件')
          .fontSize(12)
          .height(30)
          .backgroundColor(COLOR_EXPORT_ACCENT)
          .fontColor('#0d1a14')
          .borderRadius(8)
          .enabled(this.docId !== 0)
          .onClick(() => {
            this.exportImage();
          })
      }
      .width('100%')

      if (this.exportFormat === 'jpeg' || this.exportFormat === 'webp') {
        Row({ space: 8 }) {
          Text('质量').fontSize(11).fontColor('#9fd8bb').width(40)
          Slider({ value: this.exportQuality, min: 1, max: 100, step: 1 })
            .layoutWeight(1)
            .onChange((value: number, _mode: SliderChangeMode) => {
              this.exportQuality = value;
            })
          Text(this.exportQuality.toString()).fontSize(11).fontColor('#9fd8bb').width(30)
        }
        .width('100%')
        .margin({ top: 8 })
      }
    }
    .width('100%')
    .padding(12)
    .backgroundColor(COLOR_EXPORT_CARD)
    .borderRadius(14)
    .border({ width: { left: 4 }, color: COLOR_EXPORT_ACCENT })
  }

  @Builder
  Panel() {
    Scroll() {
      Column({ space: 12 }) {
        this.LayersCard()
        this.BrushCard()
        this.SelectCard()
        this.FilterCard()
        this.ShapeCard()
        this.TypeCard()
        this.AdjustmentCard()
        this.ExportCard()
      }
      .width('100%')
      .padding({ left: 12, right: 12, top: 12, bottom: 12 })
    }
    .width('100%')
    .height('100%')
    .scrollBar(BarState.Auto)
    .backgroundColor('#111318')
  }

  @Builder
  StatusBar() {
    Row() {
      Text(this.status)
        .fontSize(11)
        .fontColor(COLOR_MUTED)
        .maxLines(1)
        .textOverflow({ overflow: TextOverflow.Ellipsis })
        .layoutWeight(1)
      if (this.busy) {
        LoadingProgress()
          .width(16)
          .height(16)
          .color('#4aa3ff')
      }
    }
    .width('100%')
    .height(28)
    .padding({ left: 16, right: 16 })
    .backgroundColor(COLOR_BAR)
  }

  build() {
    Column() {
      this.TopBar()
      if (this.isWide) {
        Row() {
          Column() {
            this.CanvasPane()
          }
          .layoutWeight(1)
          .height('100%')
          Column() {
            this.Panel()
          }
          .width(350)
          .height('100%')
          .border({ width: { left: 1 }, color: '#232830' })
        }
        .width('100%')
        .layoutWeight(1)
      } else {
        Column() {
          this.CanvasPane()
        }
        .width('100%')
        .layoutWeight(1)
        Column() {
          this.Panel()
        }
        .width('100%')
        .height(330)
        .border({ width: { top: 1 }, color: '#232830' })
      }
      this.StatusBar()
    }
    .width('100%')
    .height('100%')
    .backgroundColor(COLOR_ROOT)
    .onAreaChange((oldValue: Area, newValue: Area) => {
      this.isWide = Number(newValue.width) > 820;
    })
  }
}
