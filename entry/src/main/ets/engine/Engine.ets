import native from 'libentry.so';
import { hilog } from '@kit.PerformanceAnalysisKit';

// 与 Index.ets 保持同值，保证 `hdc shell hilog | findstr PERF` 能一并抓到两侧日志。
const PERF_DOMAIN: number = 0x0A01;
const PERF_TAG: string = 'PERF';

export interface LayerInfo {
  id: number;
  name: string;
  kind: string;
  visible: boolean;
  opacity: number;
  fillOpacity: number;
  blend: string;
  clipped: boolean;
  depth: number;
  isGroup: boolean;
  /** 组取原生 `Group.expanded`；非组恒为 true（保证 UI 统一消费） */
  expanded: boolean;
  children: LayerInfo[];
}

export interface DocumentInfo {
  name: string;
  width: number;
  height: number;
  mode: string;
  depth: string;
  resolution: number;
  layers: LayerInfo[];
  canUndo: boolean;
  canRedo: boolean;
}

export interface OpenOutcome {
  id: number;
  warnings: string[];
  info: DocumentInfo;
}

export interface AdjustmentInfo {
  type: string;
  brightness: number;
  contrast: number;
  exposure: number;
  offset: number;
  gamma: number;
  vibrance: number;
  saturation: number;
  hue: number;
  lightness: number;
  levels: number;
  level: number;
  colorize: boolean;
}

/** 渐变色标（`C-EFFECTS-API@2`）：`p` 为 0..1 位置，`color` 为 BGR 顺序 `[b, g, r, a]`。 */
export interface FxGradientStop {
  p: number;
  color: number[];
}

/** 渐变叠加的几何与色标（`C-EFFECTS-API@2` 的 `gradient` 字段）。 */
export interface FxGradientInfo {
  stops: FxGradientStop[];
  style: string;
  angle: number;
  scale: number;
  reverse: boolean;
  align: boolean;
}

export interface LayerEffectInfo {
  index: number;
  kind: string;
  label: string;
  enabled: boolean;
  blend: string;
  opacity: number;
  color: number[];
  angle: number;
  distance: number;
  spread: number;
  size: number;
  position: string;
  technique: string;
  knocksOut: boolean;
  antiAlias: boolean;
  // C-EFFECTS-API@2 新增字段：全部可选，旧引擎响应缺字段时由 normalizeLayerEffect 补齐。
  /** gradientOverlay 抖动。 */
  dither?: boolean;
  /** gradientOverlay 的色标与几何。 */
  gradient?: FxGradientInfo;
  /** innerGlow 的源：edge / center。 */
  source?: string;
  /** bevelEmboss 的样式（渐变样式在 gradient.style 内）。 */
  style?: string;
  /** bevelEmboss 的技术：smooth / chiselHard / chiselSoft。 */
  bevelTechnique?: string;
  /** bevelEmboss 深度，Rust 原生 0..=10（1 = 100%）。 */
  depth?: number;
  /** bevelEmboss 方向：true = 上。 */
  up?: boolean;
  /** bevelEmboss 柔化 px。 */
  soften?: number;
  /** bevelEmboss 高度角（度）。 */
  altitude?: number;
  /** bevelEmboss 高光色（BGR）。 */
  highlightColor?: number[];
  /** bevelEmboss 高光不透明度 0..=1。 */
  highlightOpacity?: number;
  /** bevelEmboss 阴影色（BGR）。 */
  shadowColor?: number[];
  /** bevelEmboss 阴影不透明度 0..=1。 */
  shadowOpacity?: number;
}

export interface LayerEffectsInfo {
  enabled: boolean;
  items: LayerEffectInfo[];
}

/**
 * 单个文字/形状图层的可编辑参数（`C-EDIT-API@1 §B`）。
 *
 * 扁平、全字段必填；不适用的字段返回零值。颜色数组为 BGR + a `[b, g, r, a]`，
 * 与 `hexToRgba` 同序，往返不做任何通道交换。
 */
export interface LayerEditInfo {
  /** "Type" | "Shape" */
  kind: string;
  text: string;
  fontFamily: string;
  size: number;
  color: number[];
  x: number;
  y: number;
  w: number;
  h: number;
  /** "rect" | "ellipse" | "polygon" | "line" | "arrowLine" | "path"（Type 层为 ""） */
  shapeKind: string;
  fromX: number;
  fromY: number;
  toX: number;
  toY: number;
  weight: number;
  radius: number;
  fillColor: number[];
  fillEditable: boolean;
  strokeColor: number[];
  strokeEditable: boolean;
  strokeWidth: number;
}

/**
 * 图层蒙版状态（`C-MASK-API@1 §B`）。全字段必填，`present=false` 时其余为契约默认值。
 */
