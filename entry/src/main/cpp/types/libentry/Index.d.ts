export interface OpenResult {
  ok: boolean;
  data: {
    id: number;
    warnings: string[];
    info: DocumentInfo;
  };
}

export interface LayerInfo {
  id: number;
  name: string;
  kind: string;
  visible: boolean;
  opacity: number;
  clipped: boolean;
  depth: number;
  isGroup: boolean;
  /** 组取 `Group.expanded`，非组恒为 true */
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

/**
 * 单个文字/形状图层的可编辑参数（`C-EDIT-API@1 §B`）。
 *
 * 扁平、全字段必填，避免 ArkTS 严格模式下的联合类型收窄问题；不适用的字段返回零值。
 * 颜色数组一律是 BGR + a 顺序 `[b, g, r, a]`，与 `hexToRgba` 同序。
 */
export interface LayerEditInfo {
  /** "Type" | "Shape" */
  kind: string;
  /** Type：当前文本；Shape："" */
  text: string;
  /** Type：当前字体族；Shape："" */
  fontFamily: string;
  /** Type：`size_pt`；Shape：0 */
  size: number;
  /** Type：`[b, g, r, a]`；Shape：`[0, 0, 0, 0]` */
  color: number[];
  /** Type：`transform.m[4]`；Shape：几何包围盒左上角 x */
  x: number;
  /** Type：`transform.m[5]`；Shape：几何包围盒左上角 y */
  y: number;
  /** Type：0；Shape：包围盒宽 */
  w: number;
  /** Type：0；Shape：包围盒高 */
  h: number;
  /** Type：""；Shape："rect" | "ellipse" | "polygon" | "line" | "arrowLine" | "path" */
  shapeKind: string;
  /** Shape 且 live 为 Line/ArrowLine 时有效，否则 0 */
  fromX: number;
  fromY: number;
  toX: number;
  toY: number;
  /** Line/ArrowLine 的绘制粗细，否则 0 */
  weight: number;
  /** Shape 且 live 为 Rect 时取 `radii[0]`，否则 0 */
  radius: number;
  /** 纯色填充 `[b, g, r, a]`；无填充或非纯色 → `[0, 0, 0, 0]` */
  fillColor: number[];
  /** `None` 或 `Some(Fill::Solid(_))` → true；Gradient/Pattern → false */
  fillEditable: boolean;
  /** 纯色描边 `[b, g, r, a]`；无描边或非纯色 → `[0, 0, 0, 0]` */
  strokeColor: number[];
  strokeEditable: boolean;
  /** 无描边 → 0 */
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

declare const native: {
  openImage(name: string, data: ArrayBuffer): string;
  /** 新建空白文档（RGB/8 位、白色 Background）；返回与 openImage 同形的 JSON 字符串。 */
  newDocument(width: number, height: number, name: string): string;
  /** 画布旋转/翻转：turn 0=水平翻转、1=垂直翻转、2=180°、3=顺时针 90°、4=逆时针 90°。 */
  documentTurn(documentId: number, turn: number): string;
  /** 历史记录：返回 `{undo:string[],redo:string[]}`（最近一步在前）。 */
  history(documentId: number): string;
  /** 一次跳转多步撤销；返回 `{steps:number, info:DocumentInfo}`。 */
  undoSteps(documentId: number, steps: number): string;
  /** 一次跳转多步重做；返回 `{steps:number, info:DocumentInfo}`。 */
  redoSteps(documentId: number, steps: number): string;
  close(documentId: number): void;
  info(documentId: number): string;
  preview(documentId: number, maxSide: number): ArrayBuffer;
  layerThumbnail(documentId: number, layerId: number, side: number): ArrayBuffer;
  render(documentId: number, x: number, y: number, w: number, h: number): ArrayBuffer;
  paintStroke(documentId: number, layerId: number, paramsJson: string, pointsJson: string): string;
  select(documentId: number, paramsJson: string): string;
  selectionInfo(documentId: number): string;
  filterApply(documentId: number, layerId: number, paramsJson: string): string;
  shapeCreate(documentId: number, paramsJson: string): string;
  fontRegister(data: ArrayBuffer): string;
  textCreate(documentId: number, paramsJson: string): string;
  layerEditGet(documentId: number, layerId: number): string;
  layerEditApply(documentId: number, layerId: number, paramsJson: string): string;
  setLayerVisible(documentId: number, layerId: number, visible: boolean): void;
  setLayerOpacity(documentId: number, layerId: number, opacity: number): void;
  setLayerFillOpacity(documentId: number, layerId: number, opacity: number): void;
  setLayerBlend(documentId: number, layerId: number, mode: string): void;
  /** 新建空白像素图层；aboveLayerId=0 表示插到最顶层之上。name 为空则自动命名。返回 JSON 字符串。 */
  createLayer(documentId: number, aboveLayerId: number, name: string): string;
  renameLayer(documentId: number, layerId: number, name: string): void;
  moveLayer(documentId: number, layerId: number, delta: number): string;
  duplicateLayer(documentId: number, layerId: number): string;
  duplicateLayerMany(documentId: number, idsJson: string): string;
  mergeLayer(documentId: number, layerId: number): string;
  deleteLayer(documentId: number, layerId: number): void;
  /** 批量删除：idsJson 是 JSON 数组字符串；整批共享一次撤销点，返回 `{"ok":true,"data":{"deleted":N}}` */
  layerDeleteMany(documentId: number, idsJson: string): string;
  addAdjustment(documentId: number, kind: string, paramsJson: string): string;
  updateAdjustment(documentId: number, layerId: number, paramsJson: string): void;
  getAdjustment(documentId: number, layerId: number): string;
  undo(documentId: number): string;
  redo(documentId: number): string;
  exportImage(documentId: number, format: string, quality: number): ArrayBuffer;
  layerEffects(documentId: number, layerId: number): string;
  updateLayerEffect(documentId: number, layerId: number, paramsJson: string): string;
  renderPixels(documentId: number, x: number, y: number, w: number, h: number): ArrayBuffer;
  revision(documentId: number): string;
  layerMaskGet(documentId: number, layerId: number): string;
  layerMaskAdd(documentId: number, layerId: number, fill: string): string;
  layerMaskSet(documentId: number, layerId: number, paramsJson: string): string;
  layerMaskFillFromSelection(documentId: number, layerId: number, mode: string): string;
  layerMaskApply(documentId: number, layerId: number): string;
  layerMaskRemove(documentId: number, layerId: number): string;
  layerGroup(documentId: number, idsJson: string, name: string): string;
  layerUngroup(documentId: number, layerId: number): string;
  layerExpanded(documentId: number, layerId: number, expanded: boolean): string;
  layerClipped(documentId: number, layerId: number, clipped: boolean): string;
  /** 裁剪到文档坐标矩形 x0,y0,x1,y1（x1/y1 独占）。返回 {"ok":..,"data"|"error"} 的 JSON 字符串。 */
  documentCrop(documentId: number, x0: number, y0: number, x1: number, y1: number): string;
  /** 画布尺寸调整；anchor ∈ 0..8（row*3+col，行=上/中/下，列=左/中/右），默认 4=中心。 */
  documentResizeCanvas(documentId: number, width: number, height: number, anchor: number): string;
  /** 读取参考线与标尺原点：`{horizontal:number[], vertical:number[], origin:[x,y]}`（只读）。 */
  guidesGet(documentId: number): string;
  /** 全量替换参考线与标尺原点；三个键均可选（缺省键表示不改该轴），同值不写入、不占撤销栈。 */
  guidesSet(documentId: number, paramsJson: string): string;
  /** 图层自由变换（`C-XFORM-API@1`）：参数相对当前状态；返回 `{bounds:[x0,y0,x1,y1],scaleX,scaleY,angle}`。 */
  layerTransform(documentId: number, layerId: number, paramsJson: string): string;
};

export default native;