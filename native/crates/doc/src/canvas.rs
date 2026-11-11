//! 画布区域变更：按矩形裁剪（crop）与按 9 宫格锚点调整画布尺寸（canvas size）。
//!
//! 两条入口收敛到同一个私有变换 `Document::apply_canvas`，因此"裁剪"与"改尺寸"共享一份
//! 图层/蒙版/选区/样式参考点的遍历逻辑，避免两套几何语义漂移。
//!
//! **坐标约定**：所有坐标都是文档像素，`Rect` 半开区间 `[x0, x1) × [y0, y1)`。变换把
//! 旧坐标 `p` 映射到新坐标 `p - (dx, dy)`，其中 `(dx, dy)` 是目标画布矩形在旧坐标中的左上角。
//! 本模块只改文档的几何与像素，不做 undo 快照、不做 JSON、不碰 `revision`（由桥接层负责）。

use photocraft_geom::{Affine, Point, Rect};

use crate::{Document, Group, Layer, LayerContent, LiveShape, Path, ShapeLayer, SmartObject, Surface, TextLayer};

/// 画布单边上限（像素）。
pub const MAX_CANVAS_SIDE: u32 = 16384;

/// 画布总像素上限：64 M（RGBA8 ≈ 256 MB）。
pub const MAX_CANVAS_PIXELS: u64 = 64 << 20;

/// 画布尺寸调整的 9 宫格锚点（语义与 Photoshop Canvas Size 一致）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CanvasAnchor {
    TopLeft,
    Top,
    TopRight,
    Left,
    Center,
    Right,
    BottomLeft,
    Bottom,
    BottomRight,
}

impl CanvasAnchor {
    /// 表格顺序：行优先，`index() == row * 3 + col`，row 0=上/1=中/2=下，col 0=左/1=中/2=右。
    pub const ALL: [CanvasAnchor; 9] = [
        CanvasAnchor::TopLeft,
        CanvasAnchor::Top,
        CanvasAnchor::TopRight,
        CanvasAnchor::Left,
        CanvasAnchor::Center,
        CanvasAnchor::Right,
        CanvasAnchor::BottomLeft,
        CanvasAnchor::Bottom,
        CanvasAnchor::BottomRight,
    ];

    /// 0..=8 的稳定索引（`row * 3 + col`）。
    pub fn index(self) -> i32 {
        match self {
            CanvasAnchor::TopLeft => 0,
            CanvasAnchor::Top => 1,
            CanvasAnchor::TopRight => 2,
            CanvasAnchor::Left => 3,
            CanvasAnchor::Center => 4,
            CanvasAnchor::Right => 5,
            CanvasAnchor::BottomLeft => 6,
            CanvasAnchor::Bottom => 7,
            CanvasAnchor::BottomRight => 8,
        }
    }

    /// 由索引反查锚点；越界返回 `None`。
    pub fn from_index(index: i32) -> Option<CanvasAnchor> {
        usize::try_from(index).ok().and_then(|i| Self::ALL.get(i)).copied()
    }

    /// 原内容在新画布中的偏移：返回 `(nx0, ny0)` = 新画布原点在旧坐标中的位置（可为负）。
    ///
    /// `col`（`index() % 3`）：左 = 0；中 = `(old_w - new_w).div_euclid(2)`；右 = `old_w - new_w`
    /// （纵向同理）。差值为奇数时多出的 1 像素落在左/上侧。中间量用 `i64` 计算并 clamp 到 `i32`。
    pub fn origin(self, old_w: u32, old_h: u32, new_w: u32, new_h: u32) -> (i32, i32) {
        let idx = self.index();
        (axis_origin(idx % 3, old_w, new_w), axis_origin(idx / 3, old_h, new_h))
    }
}