export interface LayerMaskInfo {
  /** 该层是否有蒙版 */
  present: boolean;
  /** 停用后渲染等同于无蒙版；`present=false` 时为 true */
  enabled: boolean;
  /** 蒙版是否随图层变换移动（本轮恒为 true） */
  linked: boolean;
  /** 0..1；`present=false` 时为 1 */
  density: number;
  /** 羽化像素半径 0..1000；`present=false` 时为 0 */
  feather: number;
}

/**
 * 参考线与标尺原点（`C-GUIDES-API@1`）。
 *
 * `horizontal` / `vertical` 是文档像素坐标（引擎侧已排序去重），`origin` 是标尺零点
 * `[x, y]`（默认 `[0, 0]`）。
 */
export interface GuidesInfo {
  horizontal: number[];
  vertical: number[];
  origin: number[];
}

/**
 * 图层自由变换结果（`C-XFORM-API@1 §2.4`）。
 *
 * `bounds` 是变换后的内容包围盒（文档坐标，整数，`[x0, y0, x1, y1]`）；`scaleX` / `scaleY` /
 * `angle` 回显本次提交的缩放倍率与旋转增量（角度）。
 */
export interface LayerTransformInfo {
  bounds: number[];
  scaleX: number;
  scaleY: number;
  angle: number;
}

interface OpenPayload {
  id: number;
  warnings: string[];
  info: DocumentInfo;
}

export interface HistoryLabels {
  undo: string[];
  redo: string[];
}

interface HistoryResponse {
  ok: boolean;
  error: string;
  data: HistoryLabels;
}

export interface HistoryJump {
  steps: number;
  info: DocumentInfo;
}

interface HistoryJumpResponse {
  ok: boolean;
  error: string;
  data: HistoryJump;
}

interface ApiResponse {
  ok: boolean;
  error: string;
  data: OpenPayload;
}

interface InfoResponse {
  ok: boolean;
  error: string;
  data: DocumentInfo;
}

interface AdjustmentResponse {
  ok: boolean;
  error: string;
  data: AdjustmentInfo;
}

export interface PaintOutcome {
  layerId: number;
}

interface PaintResponse {
  ok: boolean;
  error: string;
  data: PaintOutcome;
}

export interface SelectionInfo {
  active: boolean;
  x0: number;
  y0: number;
  x1: number;
  y1: number;
}

interface SelectionResponse {
  ok: boolean;
  error: string;
  data: SelectionInfo;
}

export interface ShapeOutcome {
  layerId: number;
  info: DocumentInfo;
}

interface ShapeResponse {
  ok: boolean;
  error: string;
  data: ShapeOutcome;
}

export interface LayerOutcome {
  layerId: number;
  info: DocumentInfo;
}

interface LayerDuplicateResponse {
  ok: boolean;
  error: string;
  data: LayerOutcome;
}

export interface LayerBatchOutcome {
  layerIds: number[];
  info: DocumentInfo;
}

interface LayerBatchResponse {
  ok: boolean;
  error: string;
  data: LayerBatchOutcome;
}

export interface TextOutcome {
  layerId: number;
  info: DocumentInfo;
}

interface TextResponse {
  ok: boolean;
  error: string;
  data: TextOutcome;
}

interface FontFamilies {
  families: string[];
}

interface FontResponse {
  ok: boolean;
  error: string;
  data: FontFamilies;
}

interface EffectsResponse {
  ok: boolean;
  error: string;
  data: LayerEffectsInfo;
}

interface LayerEditResponse {
  ok: boolean;
  error: string;
  data: LayerEditInfo;
}

interface LayerMaskResponse {
  ok: boolean;
  error: string;
  data: LayerMaskInfo;
}

/** 成组结果：新组 id 与其完整图层信息（含 children）。 */
export interface LayerGroupOutcome {
  groupId: number;
  group: LayerInfo;
}

interface LayerGroupResponse {
  ok: boolean;
  error: string;
  data: LayerGroupOutcome;
}

interface UngroupData {
  /** 提到父级的子层数 */
  count: number;
}

interface UngroupResponse {
  ok: boolean;
  error: string;
  data: UngroupData;
}

interface ExpandedData {
  expanded: boolean;
}

interface ExpandedResponse {
  ok: boolean;
  error: string;
  data: ExpandedData;
}

interface ClippedData {
  clipped: boolean;
}

interface ClippedResponse {
  ok: boolean;
  error: string;
  data: ClippedData;
}

interface DeleteManyData {
  /** 实际删除的图层数（已剔除「祖先也被选中」的后代，随父组消失的子层不计入） */
  deleted: number;
}

interface DeleteManyResponse {
  ok: boolean;
  error: string;
  data: DeleteManyData;
}

interface RevisionData {
  revision: number;
}

interface RevisionResponse {
  ok: boolean;
  error: string;
  data: RevisionData;
}

interface GuidesResponse {
  ok: boolean;
  error: string;
  data: GuidesInfo;
}

/** 原生 `pc_layer_transform` 响应的顶层 `perf` 段（性能取证用）。 */
interface LayerTransformPerf {
  totalMs: number;
  resampleMs: number;
  readMs?: number;
  fillMs?: number;
  writeMs?: number;
  pruneMs?: number;
  resamplePixels: number;
  resampleCalls: number;
  threads: number;
}

interface LayerTransformResponse {
  ok: boolean;
  error: string;
  data: LayerTransformInfo;
  perf?: LayerTransformPerf;
}

export function emptyAdjustment(): AdjustmentInfo {
  return {
    type: '',
    brightness: 0,
    contrast: 0,
    exposure: 0,
    offset: 0,
    gamma: 1,
    vibrance: 0,
    saturation: 0,
    hue: 0,
    lightness: 0,
    levels: 4,
    level: 0.5,
    colorize: false
  };
}

function normalizeLayer(raw: LayerInfo): LayerInfo {
  return {
    id: raw.id,
    name: raw.name,
    kind: raw.kind,
    visible: raw.visible,
    opacity: raw.opacity,
    fillOpacity: raw.fillOpacity ?? raw.opacity,
    blend: raw.blend ?? 'Normal',
    clipped: raw.clipped,
    depth: raw.depth,
    isGroup: raw.isGroup,
    expanded: raw.expanded ?? true,
    children: (raw.children ?? []).map((child: LayerInfo) => normalizeLayer(child))
  };
}

function normalizeInfo(raw: DocumentInfo): DocumentInfo {
  return {
    name: raw.name,
    width: raw.width,
    height: raw.height,
    mode: raw.mode,
    depth: raw.depth,
    resolution: raw.resolution,
    layers: (raw.layers ?? []).map((layer: LayerInfo) => normalizeLayer(layer)),
    canUndo: raw.canUndo,
    canRedo: raw.canRedo
  };
}

/** 归一化单个渐变色标：只补缺省，不改数值、不交换颜色通道。 */
function normalizeGradientStop(raw: FxGradientStop): FxGradientStop {
  return { p: raw.p ?? 0, color: raw.color ?? [] };
}

/** 归一化渐变信息；旧引擎不返回 `gradient` 时回落到契约缺省值。 */
function normalizeGradient(raw: FxGradientInfo | undefined): FxGradientInfo {
  return {
    stops: (raw?.stops ?? []).map((stop: FxGradientStop) => normalizeGradientStop(stop)),
    style: raw?.style ?? 'linear',
    angle: raw?.angle ?? 90,
    scale: raw?.scale ?? 1,
    reverse: raw?.reverse ?? false,
    align: raw?.align ?? true
  };
}

function normalizeLayerEffect(raw: LayerEffectInfo): LayerEffectInfo {
  return {
    index: raw.index ?? 0,
    kind: raw.kind ?? 'unsupported',
    label: raw.label ?? '',
    enabled: raw.enabled ?? true,
    blend: raw.blend ?? 'Normal',
    opacity: raw.opacity ?? 1,
    color: raw.color ?? [],
    angle: raw.angle ?? 0,
    distance: raw.distance ?? 0,
    spread: raw.spread ?? 0,
    size: raw.size ?? 0,
    position: raw.position ?? 'outside',
    technique: raw.technique ?? 'softer',
    knocksOut: raw.knocksOut ?? false,
    antiAlias: raw.antiAlias ?? false,
    dither: raw.dither ?? false,
    gradient: normalizeGradient(raw.gradient),
    source: raw.source ?? 'edge',
    style: raw.style ?? 'innerBevel',
    // 契约存在一处自相矛盾：响应 JSON 示例给的是顶层 `technique: "smooth"`，而本接口字段名为
    // `bevelTechnique`。这里以 `technique` 作为回退来源，保证 UI 能读到引擎真实值。
    bevelTechnique: raw.bevelTechnique ?? (raw.kind === 'bevelEmboss' && raw.technique ? raw.technique : 'smooth'),
    depth: raw.depth ?? 1,
    up: raw.up ?? true,
    soften: raw.soften ?? 0,
    altitude: raw.altitude ?? 30,
    highlightColor: raw.highlightColor ?? [1, 1, 1, 1],
    highlightOpacity: raw.highlightOpacity ?? 0.75,
    shadowColor: raw.shadowColor ?? [0, 0, 0, 1],
    shadowOpacity: raw.shadowOpacity ?? 0.75
  };
}

function normalizeLayerEffects(raw: LayerEffectsInfo): LayerEffectsInfo {
  const items: LayerEffectInfo[] = (raw.items ?? []).map((item: LayerEffectInfo) => normalizeLayerEffect(item));
  return { enabled: raw.enabled ?? true, items: items };
}