/// 单轴偏移：`align` 0=左/上，1=中，2=右/下。
fn axis_origin(align: i32, old: u32, new: u32) -> i32 {
    let delta = i64::from(old) - i64::from(new);
    let offset = match align {
        0 => 0,
        1 => delta.div_euclid(2),
        _ => delta,
    };
    offset.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

/// 把矩形规范化为正方向（`x0 > x1` / `y0 > y1` 时交换），与既有 `select_rect` 的语义一致。
fn normalize_rect(r: Rect) -> Rect {
    Rect::new(r.x0.min(r.x1), r.y0.min(r.y1), r.x0.max(r.x1), r.y0.max(r.y1))
}

impl Document {
    /// 裁剪到 `keep`（文档坐标，`x1`/`y1` 独占语义与 `Rect` 一致）。
    ///
    /// 校验顺序：先规范化（若 `x0>x1` / `y0>y1` 则交换）→ 与 `self.bounds()` 求交 → 判空。
    /// 返回 `Ok(true)` = 已变更（调用方才可 checkpoint）；`Ok(false)` = 无变化（不 checkpoint）；
    /// `Err(msg)` = 非法参数，**文档保证完全未被修改**。
    pub fn crop_to(&mut self, keep: Rect) -> Result<bool, String> {
        let target = normalize_rect(keep).intersect(&self.bounds());
        if target.is_empty() {
            return Err("crop rectangle is outside the document".to_string());
        }
        if target == self.bounds() {
            // 与当前画布完全一致：无变化，不 checkpoint。
            return Ok(false);
        }
        self.apply_canvas(target);
        Ok(true)
    }

    /// 按目标尺寸与锚点调整画布。
    ///
    /// 校验：`w`/`h` ∈ `1..=MAX_CANVAS_SIDE` 且 `w*h <= MAX_CANVAS_PIXELS`。返回语义同
    /// [`Self::crop_to`]（目标尺寸与当前相同 => `Ok(false)`）。扩展区为透明（默认像素）。
    pub fn resize_canvas(&mut self, width: u32, height: u32, anchor: CanvasAnchor) -> Result<bool, String> {
        if width == 0 || height == 0 || width > MAX_CANVAS_SIDE || height > MAX_CANVAS_SIDE {
            return Err("invalid canvas size".to_string());
        }
        if u64::from(width) * u64::from(height) > MAX_CANVAS_PIXELS {
            return Err("canvas is too large".to_string());
        }
        if width == self.size.width && height == self.size.height {
            return Ok(false);
        }
        let (nx0, ny0) = anchor.origin(self.size.width, self.size.height, width, height);
        let target = Rect::from_xywh(nx0, ny0, width, height);
        self.apply_canvas(target);
        Ok(true)
    }

    /// 单一变换入口：`target` = 新画布在**旧**文档坐标中的矩形（调用方已保证非空）。
    ///
    /// **先算后写**：在文档副本上完成全部变换，成功后才替换自身，中途 panic 不会留下半变换。
    /// 副本与原件共享像素 tile（`Surface` 写时复制），代价只有图层结构本身。
    ///
    /// `(mx, my) = -(target.x0, target.y0)` 是施加给每个几何量的平移量；`size` 最后一步提交。
    fn apply_canvas(&mut self, target: Rect) {
        let mut next = self.clone();
        next.apply_canvas_in_place(target);
        *self = next;
    }

    /// [`Self::apply_canvas`] 的实际变换步骤。
    fn apply_canvas_in_place(&mut self, target: Rect) {
        let (mx, my) = (target.x0.saturating_neg(), target.y0.saturating_neg());

        for layer in &mut self.layers {
            apply_layer(layer, target, mx, my);
        }

        // 文档选区：同步裁剪并平移（保持"原选中像素仍被选中"），结果为空才置 None。
        if let Some(selection) = self.selection.take() {
            self.selection = moved_coverage(selection, target, mx, my);
        }
        if let Some(quick) = self.quick_mask.as_mut() {
            let moved = quick.surface.translated(mx, my, target);
            quick.surface = moved;
        }
        for channel in &mut self.channels {
            let moved = channel.surface.translated(mx, my, target);
            channel.surface = moved;
        }
        for named in &mut self.paths {
            translate_path(&mut named.path, mx, my);
        }
        if let Some(work) = self.work_path.as_mut() {
            translate_path(work, mx, my);
        }
        translate_guides(&mut self.guides, target, mx, my);
        self.ruler_origin[0] += f64::from(mx);
        self.ruler_origin[1] += f64::from(my);

        self.size = target.size();
    }
}

/// 变换一个图层：自身像素、蒙版、矢量蒙版、样式参考点、视频帧与填充缓存。
///
/// 不改 `id`/`name`/`visible`/`opacity`/`blend`/`clipped`/`link_group`/`locks`/`label`/`advanced`/
/// `blend_if`/`excluded_channels`/`xmp` 等非几何字段。
fn apply_layer(layer: &mut Layer, target: Rect, mx: i32, my: i32) {
    // 填充缓存按旧画布布局渲染，裁剪/扩边后必须重渲染，不能复用。
    layer.fill_cache = None;

    if let Some(mask) = layer.mask.as_mut() {
        let moved = mask.surface.translated(mx, my, target);
        mask.surface = moved;
    }
    if let Some(vector_mask) = layer.vector_mask.as_mut() {
        translate_path(&mut vector_mask.path, mx, my);
    }
    if let Some((x, y)) = layer.effects.reference {
        // `fxrp` 是文档绝对坐标，随图层平移；`items` 内的偏移/角度是相对量，不动。
        layer.effects.reference = Some((x + f64::from(mx), y + f64::from(my)));
    }

    apply_content(&mut layer.content, target, mx, my);

    if let Some(video) = layer.video.as_mut() {
        for frame in video.frames.iter_mut().chain(video.original_frames.iter_mut().flatten()) {
            let moved = frame.translated(mx, my, target);
            *frame = moved;
        }
    }
}

/// 按 `LayerContent` 变体变换内容。
fn apply_content(content: &mut LayerContent, target: Rect, mx: i32, my: i32) {
    match content {
        LayerContent::Raster(surface) => {
            let moved = surface.translated(mx, my, target);
            *surface = moved;
        }
        LayerContent::Group(group) => apply_group(group, target, mx, my),
        // 调整层无自身像素，渲染时按新画布实时作用。
        LayerContent::Adjustment(_) => {}
        // 填充层没有像素几何；其 `fill_cache` 已在 `apply_layer` 中作废。
        LayerContent::Fill(_) => {}
        LayerContent::Text(text) => apply_text(text, target, mx, my),
        LayerContent::Shape(shape) => apply_shape(shape, target, mx, my),
        LayerContent::Smart(smart) => apply_smart(smart, target, mx, my),
    }
}

/// 组：递归子层，并按同一平移量移动 artboard 矩形。
fn apply_group(group: &mut Group, target: Rect, mx: i32, my: i32) {
    for child in &mut group.children {
        apply_layer(child, target, mx, my);
    }
    if let Some(artboard) = group.artboard.as_mut() {
        // `frame_stroke` 是沿矢量蒙版的相对描边参数，不动。
        artboard.rect = artboard.rect.translate(mx, my);
    }
}

/// 文字层：前置平移 `transform`，并裁剪/平移已栅格化外观。
///
/// `shape`（`Box`/`Path`/`Area`）是 text-space 局部坐标，**不重排、不平移**（裁剪不重排文字）。
fn apply_text(text: &mut TextLayer, target: Rect, mx: i32, my: i32) {
    text.transform = Affine::translate(f64::from(mx), f64::from(my)).mul(&text.transform);
    if let Some(cache) = text.cache.as_ref() {
        let moved = cache.translated(mx, my, target);
        text.cache = Some(moved);
    }
}

/// 形状层：平移 `path` 的全部 knot、`live` 参数化几何与已栅格化外观。
fn apply_shape(shape: &mut ShapeLayer, target: Rect, mx: i32, my: i32) {
    translate_path(&mut shape.path, mx, my);
    if let Some(live) = shape.live.as_mut() {
        translate_live(live, f64::from(mx), f64::from(my));
    }
    if let Some(cache) = shape.cache.as_ref() {
        let moved = cache.translated(mx, my, target);
        shape.cache = Some(moved);
    }
}

/// 智能对象：平移 `transform`、透视矩阵、外观缓存与智能滤镜蒙版。
fn apply_smart(smart: &mut SmartObject, target: Rect, mx: i32, my: i32) {
    smart.transform = Affine::translate(f64::from(mx), f64::from(my)).mul(&smart.transform);
    if let Some(perspective) = smart.perspective.as_mut() {
        // 行主序 3×3：x' = (p0·x + p1·y + p2) / (p6·x + p7·y + p8)，故平移只修正分子常数项。
        perspective[2] += f64::from(mx) * perspective[8];
        perspective[5] += f64::from(my) * perspective[8];
    }
    if let Some(cache) = smart.cache.as_ref() {
        let moved = cache.translated(mx, my, target);
        smart.cache = Some(moved);
    }
    if let Some(mask) = smart.filter_mask.as_mut() {
        let moved = mask.surface.translated(mx, my, target);
        mask.surface = moved;
    }
}

/// 平移矢量路径的每个 knot（锚点与两个控制点）。
fn translate_path(path: &mut Path, mx: i32, my: i32) {
    let (ox, oy) = (f64::from(mx), f64::from(my));
    for subpath in &mut path.subpaths {
        for knot in &mut subpath.knots {
            knot.anchor = Point::new(knot.anchor.x + ox, knot.anchor.y + oy);
            knot.in_ctrl = Point::new(knot.in_ctrl.x + ox, knot.in_ctrl.y + oy);
            knot.out_ctrl = Point::new(knot.out_ctrl.x + ox, knot.out_ctrl.y + oy);
        }
    }
}

/// 平移参数化（"live"）形状的几何，使其与 `path` 保持同步，否则再次编辑会跳位。
fn translate_live(live: &mut LiveShape, mx: f64, my: f64) {
    match live {
        LiveShape::Rect { rect, .. } | LiveShape::Ellipse { rect } | LiveShape::Polygon { rect, .. } => {
            rect[0] += mx;
            rect[1] += my;
        }
        LiveShape::Line { from, to, .. } | LiveShape::ArrowLine { from, to, .. } => {
            from[0] += mx;
            from[1] += my;
            to[0] += mx;
            to[1] += my;
        }
    }
}

/// 平移并裁剪一个可选的灰度覆盖面；结果不再覆盖任何像素（默认值 ≤ 0 且无 tile）时返回 `None`。
fn moved_coverage(surface: Surface, target: Rect, mx: i32, my: i32) -> Option<Surface> {
    let moved = surface.translated(mx, my, target);
    let unselected = moved.default_pixel().iter().all(|v| *v <= 0.0);
    if unselected && moved.content_bounds().is_empty() { None } else { Some(moved) }
}

/// 平移标尺参考线并丢弃落在新画布之外的线与颜色。
///
/// `GuideColor` 以（方向, 位置）为键，位置必须与线同步平移并按保留结果裁剪，否则颜色会错位。
fn translate_guides(guides: &mut crate::Guides, target: Rect, mx: i32, my: i32) {
    let (x0, x1) = (f64::from(target.x0), f64::from(target.x1));
    let (y0, y1) = (f64::from(target.y0), f64::from(target.y1));
    guides.horizontal.retain(|v| *v >= y0 && *v <= y1);
    guides.vertical.retain(|v| *v >= x0 && *v <= x1);
    let (ox, oy) = (f64::from(mx), f64::from(my));
    for v in &mut guides.horizontal {
        *v += oy;
    }
    for v in &mut guides.vertical {
        *v += ox;
    }
    for color in &mut guides.colors {
        color.position += if color.vertical { ox } else { oy };
    }
    guides.prune_colors();
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

    use std::sync::Arc;

    use photocraft_geom::Size;

    use super::*;
    use crate::{
        Artboard, Color, ColorMode, Fill, FillCache, Guides, LayerMask, PixelFormat, SampleType, SmartSource, Subpath, VectorMask,
    };

    /// 以像素取色断言 `(r,g,b,a)` 均为给定值（RGBA8）。
    fn assert_rgba(actual: Vec<f32>, expected: [f32; 4]) {
        assert_eq!(actual.len(), 4);
        for (a, e) in actual.iter().zip(expected.iter()) {
            assert!((a - e).abs() < 1e-6, "got {actual:?}, expected {expected:?}");
        }
    }

    /// 100×100、RGB/U8、无图层的文档。
    fn doc_100() -> Document {
        Document::new("t", Size::new(100, 100), ColorMode::Rgb, SampleType::U8)
    }

    /// 一个 RGBA8 像素层，`rect` 内填红色。
    fn red_layer(rect: Rect) -> Layer {
        let mut layer = Layer::raster("px", PixelFormat::RGBA8);
        if let Some(surface) = layer.surface_mut() {
            surface.fill_rect(rect, &[1.0, 0.0, 0.0, 1.0]);
        }
        layer
    }

    /// 灰度覆盖面（选择/蒙版），`rect` 内填 1。
    fn coverage(rect: Rect) -> Surface {
        let mut surface = Surface::with_default(PixelFormat::GRAY8, &[0.0]);
        surface.fill_rect(rect, &[1.0]);
        surface
    }

    // ---- 断言组 1：`from_index` 与 `index()` 往返 ----
    #[test]
    fn e1_anchor_index_roundtrip_and_bounds() {
        for (i, anchor) in CanvasAnchor::ALL.iter().enumerate() {
            assert_eq!(anchor.index(), i as i32);
            assert_eq!(CanvasAnchor::from_index(i as i32), Some(*anchor));
        }
        assert_eq!(CanvasAnchor::from_index(-1), None);
        assert_eq!(CanvasAnchor::from_index(9), None);
        assert_eq!(CanvasAnchor::from_index(i32::MAX), None);
    }

    // ---- 断言组 2：`origin` 九宫格 + 奇数差 ----
    #[test]
    fn e2_origin_nine_cells_and_odd_difference() {
        // 横向 100 → 200
        assert_eq!(CanvasAnchor::Left.origin(100, 100, 200, 100), (0, 0));
        assert_eq!(CanvasAnchor::Center.origin(100, 100, 200, 100), (-50, 0));
        assert_eq!(CanvasAnchor::Right.origin(100, 100, 200, 100), (-100, 0));
        // 纵向 100 → 200
        assert_eq!(CanvasAnchor::Top.origin(100, 100, 100, 200), (0, 0));
        assert_eq!(CanvasAnchor::Center.origin(100, 100, 100, 200), (0, -50));
        assert_eq!(CanvasAnchor::Bottom.origin(100, 100, 100, 200), (0, -100));
        // 缩小 200 → 100：右/下锚点保留右侧/下侧
        assert_eq!(CanvasAnchor::Right.origin(200, 100, 100, 100), (100, 0));
        assert_eq!(CanvasAnchor::Bottom.origin(100, 200, 100, 100), (0, 100));
        // 双轴组合（右下、左上）
        assert_eq!(CanvasAnchor::BottomRight.origin(100, 100, 250, 150), (-150, -50));
        assert_eq!(CanvasAnchor::TopLeft.origin(100, 100, 40, 60), (0, 0));
        // 奇数差 100 → 101：多出的 1 像素落在左/上侧
        assert_eq!(CanvasAnchor::Center.origin(100, 100, 101, 101), (-1, -1));
        assert_eq!(CanvasAnchor::Center.origin(101, 101, 100, 100), (0, 0));
    }

    // ---- 断言组 3：crop 三情形 ----
    #[test]
    fn e3_crop_same_outside_and_partial_rectangles() {
        let mut doc = doc_100();
        // 与当前 bounds 相同 → Ok(false)，尺寸不变
        assert_eq!(doc.crop_to(Rect::new(0, 0, 100, 100)), Ok(false));
        assert_eq!(doc.size, Size::new(100, 100));
        // 完全越界 → Err，文档不被修改
        assert_eq!(doc.crop_to(Rect::new(200, 200, 300, 300)), Err("crop rectangle is outside the document".to_string()));
        assert_eq!(doc.size, Size::new(100, 100));
        // 空矩形 → 同一错误
        assert_eq!(doc.crop_to(Rect::new(10, 10, 10, 40)), Err("crop rectangle is outside the document".to_string()));
        // 部分越界 → Ok(true)，结果尺寸取交集
        assert_eq!(doc.crop_to(Rect::new(80, 80, 140, 140)), Ok(true));
        assert_eq!(doc.size, Size::new(20, 20));
        // 反向矩形自动规范化
        let mut doc = doc_100();
        assert_eq!(doc.crop_to(Rect::new(60, 60, 20, 20)), Ok(true));
        assert_eq!(doc.size, Size::new(40, 40));
    }

    // ---- 断言组 4：resize 四种非法 + 尺寸不变 ----
    #[test]
    fn e4_resize_rejects_invalid_sizes_and_noops_on_same_size() {
        let mut doc = doc_100();
        let center = CanvasAnchor::Center;
        assert_eq!(doc.resize_canvas(0, 100, center), Err("invalid canvas size".to_string()));
        assert_eq!(doc.resize_canvas(100, 0, center), Err("invalid canvas size".to_string()));
        assert_eq!(doc.resize_canvas(MAX_CANVAS_SIDE + 1, 100, center), Err("invalid canvas size".to_string()));
        assert_eq!(doc.resize_canvas(100, MAX_CANVAS_SIDE + 1, center), Err("invalid canvas size".to_string()));
        // 单边合法但总像素超过 64 M
        assert_eq!(doc.resize_canvas(16384, 16384, center), Err("canvas is too large".to_string()));
        // 恰好等于上限（16384 × 4096 = 64 M）合法
        assert_eq!(doc.resize_canvas(16384, 4096, center), Ok(true));
        // 尺寸不变 → Ok(false)
        let mut doc = doc_100();
        assert_eq!(doc.resize_canvas(100, 100, center), Ok(false));
        assert_eq!(doc.size, Size::new(100, 100));
    }

    // ---- 断言组 4 续：resize 生效时内容位置与透明扩展 ----
    #[test]
    fn e5_resize_places_content_by_anchor_and_expands_transparent() {
        // 左锚点：原内容保持 x=0..100，右侧新区域透明
        let mut doc = doc_100();
        doc.layers.push(red_layer(Rect::new(0, 0, 100, 100)));
        assert_eq!(doc.resize_canvas(200, 100, CanvasAnchor::Left), Ok(true));
        assert_eq!(doc.size, Size::new(200, 100));
        let surface = doc.layers[0].surface().unwrap();
        assert_rgba(surface.pixel(0, 0), [1.0, 0.0, 0.0, 1.0]);
        assert_rgba(surface.pixel(199, 0), [0.0; 4]);

        // 右锚点：原内容移到 x=100..200，左侧新区域透明
        let mut doc = doc_100();
        doc.layers.push(red_layer(Rect::new(0, 0, 100, 100)));
        assert_eq!(doc.resize_canvas(200, 100, CanvasAnchor::Right), Ok(true));
        let surface = doc.layers[0].surface().unwrap();
        assert_rgba(surface.pixel(0, 0), [0.0; 4]);
        assert_rgba(surface.pixel(100, 0), [1.0, 0.0, 0.0, 1.0]);
        assert_rgba(surface.pixel(199, 0), [1.0, 0.0, 0.0, 1.0]);
    }

    // ---- 断言组 5：裁剪后像素层 / 文字层 / 形状层几何 ----
    #[test]
    fn e6_crop_moves_raster_text_and_shape_geometry() {
        let mut doc = doc_100();
        doc.layers.push(red_layer(Rect::new(20, 20, 30, 30)));

        let text = TextLayer {
            text: "hi".to_string(),
            transform: Affine::translate(3.0, 4.0),
            cache: Some(coverage_rgba(Rect::new(20, 20, 30, 30))),
            ..Default::default()
        };
        doc.layers.push(Layer::new("type", LayerContent::Text(text)));

        let shape = ShapeLayer {
            path: Path::new(vec![Subpath::polygon(&[(20.0, 30.0), (40.0, 30.0), (40.0, 50.0)])]),
            live: Some(LiveShape::Rect { rect: [20.0, 30.0, 20.0, 20.0], radii: [0.0; 4] }),
            cache: Some(coverage_rgba(Rect::new(20, 30, 40, 50))),
            ..Default::default()
        };
        doc.layers.push(Layer::new("shape", LayerContent::Shape(shape)));

        // 组：递归子层 + artboard 矩形
        let mut child = red_layer(Rect::new(40, 40, 50, 50));
        child.name = "child".to_string();
        let mut group = Layer::group("g", vec![child]);
        if let LayerContent::Group(inner) = &mut group.content {
            inner.artboard = Some(Artboard::new(Rect::new(20, 20, 60, 60)));
        }
        doc.layers.push(group);

        assert_eq!(doc.crop_to(Rect::new(10, 10, 60, 50)), Ok(true));
        assert_eq!(doc.size, Size::new(50, 40));

        // 像素层：旧 (20,20) 现在位于 (10,10)
        let raster = doc.layers[0].surface().unwrap();
        assert_rgba(raster.pixel(10, 10), [1.0, 0.0, 0.0, 1.0]);
        assert_rgba(raster.pixel(0, 0), [0.0; 4]);

        // 文字层：transform 前置平移 (dx,dy) 的负值；shape 不重排
        let LayerContent::Text(text) = &doc.layers[1].content else { panic!("expected text") };
        assert_eq!(text.transform.m[4], -7.0);
        assert_eq!(text.transform.m[5], -6.0);
        assert_eq!(text.text, "hi");
        let cache = text.cache.as_ref().unwrap();
        assert_rgba(cache.pixel(10, 10), [1.0, 0.0, 0.0, 1.0]);

        // 形状层：全部 knot 与 live 几何同步平移
        let LayerContent::Shape(shape) = &doc.layers[2].content else { panic!("expected shape") };
        let anchors: Vec<(f64, f64)> = shape.path.subpaths[0].knots.iter().map(|k| (k.anchor.x, k.anchor.y)).collect();
        assert_eq!(anchors, vec![(10.0, 20.0), (30.0, 20.0), (30.0, 40.0)]);
        assert_eq!(shape.path.subpaths[0].knots[0].in_ctrl, Point::new(10.0, 20.0));
        let Some(LiveShape::Rect { rect, .. }) = shape.live else { panic!("expected live rect") };
        assert_eq!(rect, [10.0, 20.0, 20.0, 20.0]);
        let shape_cache = shape.cache.as_ref().unwrap();
        assert_rgba(shape_cache.pixel(10, 20), [1.0, 0.0, 0.0, 1.0]);

        // 组：递归子层 + artboard 矩形（一次裁剪内完成）
        let LayerContent::Group(inner) = &doc.layers[3].content else { panic!("expected group") };
        assert_eq!(inner.artboard.as_ref().unwrap().rect, Rect::new(10, 10, 50, 50));
        assert_rgba(inner.children[0].surface().unwrap().pixel(30, 30), [1.0, 0.0, 0.0, 1.0]);
    }

    // ---- 断言组 5 续：蒙版 / 矢量蒙版 / 选区 / effects.reference / guides / ruler ----
    #[test]
    fn e7_crop_moves_masks_selection_effects_and_document_geometry() {
        let mut doc = doc_100();
        let mut layer = red_layer(Rect::new(20, 20, 30, 30));

        let mut mask = LayerMask::reveal_all();
        mask.surface.fill_rect(Rect::new(20, 20, 30, 30), &[0.0]);
        layer.mask = Some(mask);
        layer.vector_mask = Some(VectorMask::new(Path::new(vec![Subpath::polygon(&[
            (20.0, 20.0),
            (40.0, 20.0),
            (40.0, 40.0),
        ])])));
        layer.effects.reference = Some((25.0, 35.0));
        doc.layers.push(layer);

        let mut fill = Layer::new("fill", LayerContent::Fill(Fill::Solid(Color::BLACK)));
        fill.fill_cache = Some(FillCache { fill: Fill::Solid(Color::BLACK), surface: coverage(Rect::new(20, 20, 30, 30)) });
        doc.layers.push(fill);

        doc.selection = Some(coverage(Rect::new(20, 20, 30, 30)));
        // 20 与 15 在裁剪区内，90 / 500 落在新区间之外应被丢弃
        doc.guides = Guides { horizontal: vec![20.0, 500.0], vertical: vec![15.0, 90.0], colors: Vec::new() };
        doc.guides.set_color(true, 15.0, Some([1, 2, 3]));
        doc.ruler_origin = [5.0, 7.0];

        assert_eq!(doc.crop_to(Rect::new(10, 10, 60, 50)), Ok(true));
        assert_eq!(doc.size, Size::new(50, 40));

        let layer = &doc.layers[0];
        // 蒙版：旧 (20,20) 的 0 值现在在 (10,10)；未覆盖区保持默认 1.0
        let mask = layer.mask.as_ref().unwrap();
        assert!((mask.surface.pixel(10, 10)[0] - 0.0).abs() < 1e-6);
        assert!((mask.surface.pixel(0, 0)[0] - 1.0).abs() < 1e-6);
        assert_eq!(mask.surface.content_bounds(), Rect::new(10, 10, 20, 20));
        // 矢量蒙版：knot 平移
        let vector_mask = layer.vector_mask.as_ref().unwrap();
        assert_eq!(vector_mask.path.subpaths[0].knots[0].anchor, Point::new(10.0, 10.0));
        // 样式参考点：随图层平移
        assert_eq!(layer.effects.reference, Some((15.0, 25.0)));
        // 填充缓存作废
        assert!(doc.layers[1].fill_cache.is_none());

        // 选区同步裁剪并平移，且仍在覆盖同一批像素
        let selection = doc.selection.as_ref().unwrap();
        assert_eq!(selection.content_bounds(), Rect::new(10, 10, 20, 20));
        // 标尺参考线：保留区内的平移，区外的丢弃，颜色不错位
        assert_eq!(doc.guides.horizontal, vec![10.0]);
        assert_eq!(doc.guides.vertical, vec![5.0]);
        assert_eq!(doc.guides.color(true, 5.0), Some([1, 2, 3]));
        assert_eq!(doc.ruler_origin, [-5.0, -3.0]);
    }

    // ---- 断言组 5 续：智能对象与"裁剪后选区为空 → None" ----
    #[test]
    fn e8_crop_moves_smart_object_and_clears_empty_selection() {
        let mut doc = doc_100();
        let mut smart = SmartObject::new(
            SmartSource::Embedded { file_name: "a.png".to_string(), bytes: Arc::new(Vec::new()) },
            Affine::translate(3.0, 4.0),
            Some(coverage_rgba(Rect::new(20, 20, 30, 30))),
        );
        smart.perspective = Some([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]);
        let mut filter_mask = LayerMask::reveal_all();
        filter_mask.surface.fill_rect(Rect::new(20, 20, 30, 30), &[0.0]);
        smart.filter_mask = Some(filter_mask);
        doc.layers.push(Layer::new("smart", LayerContent::Smart(smart)));

        // 选区只在裁剪区之外：裁剪后应变为"无选区"
        doc.selection = Some(coverage(Rect::new(80, 80, 90, 90)));

        assert_eq!(doc.crop_to(Rect::new(10, 10, 60, 50)), Ok(true));
        let LayerContent::Smart(smart) = &doc.layers[0].content else { panic!("expected smart") };
        assert_eq!(smart.transform.m[4], -7.0);
        assert_eq!(smart.transform.m[5], -6.0);
        let perspective = smart.perspective.unwrap();
        assert_eq!(perspective[2], -10.0);
        assert_eq!(perspective[5], -10.0);
        assert_eq!(smart.cache.as_ref().unwrap().content_bounds(), Rect::new(10, 10, 20, 20));
        assert_eq!(smart.filter_mask.as_ref().unwrap().surface.content_bounds(), Rect::new(10, 10, 20, 20));

        assert!(doc.selection.is_none());
    }

    /// RGBA8 表面，`rect` 内填红（用于文字/形状/智能对象的已栅格化外观）。
    fn coverage_rgba(rect: Rect) -> Surface {
        let mut surface = Surface::new(PixelFormat::RGBA8);
        surface.fill_rect(rect, &[1.0, 0.0, 0.0, 1.0]);
        surface
    }
}