/** 归一化图层编辑读数：只补缺省，不改数值、不交换颜色通道。 */
function normalizeLayerEdit(raw: LayerEditInfo): LayerEditInfo {
  return {
    kind: raw.kind ?? '',
    text: raw.text ?? '',
    fontFamily: raw.fontFamily ?? '',
    size: raw.size ?? 0,
    color: raw.color ?? [],
    x: raw.x ?? 0,
    y: raw.y ?? 0,
    w: raw.w ?? 0,
    h: raw.h ?? 0,
    shapeKind: raw.shapeKind ?? '',
    fromX: raw.fromX ?? 0,
    fromY: raw.fromY ?? 0,
    toX: raw.toX ?? 0,
    toY: raw.toY ?? 0,
    weight: raw.weight ?? 0,
    radius: raw.radius ?? 0,
    fillColor: raw.fillColor ?? [],
    fillEditable: raw.fillEditable ?? false,
    strokeColor: raw.strokeColor ?? [],
    strokeEditable: raw.strokeEditable ?? false,
    strokeWidth: raw.strokeWidth ?? 0
  };
}

/** 归一化蒙版读数：只补缺省，不改数值（`C-MASK-API@1 §A.4`）。 */
function normalizeLayerMask(raw: LayerMaskInfo | undefined): LayerMaskInfo {
  return {
    present: raw?.present ?? false,
    enabled: raw?.enabled ?? true,
    linked: raw?.linked ?? true,
    density: raw?.density ?? 1,
    feather: raw?.feather ?? 0
  };
}

/** 归一化参考线读数：缺字段补空数组 / `[0, 0]`，不改数值（`C-GUIDES-API@1`）。 */
function normalizeGuides(raw: GuidesInfo | undefined): GuidesInfo {
  return {
    horizontal: raw?.horizontal ?? [],
    vertical: raw?.vertical ?? [],
    origin: raw?.origin ?? [0, 0]
  };
}

/** 归一化变换结果：`bounds` 补空数组、数值补 0，不改数值（`C-XFORM-API@1 §2.4`）。 */
function normalizeLayerTransform(raw: LayerTransformInfo | undefined): LayerTransformInfo {
  return {
    bounds: raw?.bounds ?? [],
    scaleX: raw?.scaleX ?? 0,
    scaleY: raw?.scaleY ?? 0,
    angle: raw?.angle ?? 0
  };
}

export class Engine {
  static open(name: string, data: ArrayBuffer): OpenOutcome {
    const parsed = JSON.parse(native.openImage(name, data)) as ApiResponse;
    if (!parsed.ok) {
      throw new Error(parsed.error);
    }
    return { id: parsed.data.id, warnings: parsed.data.warnings ?? [], info: normalizeInfo(parsed.data.info) };
  }

  static newDocument(width: number, height: number, name: string): OpenOutcome {
    const parsed = JSON.parse(native.newDocument(width, height, name)) as ApiResponse;
    if (!parsed.ok) {
      throw new Error(parsed.error);
    }
    return { id: parsed.data.id, warnings: parsed.data.warnings ?? [], info: normalizeInfo(parsed.data.info) };
  }

  static documentTurn(documentId: number, turn: number): DocumentInfo {
    const parsed = JSON.parse(native.documentTurn(documentId, turn)) as InfoResponse;
    if (!parsed.ok) {
      throw new Error(parsed.error);
    }
    return normalizeInfo(parsed.data);
  }

  static history(documentId: number): HistoryLabels {
    const parsed = JSON.parse(native.history(documentId)) as HistoryResponse;
    if (!parsed.ok) {
      throw new Error(parsed.error);
    }
    return parsed.data;
  }

  /** 一次跳转多步撤销：引擎侧只递增一次 revision，返回实际步数与最新元数据。 */
  static undoSteps(documentId: number, steps: number): HistoryJump {
    const parsed = JSON.parse(native.undoSteps(documentId, steps)) as HistoryJumpResponse;
    if (!parsed.ok) {
      throw new Error(parsed.error);
    }
    return { steps: parsed.data.steps, info: normalizeInfo(parsed.data.info) };
  }

  static redoSteps(documentId: number, steps: number): HistoryJump {
    const parsed = JSON.parse(native.redoSteps(documentId, steps)) as HistoryJumpResponse;
    if (!parsed.ok) {
      throw new Error(parsed.error);
    }
    return { steps: parsed.data.steps, info: normalizeInfo(parsed.data.info) };
  }

  static close(documentId: number): void {
    native.close(documentId);
  }

  static info(documentId: number): DocumentInfo {
    const parsed = JSON.parse(native.info(documentId)) as InfoResponse;
    if (!parsed.ok) {
      throw new Error(parsed.error);
    }
    return normalizeInfo(parsed.data);
  }

  static preview(documentId: number, maxSide: number): ArrayBuffer {
    return native.preview(documentId, maxSide);
  }

  static layerThumbnail(documentId: number, layerId: number, side: number): ArrayBuffer {
    return native.layerThumbnail(documentId, layerId, side);
  }

  static render(documentId: number, x: number, y: number, w: number, h: number): ArrayBuffer {
    return native.render(documentId, x, y, w, h);
  }

  static paintStroke(documentId: number, layerId: number, paramsJson: string, pointsJson: string): number {
    const parsed = JSON.parse(native.paintStroke(documentId, layerId, paramsJson, pointsJson)) as PaintResponse;
    if (!parsed.ok) {
      throw new Error(parsed.error);
    }
    return parsed.data.layerId;
  }

  static select(documentId: number, paramsJson: string): SelectionInfo {
    const parsed = JSON.parse(native.select(documentId, paramsJson)) as SelectionResponse;
    if (!parsed.ok) {
      throw new Error(parsed.error);
    }
    const d = parsed.data;
    return {
      active: d.active ?? false,
      x0: d.x0 ?? 0,
      y0: d.y0 ?? 0,
      x1: d.x1 ?? 0,
      y1: d.y1 ?? 0
    };
  }

  static selectionInfo(documentId: number): SelectionInfo {
    const parsed = JSON.parse(native.selectionInfo(documentId)) as SelectionResponse;
    if (!parsed.ok) {
      throw new Error(parsed.error);
    }
    const d = parsed.data;
    return {
      active: d.active ?? false,
      x0: d.x0 ?? 0,
      y0: d.y0 ?? 0,
      x1: d.x1 ?? 0,
      y1: d.y1 ?? 0
    };
  }

  static filterApply(documentId: number, layerId: number, paramsJson: string): DocumentInfo {
    const parsed = JSON.parse(native.filterApply(documentId, layerId, paramsJson)) as InfoResponse;
    if (!parsed.ok) {
      throw new Error(parsed.error);
    }
    return normalizeInfo(parsed.data);
  }

  static shapeCreate(documentId: number, paramsJson: string): ShapeOutcome {
    const parsed = JSON.parse(native.shapeCreate(documentId, paramsJson)) as ShapeResponse;
    if (!parsed.ok) {
      throw new Error(parsed.error);
    }
    const d = parsed.data;
    return { layerId: d.layerId, info: normalizeInfo(d.info) };
  }

  static textCreate(documentId: number, paramsJson: string): TextOutcome {
    const parsed = JSON.parse(native.textCreate(documentId, paramsJson)) as TextResponse;
    if (!parsed.ok) {
      throw new Error(parsed.error);
    }
    const d = parsed.data;
    return { layerId: d.layerId, info: normalizeInfo(d.info) };
  }

  static layerEditGet(documentId: number, layerId: number): LayerEditInfo {
    const parsed = JSON.parse(native.layerEditGet(documentId, layerId)) as LayerEditResponse;
    if (!parsed.ok) {
      throw new Error(parsed.error);
    }
    return normalizeLayerEdit(parsed.data);
  }

  static layerEditApply(documentId: number, layerId: number, paramsJson: string): LayerEditInfo {
    const parsed = JSON.parse(native.layerEditApply(documentId, layerId, paramsJson)) as LayerEditResponse;
    if (!parsed.ok) {
      throw new Error(parsed.error);
    }
    return normalizeLayerEdit(parsed.data);
  }

  static fontRegister(data: ArrayBuffer): string[] {
    const parsed = JSON.parse(native.fontRegister(data)) as FontResponse;
    if (!parsed.ok) {
      throw new Error(parsed.error);
    }
    return parsed.data.families ?? [];
  }

  static setLayerVisible(documentId: number, layerId: number, visible: boolean): void {
    native.setLayerVisible(documentId, layerId, visible);
  }

  static setLayerOpacity(documentId: number, layerId: number, opacity: number): void {
    native.setLayerOpacity(documentId, layerId, opacity);
  }

  static setLayerFillOpacity(documentId: number, layerId: number, opacity: number): void {
    native.setLayerFillOpacity(documentId, layerId, opacity);
  }

  static setLayerBlend(documentId: number, layerId: number, mode: string): void {
    native.setLayerBlend(documentId, layerId, mode);
  }

  static renameLayer(documentId: number, layerId: number, name: string): void {
    native.renameLayer(documentId, layerId, name);
  }

  static moveLayer(documentId: number, layerId: number, delta: number): DocumentInfo {
    const parsed = JSON.parse(native.moveLayer(documentId, layerId, delta)) as InfoResponse;
    if (!parsed.ok) {
      throw new Error(parsed.error);
    }
    return normalizeInfo(parsed.data);
  }

  static createLayer(documentId: number, aboveLayerId: number, name: string): LayerOutcome {
    const parsed = JSON.parse(native.createLayer(documentId, aboveLayerId, name)) as LayerDuplicateResponse;
    if (!parsed.ok) {
      throw new Error(parsed.error);
    }
    const d = parsed.data;
    return { layerId: d.layerId, info: normalizeInfo(d.info) };
  }

  static duplicateLayer(documentId: number, layerId: number): LayerOutcome {
    const parsed = JSON.parse(native.duplicateLayer(documentId, layerId)) as LayerDuplicateResponse;
    if (!parsed.ok) {
      throw new Error(parsed.error);
    }
    const d = parsed.data;
    return { layerId: d.layerId, info: normalizeInfo(d.info) };
  }

  static duplicateLayerMany(documentId: number, idsJson: string): LayerBatchOutcome {
    const parsed = JSON.parse(native.duplicateLayerMany(documentId, idsJson)) as LayerBatchResponse;
    if (!parsed.ok) {
      throw new Error(parsed.error);
    }
    const d = parsed.data;
    return { layerIds: d.layerIds ?? [], info: normalizeInfo(d.info) };
  }

  static mergeLayer(documentId: number, layerId: number): LayerOutcome {
    const parsed = JSON.parse(native.mergeLayer(documentId, layerId)) as LayerDuplicateResponse;
    if (!parsed.ok) {
      throw new Error(parsed.error);
    }
    const d = parsed.data;
    return { layerId: d.layerId, info: normalizeInfo(d.info) };
  }

  static deleteLayer(documentId: number, layerId: number): void {
    native.deleteLayer(documentId, layerId);
  }

  /**
   * 批量删除图层：引擎在一次 checkpoint 内删完整批，因此**一次撤销即可恢复全部**。
   * 父子同时出现在 `ids` 时，引擎会剔除「祖先也被选中」的后代，返回实际删除数。
   */
  static layerDeleteMany(documentId: number, ids: number[]): number {
    const parsed = JSON.parse(native.layerDeleteMany(documentId, JSON.stringify(ids ?? []))) as DeleteManyResponse;
    if (!parsed.ok) {
      throw new Error(parsed.error);
    }
    return parsed.data.deleted ?? 0;
  }

  static addAdjustment(documentId: number, kind: string, paramsJson: string): void {
    native.addAdjustment(documentId, kind, paramsJson);
  }

  static updateAdjustment(documentId: number, layerId: number, paramsJson: string): void {
    native.updateAdjustment(documentId, layerId, paramsJson);
  }

  static adjustment(documentId: number, layerId: number): AdjustmentInfo {
    const parsed = JSON.parse(native.getAdjustment(documentId, layerId)) as AdjustmentResponse;
    if (!parsed.ok) {
      throw new Error(parsed.error);
    }
    const d = parsed.data;
    return {
      type: d.type ?? '',
      brightness: d.brightness ?? 0,
      contrast: d.contrast ?? 0,
      exposure: d.exposure ?? 0,
      offset: d.offset ?? 0,
      gamma: d.gamma ?? 1,
      vibrance: d.vibrance ?? 0,
      saturation: d.saturation ?? 0,
      hue: d.hue ?? 0,
      lightness: d.lightness ?? 0,
      levels: d.levels ?? 4,
      level: d.level ?? 0.5,
      colorize: d.colorize ?? false
    };
  }

  static undo(documentId: number): DocumentInfo {
    const parsed = JSON.parse(native.undo(documentId)) as InfoResponse;
    if (!parsed.ok) {
      throw new Error(parsed.error);
    }
    return normalizeInfo(parsed.data);
  }

  static redo(documentId: number): DocumentInfo {
    const parsed = JSON.parse(native.redo(documentId)) as InfoResponse;
    if (!parsed.ok) {
      throw new Error(parsed.error);
    }
    return normalizeInfo(parsed.data);
  }

  static exportImage(documentId: number, format: string, quality: number): ArrayBuffer {
    return native.exportImage(documentId, format, quality);
  }

  static layerEffects(documentId: number, layerId: number): LayerEffectsInfo {
    const parsed = JSON.parse(native.layerEffects(documentId, layerId)) as EffectsResponse;
    if (!parsed.ok) {
      throw new Error(parsed.error);
    }
    return normalizeLayerEffects(parsed.data);
  }

  static updateLayerEffect(documentId: number, layerId: number, paramsJson: string): LayerEffectsInfo {
    const parsed = JSON.parse(native.updateLayerEffect(documentId, layerId, paramsJson)) as EffectsResponse;
    if (!parsed.ok) {
      throw new Error(parsed.error);
    }
    return normalizeLayerEffects(parsed.data);
  }

  // The native side returns bare RGBA8 bytes (no size header), so they go straight to PixelMap.
  static renderPixels(documentId: number, x: number, y: number, w: number, h: number): ArrayBuffer {
    return native.renderPixels(documentId, x, y, w, h);
  }

  static revision(documentId: number): number {
    const parsed = JSON.parse(native.revision(documentId)) as RevisionResponse;
    if (!parsed.ok) {
      throw new Error(parsed.error);
    }
    return parsed.data.revision ?? 0;
  }

  static layerMaskGet(documentId: number, layerId: number): LayerMaskInfo {
    const parsed = JSON.parse(native.layerMaskGet(documentId, layerId)) as LayerMaskResponse;
    if (!parsed.ok) {
      throw new Error(parsed.error);
    }
    return normalizeLayerMask(parsed.data);
  }

  static layerMaskAdd(documentId: number, layerId: number, fill: string): LayerMaskInfo {
    const parsed = JSON.parse(native.layerMaskAdd(documentId, layerId, fill)) as LayerMaskResponse;
    if (!parsed.ok) {
      throw new Error(parsed.error);
    }
    return normalizeLayerMask(parsed.data);
  }

  static layerMaskSet(documentId: number, layerId: number, paramsJson: string): LayerMaskInfo {
    const parsed = JSON.parse(native.layerMaskSet(documentId, layerId, paramsJson)) as LayerMaskResponse;
    if (!parsed.ok) {
      throw new Error(parsed.error);
    }
    return normalizeLayerMask(parsed.data);
  }

  static layerMaskFillFromSelection(documentId: number, layerId: number, mode: string): LayerMaskInfo {
    const parsed = JSON.parse(native.layerMaskFillFromSelection(documentId, layerId, mode)) as LayerMaskResponse;
    if (!parsed.ok) {
      throw new Error(parsed.error);
    }
    return normalizeLayerMask(parsed.data);
  }

  static layerMaskApply(documentId: number, layerId: number): LayerMaskInfo {
    const parsed = JSON.parse(native.layerMaskApply(documentId, layerId)) as LayerMaskResponse;
    if (!parsed.ok) {
      throw new Error(parsed.error);
    }
    return normalizeLayerMask(parsed.data);
  }

  static layerMaskRemove(documentId: number, layerId: number): LayerMaskInfo {
    const parsed = JSON.parse(native.layerMaskRemove(documentId, layerId)) as LayerMaskResponse;
    if (!parsed.ok) {
      throw new Error(parsed.error);
    }
    return normalizeLayerMask(parsed.data);
  }

  /** 把多个图层合并成一个新组；`ids` 顺序即组内子层顺序（引擎会去重保序）。 */
  static layerGroup(documentId: number, ids: number[], name: string): LayerGroupOutcome {
    const parsed = JSON.parse(native.layerGroup(documentId, JSON.stringify(ids ?? []), name)) as LayerGroupResponse;
    if (!parsed.ok) {
      throw new Error(parsed.error);
    }
    const d = parsed.data;
    return { groupId: d.groupId ?? 0, group: normalizeLayer(d.group) };
  }

  /** 解散组，返回提到父级的子层数。 */
  static layerUngroup(documentId: number, layerId: number): number {
    const parsed = JSON.parse(native.layerUngroup(documentId, layerId)) as UngroupResponse;
    if (!parsed.ok) {
      throw new Error(parsed.error);
    }
    return parsed.data.count ?? 0;
  }

  /** 设置组的展开/折叠；纯面板状态，不占撤销栈、不触发重渲染。 */
  static layerExpanded(documentId: number, layerId: number, expanded: boolean): boolean {
    const parsed = JSON.parse(native.layerExpanded(documentId, layerId, expanded)) as ExpandedResponse;
    if (!parsed.ok) {
      throw new Error(parsed.error);
    }
    return parsed.data.expanded ?? expanded;
  }

  /**
   * 设置图层的剪贴蒙版：开启时该层必须 clip 到同一父级内它下方最近的图层。
   * 同值调用是空操作（不占撤销栈）；成功后返回文档里的实际状态。
   */
  static layerClipped(documentId: number, layerId: number, clipped: boolean): boolean {
    const parsed = JSON.parse(native.layerClipped(documentId, layerId, clipped)) as ClippedResponse;
    if (!parsed.ok) {
      throw new Error(parsed.error);
    }
    return parsed.data.clipped ?? false;
  }

  /**
   * 裁剪到文档坐标矩形 `x0,y0,x1,y1`（`x1`/`y1` 独占，`C-CANVAS-API@1` 第 5 段）。
   *
   * 失败（矩形越界等）抛错；"是否真的改变"由调用方用宽高比较判定（引擎无变化时仍返回
   * `ok=true` 与当前 `DocumentInfo`）。
   */
  static documentCrop(documentId: number, x0: number, y0: number, x1: number, y1: number): DocumentInfo {
    const parsed = JSON.parse(native.documentCrop(documentId, x0, y0, x1, y1)) as InfoResponse;
    if (parsed.ok !== true) {
      throw new Error(parsed.error);
    }
    return normalizeInfo(parsed.data);
  }

  /**
   * 按目标尺寸与锚点调整画布；`anchor` ∈ 0..8（`row * 3 + col`，4 = 中心）。
   * 失败（非法尺寸/锚点、超出上限）抛错，成功返回当前 `DocumentInfo`。
   */
  static documentResizeCanvas(documentId: number, width: number, height: number, anchor: number): DocumentInfo {
    const parsed = JSON.parse(native.documentResizeCanvas(documentId, width, height, anchor)) as InfoResponse;
    if (parsed.ok !== true) {
      throw new Error(parsed.error);
    }
    return normalizeInfo(parsed.data);
  }

  /** 读取参考线与标尺原点（只读，不占撤销栈、不影响 revision）。 */
  static guidesGet(documentId: number): GuidesInfo {
    const parsed = JSON.parse(native.guidesGet(documentId)) as GuidesResponse;
    if (parsed.ok !== true) {
      throw new Error(parsed.error);
    }
    return normalizeGuides(parsed.data);
  }

  /**
   * 全量替换参考线与标尺原点。`paramsJson` 的 `horizontal` / `vertical` / `origin` 均可选，
   * 缺省键表示该轴不改；与当前状态完全一致时不写入、不占撤销栈。
   */
  static guidesSet(documentId: number, paramsJson: string): GuidesInfo {
    const parsed = JSON.parse(native.guidesSet(documentId, paramsJson)) as GuidesResponse;
    if (parsed.ok !== true) {
      throw new Error(parsed.error);
    }
    return normalizeGuides(parsed.data);
  }

  /**
   * 图层自由变换（`C-XFORM-API@1`）：`paramsJson` 的 `scaleX` / `scaleY` / `angle` / `dx` / `dy`
   * 全部可选且**相对当前状态**（倍率 / 度数增量 / 像素增量）；一次调用一次撤销点。
   *
   * 失败（非法参数、图层不存在、类型不支持、输出过大）抛错；全默认值提交是空操作（引擎不写、
   * 不占撤销栈、仍返回 `ok=true`）。
   */
  static layerTransform(documentId: number, layerId: number, paramsJson: string): LayerTransformInfo {
    const parsed = JSON.parse(native.layerTransform(documentId, layerId, paramsJson)) as LayerTransformResponse;
    if (parsed.ok !== true) {
      throw new Error(parsed.error);
    }
    // Rust 侧 eprintln! 在鸿蒙设备上不进 hilog，性能数据只能由 ArkTS 侧转发。
    const perf = parsed.perf;
    if (perf !== undefined) {
      const readMs = perf.readMs !== undefined ? perf.readMs : 0;
      const fillMs = perf.fillMs !== undefined ? perf.fillMs : 0;
      const writeMs = perf.writeMs !== undefined ? perf.writeMs : 0;
      const pruneMs = perf.pruneMs !== undefined ? perf.pruneMs : 0;
      hilog.info(PERF_DOMAIN, PERF_TAG, '%{public}s',
        'transform ms=' + perf.totalMs.toFixed(1)
        + ' resampleMs=' + perf.resampleMs.toFixed(1)
        + ' px=' + perf.resamplePixels
        + ' calls=' + perf.resampleCalls
        + ' threads=' + perf.threads
        + ' readMs=' + readMs.toFixed(1)
        + ' fillMs=' + fillMs.toFixed(1)
        + ' writeMs=' + writeMs.toFixed(1)
        + ' pruneMs=' + pruneMs.toFixed(1));
    }
    return normalizeLayerTransform(parsed.data);
  }
}

export function flattenLayers(layers: LayerInfo[], depth: number): LayerInfo[] {
  const out: LayerInfo[] = [];
  for (const layer of layers) {
    out.push({
      id: layer.id,
      name: layer.name,
      kind: layer.kind,
      visible: layer.visible,
      opacity: layer.opacity,
      fillOpacity: layer.fillOpacity,
      blend: layer.blend,
      clipped: layer.clipped,
      depth: depth,
      isGroup: layer.isGroup,
      expanded: layer.expanded,
      children: layer.children
    });
    // 组折叠时保留该组行本身，但不再展开其子层（面板与 `Group.expanded` 一致）。
    if (layer.children.length > 0 && layer.expanded !== false) {
      out.push(...flattenLayers(layer.children, depth + 1));
    }
  }
  return out;
}