#![allow(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! C ABI bridge between the PhotoCraft Rust engine and the HarmonyOS ArkTS shell.
//!
//! Every call that returns JSON allocates a [`PcBuffer`] which the caller must release with
//! [`pc_buffer_free`]. Binary payloads (rendered pixels, encoded files) are written through the
//! same buffer type.

use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use photocraft_color::blend::{BlendMode, composite};
use photocraft_color::{Color, ColorMode, SampleType};
use photocraft_compose::masks::{combined_mask, feather_sigma};
use photocraft_compose::{Buffer, render_layer_in};
use photocraft_raster::{from_rgba_into, to_rgba, Rgba8Image, Surface};
use photocraft_doc::adjust::HueRange;
use photocraft_doc::text::{AntiAlias, TextShape};
use photocraft_doc::vector::{LiveShape, ShapeLayer, ShapeStroke};
use photocraft_doc::{
    Adjustment, Bevel, BevelStyle, BevelTechnique, Contour, Document, Effect, Fill, FxCommon, FxPaint, Glow, GlowSource, GlowTechnique, Gradient, GradientStyle,
    Layer, LayerContent, LayerMask, StrokeFx, StrokePosition, TextLayer,
};
use photocraft_geom::{Affine, Rect};

use photocraft_algo::selection::{self, SelectionMode};
use photocraft_algo::{FilterParams, apply_in, output_area};
use photocraft_paint::{apply_stroke, BrushSettings, Stroke, StrokePoint};
use photocraft_text::TextEngine;
use photocraft_vector::{flatten::validate_shape, render_shape, shapes::live_path};
use serde::Deserialize;
use serde_json::{Value, json};

const MAX_UNDO: usize = 24;
const MAX_PREVIEW_SIDE: u32 = 8192;
const MAX_RENDER_PIXELS: u64 = 16 << 20;

/// An owned byte buffer handed to the ArkTS side.
#[repr(C)]
pub struct PcBuffer {
    ptr: *mut u8,
    len: usize,
    cap: usize,
}

impl PcBuffer {
    fn from_vec(mut v: Vec<u8>) -> *mut PcBuffer {
        v.shrink_to_fit();
        let (ptr, len, cap) = (v.as_mut_ptr(), v.len(), v.capacity());
        std::mem::forget(v);
        Box::into_raw(Box::new(PcBuffer { ptr, len, cap }))
    }

    fn into_vec(ptr: *mut PcBuffer) -> Option<Vec<u8>> {
        if ptr.is_null() {
            return None;
        }
        // SAFETY: the pointer came from `PcBuffer::from_vec` and is freed exactly once.
        let b = unsafe { Box::from_raw(ptr) };
        if b.ptr.is_null() {
            return None;
        }
        // SAFETY: ptr/len/cap are the original Vec's allocation.
        Some(unsafe { Vec::from_raw_parts(b.ptr, b.len, b.cap) })
    }
}

/// Releases a buffer returned by any `pc_*` call. Passing NULL is a no-op.
#[unsafe(no_mangle)]
pub extern "C" fn pc_buffer_free(ptr: *mut PcBuffer) {
    drop(PcBuffer::into_vec(ptr));
}

struct DocEntry {
    doc: Document,
    undo: Vec<Document>,
    redo: Vec<Document>,
    /// 与 `undo` 同长同序的操作标签（历史面板展示用）。
    undo_labels: Vec<String>,
    /// 与 `redo` 同长同序的操作标签。
    redo_labels: Vec<String>,
    /// Monotonic document version used by the ArkTS render cache (see `pc_revision`).
    revision: u64,
}

impl DocEntry {
    fn new(doc: Document) -> Self {
        Self { doc, undo: Vec::new(), redo: Vec::new(), undo_labels: Vec::new(), redo_labels: Vec::new(), revision: 0 }
    }

    /// 默认标签「编辑」：未显式命名的入口使用它。
    fn checkpoint(&mut self) {
        self.checkpoint_named("编辑");
    }

    /// 压入变更前快照并记录操作标签；淘汰最旧、清空重做、递增 revision 的规则与旧版一致。
    fn checkpoint_named(&mut self, label: &str) {
        self.undo.push(self.doc.clone());
        self.undo_labels.push(label.to_string());
        if self.undo.len() > MAX_UNDO {
            self.undo.remove(0);
            self.undo_labels.remove(0);
        }
        self.redo.clear();
        self.redo_labels.clear();
        self.bump_revision();
    }

    /// 用调用方提供的**变更前**文档快照压入撤销栈并记录标签。淘汰最旧、清空重做、递增
    /// revision 的规则与 [`Self::checkpoint_named`] 完全一致，唯一区别是入栈的不是"当前
    /// （已变更）文档"。
    ///
    /// 供"必须先在 doc 层改完才能知道是否真的变更"的入口使用（`pc_document_crop` /
    /// `pc_document_resize_canvas`）：这类入口无法沿用"先记账后写"的顺序，否则撤销栈里存的
    /// 是裁剪/缩放之后的文档，`pc_undo` 会把"还原"变成"再裁一次"。
    fn checkpoint_snapshot_named(&mut self, before: Document, label: &str) {
        self.undo.push(before);
        self.undo_labels.push(label.to_string());
        if self.undo.len() > MAX_UNDO {
            self.undo.remove(0);
            self.undo_labels.remove(0);
        }
        self.redo.clear();
        self.redo_labels.clear();
        self.bump_revision();
    }

    /// Increments the document revision without wrapping: `u64::MAX` is unreachable in practice
    /// and staying flat there keeps the value monotonic for cache consumers.
    fn bump_revision(&mut self) {
        self.revision = self.revision.saturating_add(1);
    }
}

pub struct Session {
    docs: HashMap<u64, DocEntry>,
    next_id: u64,
}

impl Session {
    fn entry(&self, id: u64) -> Result<&DocEntry, String> {
        self.docs.get(&id).ok_or_else(|| format!("document {id} is not open"))
    }

    fn entry_mut(&mut self, id: u64) -> Result<&mut DocEntry, String> {
        self.docs.get_mut(&id).ok_or_else(|| format!("document {id} is not open"))
    }

    fn add(&mut self, doc: Document, image_name: &str) -> u64 {
        self.next_id += 1;
        let id = self.next_id;
        let mut entry = DocEntry::new(doc);
        entry.doc.name = image_name.to_string();
        self.docs.insert(id, entry);
        id
    }
}

fn ok(data: Value) -> Vec<u8> {
    json!({ "ok": true, "data": data }).to_string().into_bytes()
}

/// 成功响应 + 顶层 `perf`（`data` 结构与 [`ok`] 完全一致，仅在顶层多一个 `perf` 对象）。
fn ok_perf(data: Value, perf: Value) -> Vec<u8> {
    json!({ "ok": true, "data": data, "perf": perf }).to_string().into_bytes()
}

fn fail(message: impl Into<String>) -> Vec<u8> {
    json!({ "ok": false, "error": message.into() }).to_string().into_bytes()
}

fn guard(f: impl FnOnce() -> Vec<u8>) -> *mut PcBuffer {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(bytes) => PcBuffer::from_vec(bytes),
        Err(_) => PcBuffer::from_vec(fail("internal error: the engine panicked")),
    }
}

unsafe fn slice<'a>(ptr: *const u8, len: usize) -> Result<&'a [u8], String> {
    if len == 0 {
        return Ok(&[]);
    }
    if ptr.is_null() {
        return Err("null pointer".into());
    }
    // SAFETY: the caller guarantees ptr/len describe a live buffer for the duration of the call.
    Ok(unsafe { std::slice::from_raw_parts(ptr, len) })
}

fn find_layer<'a>(layers: &'a [Layer], id: u64) -> Option<&'a Layer> {
    for l in layers {
        if l.id.0 == id {
            return Some(l);
        }
        if let Some(children) = l.children()
            && let Some(found) = find_layer(children, id)
        {
            return Some(found);
        }
    }
    None
}

fn find_layer_mut<'a>(layers: &'a mut [Layer], id: u64) -> Option<&'a mut Layer> {
    for l in layers {
        if l.id.0 == id {
            return Some(l);
        }
        if let Some(children) = l.children_mut()
            && let Some(found) = find_layer_mut(children, id)
        {
            return Some(found);
        }
    }
    None
}

/// 返回包含 `id` 的顶层条目下标：组内层也能定位到它最外层的祖先。
fn top_index_containing(layers: &[Layer], id: u64) -> Option<usize> {
    layers.iter().position(|l| find_layer(std::slice::from_ref(l), id).is_some())
}

/// 自动图层名：现有「图层 N」中取最大 N 再加一；没有任何命中时从「图层 1」开始。
fn next_layer_name(layers: &[Layer]) -> String {
    fn walk(layers: &[Layer], max: &mut u64) {
        for l in layers {
            if let Some(rest) = l.name.strip_prefix("图层 ")
                && let Ok(n) = rest.trim().parse::<u64>()
            {
                *max = (*max).max(n);
            }
            if let Some(children) = l.children() {
                walk(children, max);
            }
        }
    }
    let mut max = 0u64;
    walk(layers, &mut max);
    format!("图层 {}", max.saturating_add(1))
}

fn delete_layer(layers: &mut Vec<Layer>, id: u64) -> bool {
    if let Some(pos) = layers.iter().position(|l| l.id.0 == id) {
        layers.remove(pos);
        return true;
    }
    for l in layers.iter_mut() {
        if let Some(children) = l.children_mut()
            && delete_layer(children, id)
        {
            return true;
        }
    }
    false
}

/// (index within its parent vector, parent length) for the layer, searching groups.
fn layer_position(layers: &[Layer], id: u64) -> Option<(usize, usize)> {
    if let Some(pos) = layers.iter().position(|l| l.id.0 == id) {
        return Some((pos, layers.len()));
    }
    for l in layers {
        if let Some(children) = l.children()
            && let Some(found) = layer_position(children, id)
        {
            return Some(found);
        }
    }
    None
}

fn move_layer(layers: &mut Vec<Layer>, id: u64, delta: i32) -> bool {
    if let Some(pos) = layers.iter().position(|l| l.id.0 == id) {
        let target = pos as i32 + delta;
        if target < 0 || target >= layers.len() as i32 {
            return false;
        }
        // remove + insert（而非 swap）：跨步移动时其余图层顺序保持，delta=±1 与 swap 等价。
        let layer = layers.remove(pos);
        layers.insert(target as usize, layer);
        return true;
    }
    for l in layers.iter_mut() {
        if let Some(children) = l.children_mut()
            && move_layer(children, id, delta)
        {
            return true;
        }
    }
    false
}

fn duplicate_layer(layers: &mut Vec<Layer>, id: u64) -> Option<u64> {
    if let Some(pos) = layers.iter().position(|l| l.id.0 == id) {
        let mut copy = layers[pos].duplicate();
        copy.name = format!("{} copy", layers[pos].name);
        let new_id = copy.id.0;
        layers.insert(pos + 1, copy);
        return Some(new_id);
    }
    for l in layers.iter_mut() {
        if let Some(children) = l.children_mut()
            && let Some(new_id) = duplicate_layer(children, id)
        {
            return Some(new_id);
        }
    }
    None
}

fn layer_json(layer: &Layer, depth: u32) -> Value {
    let children: Vec<Value> = layer.children().map(|c| c.iter().rev().map(|l| layer_json(l, depth + 1)).collect()).unwrap_or_default();
    // 非组图层恒为 `true`，使 UI 不必按 `isGroup` 分支读取折叠状态。
    let expanded = match &layer.content {
        LayerContent::Group(group) => group.expanded,
        _ => true,
    };
    json!({
        "id": layer.id.0,
        "name": layer.name,
        "kind": layer.content.kind_name(),
        "visible": layer.visible,
        "opacity": layer.opacity,
        "fillOpacity": layer.fill_opacity,
        "blend": format!("{:?}", layer.blend),
        "clipped": layer.clipped,
        "depth": depth,
        "isGroup": layer.is_group(),
        "expanded": expanded,
        "children": children,
    })
}

fn adjustment_json(adj: &Adjustment) -> Value {
    match adj {
        Adjustment::BrightnessContrast { brightness, contrast, legacy } => {
            json!({ "type": "brightnessContrast", "brightness": brightness, "contrast": contrast, "legacy": legacy })
        }
        Adjustment::Exposure { exposure, offset, gamma } => {
            json!({ "type": "exposure", "exposure": exposure, "offset": offset, "gamma": gamma })
        }
        Adjustment::Vibrance { vibrance, saturation } => {
            json!({ "type": "vibrance", "vibrance": vibrance, "saturation": saturation })
        }
        Adjustment::HueSaturation { hue, saturation, lightness, colorize, .. } => {
            json!({ "type": "hueSaturation", "hue": hue, "saturation": saturation, "lightness": lightness, "colorize": colorize })
        }
        Adjustment::BlackWhite { weights, tint } => json!({ "type": "blackWhite", "weights": weights, "tint": tint }),
        Adjustment::Invert => json!({ "type": "invert" }),
        Adjustment::Posterize { levels } => json!({ "type": "posterize", "levels": levels }),
        Adjustment::Threshold { level } => json!({ "type": "threshold", "level": level }),
        other => json!({ "type": "other", "label": format!("{other:?}") }),
    }
}

fn mode_label(mode: ColorMode) -> &'static str {
    match mode {
        ColorMode::Bitmap => "Bitmap",
        ColorMode::Grayscale => "Grayscale",
        ColorMode::Indexed => "Indexed",
        ColorMode::Rgb => "RGB",
        ColorMode::Cmyk => "CMYK",
        ColorMode::Lab => "Lab",
        ColorMode::Multichannel => "Multichannel",
        ColorMode::Duotone => "Duotone",
    }
}

fn depth_label(depth: SampleType) -> &'static str {
    match depth {
        SampleType::U8 => "8 位/通道",
        SampleType::U16 => "16 位/通道",
        SampleType::F32 => "32 位/通道",
    }
}

fn doc_info(entry: &DocEntry) -> Value {
    let doc = &entry.doc;
    json!({
        "name": doc.name,
        "width": doc.size.width,
        "height": doc.size.height,
        "mode": mode_label(doc.mode),
        "depth": depth_label(doc.depth),
        "resolution": doc.resolution_dpi,
        "layers": doc.layers.iter().rev().map(|l| layer_json(l, 0)).collect::<Vec<_>>(),
        "canUndo": !entry.undo.is_empty(),
        "canRedo": !entry.redo.is_empty(),
    })
}

fn number(params: &Value, key: &str, default: f32) -> f32 {
    params.get(key).and_then(Value::as_f64).map(|v| v as f32).unwrap_or(default)
}

fn build_adjustment(kind: &str, p: &Value) -> Result<Adjustment, String> {
    match kind {
        "brightnessContrast" => Ok(Adjustment::BrightnessContrast {
            brightness: number(p, "brightness", 0.0),
            contrast: number(p, "contrast", 0.0),
            legacy: false,
        }),
        "exposure" => Ok(Adjustment::Exposure {
            exposure: number(p, "exposure", 0.0),
            offset: number(p, "offset", 0.0),
            gamma: number(p, "gamma", 1.0).max(0.01),
        }),
        "vibrance" => Ok(Adjustment::Vibrance {
            vibrance: number(p, "vibrance", 0.0),
            saturation: number(p, "saturation", 0.0),
        }),
        "hueSaturation" => Ok(Adjustment::HueSaturation {
            hue: number(p, "hue", 0.0),
            saturation: number(p, "saturation", 0.0),
            lightness: number(p, "lightness", 0.0),
            colorize: p.get("colorize").and_then(Value::as_bool).unwrap_or(false),
            ranges: HueRange::defaults(),
        }),
        "blackWhite" => Ok(Adjustment::BlackWhite {
            weights: [0.4, 0.6, 0.4, 0.6, 0.4, 0.6],
            tint: None,
        }),
        "invert" => Ok(Adjustment::Invert),
        "posterize" => Ok(Adjustment::Posterize { levels: number(p, "levels", 4.0).clamp(2.0, 255.0) as u32 }),
        "threshold" => Ok(Adjustment::Threshold { level: number(p, "level", 0.5).clamp(0.0, 1.0) }),
        other => Err(format!("unsupported adjustment: {other}")),
    }
}

fn layer_name(kind: &str) -> &'static str {
    match kind {
        "brightnessContrast" => "Brightness/Contrast",
        "exposure" => "Exposure",
        "vibrance" => "Vibrance",
        "hueSaturation" => "Hue/Saturation",
        "blackWhite" => "Black & White",
        "invert" => "Invert",
        "posterize" => "Posterize",
        "threshold" => "Threshold",
        _ => "Adjustment",
    }
}

fn preview_bytes(img: Rgba8Image) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + img.pixels.len());
    out.extend_from_slice(&img.width.to_le_bytes());
    out.extend_from_slice(&img.height.to_le_bytes());
    out.extend_from_slice(&img.pixels);
    out
}

fn buffer_bytes(buf: Buffer) -> Vec<u8> {
    preview_bytes(buf.to_rgba8())
}

/// Creates a session. Every session owns its documents and history.
#[unsafe(no_mangle)]
pub extern "C" fn pc_session_new() -> *mut Session {
    Box::into_raw(Box::new(Session { docs: HashMap::new(), next_id: 0 }))
}

/// Frees a session and every document it holds.
#[unsafe(no_mangle)]
pub extern "C" fn pc_session_free(ptr: *mut Session) {
    if !ptr.is_null() {
        // SAFETY: created by `pc_session_new`, freed once.
        drop(unsafe { Box::from_raw(ptr) });
    }
}

/// Opens an image from memory. Returns `{"ok":true,"data":{"id":..,"info":{..}}}`.
#[unsafe(no_mangle)]
pub extern "C" fn pc_open(session: *mut Session, name_ptr: *const u8, name_len: usize, data_ptr: *const u8, data_len: usize) -> *mut PcBuffer {
    guard(move || {
        if session.is_null() {
            return fail("null session");
        }
        let name = match unsafe { slice(name_ptr, name_len) } {
            Ok(b) => String::from_utf8_lossy(b).to_string(),
            Err(e) => return fail(e),
        };
        let data = match unsafe { slice(data_ptr, data_len) } {
            Ok(b) => b,
            Err(e) => return fail(e),
        };
        match photocraft_io::import(&name, data) {
            Ok(result) => {
                // SAFETY: session is a live pointer from `pc_session_new`.
                let s = unsafe { &mut *session };
                let warnings = result.warnings.clone();
                let id = s.add(result.document, &name);
                let info = s.docs.get(&id).map(doc_info).unwrap_or(Value::Null);
                ok(json!({ "id": id, "warnings": warnings, "info": info }))
            }
            Err(e) => fail(format!("{e}")),
        }
    })
}

/// 新建空白文档（桌面版 `file.new` 的鸿蒙子集）：RGB / 8 位、单个白色 Background 层。
/// 尺寸上限与 doc 层一致（单边 ≤ `MAX_CANVAS_SIDE`、总像素 ≤ `MAX_CANVAS_PIXELS`）。
/// 返回 `{"id":id,"warnings":[],"info":{..}}`，与 `pc_open` 同形，UI 复用同一套装载流程。
#[unsafe(no_mangle)]
pub extern "C" fn pc_document_new(session: *mut Session, width: u32, height: u32, name_ptr: *const u8, name_len: usize) -> *mut PcBuffer {
    guard(move || {
        if session.is_null() {
            return fail("null session");
        }
        if width == 0 || height == 0 {
            return fail("document size must be positive");
        }
        if width > photocraft_doc::canvas::MAX_CANVAS_SIDE || height > photocraft_doc::canvas::MAX_CANVAS_SIDE {
            return fail(format!("document side exceeds {}", photocraft_doc::canvas::MAX_CANVAS_SIDE));
        }
        if u64::from(width) * u64::from(height) > photocraft_doc::canvas::MAX_CANVAS_PIXELS {
            return fail("document is too large");
        }
        let name_bytes = match unsafe { slice(name_ptr, name_len) } {
            Ok(b) => b,
            Err(e) => return fail(e),
        };
        let name = match std::str::from_utf8(name_bytes) {
            Ok(v) => v.trim(),
            Err(_) => return fail("document name must be UTF-8"),
        };
        let name = if name.is_empty() { "Untitled" } else { name };
        let (w, h) = (i32::try_from(width).unwrap_or(0), i32::try_from(height).unwrap_or(0));
        let mut doc = Document::new(name, photocraft_geom::Size::new(width, height), ColorMode::Rgb, SampleType::U8);
        let mut background = Layer::raster("Background", doc.pixel_format());
        if let Some(surface) = background.surface_mut() {
            surface.fill_rect(Rect::new(0, 0, w, h), &[1.0, 1.0, 1.0, 1.0]);
        }
        doc.layers.push(background);
        // SAFETY: session is a live pointer from `pc_session_new`.
        let s = unsafe { &mut *session };
        let id = s.add(doc, name);
        let info = s.docs.get(&id).map(doc_info).unwrap_or(Value::Null);
        ok(json!({ "id": id, "warnings": [], "info": info }))
    })
}


// ===== 画布旋转/翻转（bridge 自包含；engine 的 canvas_geom 不在本 workspace）=====

#[derive(Clone, Copy)]
enum CanvasTurn {
    FlipHorizontal,
    FlipVertical,
    Rotate180,
    Cw90,
    Ccw90,
}

impl CanvasTurn {
    fn parse(code: u32) -> Option<Self> {
        match code {
            0 => Some(Self::FlipHorizontal),
            1 => Some(Self::FlipVertical),
            2 => Some(Self::Rotate180),
            3 => Some(Self::Cw90),
            4 => Some(Self::Ccw90),
            _ => None,
        }
    }

    fn pixels(self, x: i32, y: i32, w: i32, h: i32) -> (i32, i32) {
        match self {
            Self::FlipHorizontal => (w - 1 - x, y),
            Self::FlipVertical => (x, h - 1 - y),
            Self::Rotate180 => (w - 1 - x, h - 1 - y),
            Self::Cw90 => (h - 1 - y, x),
            Self::Ccw90 => (y, w - 1 - x),
        }
    }

    /// 与 `pixels()` 同一映射的连续坐标（像素边缘）仿射矩阵，用于变换矢量/文字/智能对象几何。
    fn affine(self, w: f64, h: f64) -> Affine {
        Affine {
            m: match self {
                Self::FlipHorizontal => [-1.0, 0.0, 0.0, 1.0, w, 0.0],
                Self::FlipVertical => [1.0, 0.0, 0.0, -1.0, 0.0, h],
                Self::Rotate180 => [-1.0, 0.0, 0.0, -1.0, w, h],
                Self::Cw90 => [0.0, 1.0, -1.0, 0.0, h, 0.0],
                Self::Ccw90 => [0.0, -1.0, 1.0, 0.0, 0.0, w],
            },
        }
    }

    fn swaps_axes(self) -> bool {
        matches!(self, Self::Cw90 | Self::Ccw90)
    }
}

/// 按 `turn` 重映射一个像素表面（只遍历 content bounds，边界外不分配）。
fn remap_surface(s: &Surface, turn: CanvasTurn, w: i32, h: i32) -> Surface {
    let r = s.content_bounds();
    let mut out = Surface::with_default(s.format(), &s.default_pixel());
    let n = s.channels().max(1);
    let raw = s.read_region(r);
    let width = r.width() as usize;
    for (i, px) in raw.chunks_exact(n).enumerate() {
        let (x, y) = (r.x0 + (i % width) as i32, r.y0 + (i / width) as i32);
        let (nx, ny) = turn.pixels(x, y, w, h);
        out.write_pixel(nx, ny, px);
    }
    out
}

const TURN_EPS: f64 = 1e-9;

fn turn_map_pt(a: &Affine, p: [f64; 2]) -> [f64; 2] {
    let q = a.apply(photocraft_geom::Point::new(p[0], p[1]));
    [q.x, q.y]
}

fn turn_map_box(a: &Affine, b: [f64; 4]) -> [f64; 4] {
    let c = [[b[0], b[1]], [b[2], b[1]], [b[2], b[3]], [b[0], b[3]]].map(|p| turn_map_pt(a, p));
    c.iter().fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |r, p| [r[0].min(p[0]), r[1].min(p[1]), r[2].max(p[0]), r[3].max(p[1])])
}

fn turn_map_rect(a: &Affine, r: Rect) -> Rect {
    if r.is_empty() {
        return r;
    }
    let b = turn_map_box(a, [f64::from(r.x0), f64::from(r.y0), f64::from(r.x1), f64::from(r.y1)]);
    let i = |v: f64| v.round().clamp(f64::from(i32::MIN / 2), f64::from(i32::MAX / 2)) as i32;
    Rect::new(i(b[0]), i(b[1]), i(b[2]), i(b[3]))
}

/// 渐变角度（屏幕逆时针度数）随 `a` 的线性部分旋转；整度保持整度（90° 变换不引入舍入噪声）。
fn turn_map_angle(a: &Affine, deg: f32) -> f32 {
    let (s, c) = f64::from(deg).to_radians().sin_cos();
    let (dx, dy) = (c, -s);
    let (nx, ny) = (a.m[0] * dx + a.m[2] * dy, a.m[1] * dx + a.m[3] * dy);
    if nx.abs() < TURN_EPS && ny.abs() < TURN_EPS {
        return deg;
    }
    let out = (-ny).atan2(nx).to_degrees();
    let out = if (out - out.round()).abs() < 1e-6 { out.round() } else { out };
    (if out <= -180.0 { out + 360.0 } else { out }) as f32
}

fn turn_map_fill(a: &Affine, f: &mut Fill) {
    if let Fill::Gradient { angle, offset, .. } = f {
        *angle = turn_map_angle(a, *angle);
        let unit = |x: f64, y: f64| {
            let n = x.hypot(y);
            if n < TURN_EPS { (x, y) } else { (x / n, y / n) }
        };
        let ((m0, m1), (m2, m3)) = (unit(a.m[0], a.m[1]), unit(a.m[2], a.m[3]));
        let (ox, oy) = (f64::from(offset.0), f64::from(offset.1));
        *offset = ((m0 * ox + m2 * oy) as f32, (m1 * ox + m3 * oy) as f32);
    }
}

/// 非像素几何：矢量蒙版、样式参考点、填充渐变、形状路径、文字/智能对象变换、组画板。
///
/// 形状的 `live` 参数降级为 `None`：路径已精确变换，但参数化编辑能力会丢失。engine 在非轴对齐
/// 变换时也做同样降级；90°/翻转虽然轴对齐，这里统一降级以保持实现单一且不产生不一致参数。
fn turn_layer_geometry(l: &mut Layer, a: &Affine) {
    if let Some(vm) = l.vector_mask.as_mut() {
        vm.path = vm.path.transform(a);
    }
    if let Some((x, y)) = l.effects.reference {
        let p = turn_map_pt(a, [x, y]);
        l.effects.reference = Some((p[0], p[1]));
    }
    // 填充缓存的渐变参数即将改变，旧缓存必须重渲染。
    l.fill_cache = None;
    match &mut l.content {
        LayerContent::Fill(f) => turn_map_fill(a, f),
        LayerContent::Shape(sh) => {
            if let Some(f) = &mut sh.fill {
                turn_map_fill(a, f);
            }
            if let Some(st) = &mut sh.stroke {
                turn_map_fill(a, &mut st.paint);
            }
            sh.path = sh.path.transform(a);
            sh.live = None;
        }
        LayerContent::Text(t) => t.transform = a.mul(&t.transform),
        LayerContent::Smart(sm) => sm.transform = a.mul(&sm.transform),
        LayerContent::Group(g) => {
            if let Some(ab) = &mut g.artboard {
                ab.rect = turn_map_rect(a, ab.rect);
            }
            for c in &mut g.children {
                turn_layer_geometry(c, a);
            }
        }
        _ => {}
    }
}


fn turn_layer_pixels(layers: &mut [Layer], turn: CanvasTurn, w: i32, h: i32) {
    for l in layers {
        match &mut l.content {
            LayerContent::Raster(s) => *s = remap_surface(s, turn, w, h),
            LayerContent::Text(t) => {
                if let Some(cache) = t.cache.as_mut() {
                    *cache = remap_surface(cache, turn, w, h);
                }
            }
            LayerContent::Shape(sh) => {
                if let Some(cache) = sh.cache.as_mut() {
                    *cache = remap_surface(cache, turn, w, h);
                }
            }
            LayerContent::Smart(sm) => {
                if let Some(cache) = sm.cache.as_mut() {
                    *cache = remap_surface(cache, turn, w, h);
                }
                if let Some(fm) = sm.filter_mask.as_mut() {
                    fm.surface = remap_surface(&fm.surface, turn, w, h);
                }
            }
            LayerContent::Group(g) => turn_layer_pixels(&mut g.children, turn, w, h),
            _ => {}
        }
        if let Some(mask) = l.mask.as_mut() {
            mask.surface = remap_surface(&mask.surface, turn, w, h);
        }
        // 填充缓存按旧布局渲染，轴交换后必须重渲染而不是复用。
        l.fill_cache = None;
        if let Some(video) = l.video.as_mut() {
            for frame in video.frames.iter_mut().chain(video.original_frames.iter_mut().flatten()) {
                *frame = remap_surface(frame, turn, w, h);
            }
        }
    }
}

fn turn_document(doc: &mut Document, turn: CanvasTurn) {
    let (w, h) = (i32::try_from(doc.size.width).unwrap_or(0), i32::try_from(doc.size.height).unwrap_or(0));
    // 仿射矩阵用**旧**宽高构造：像素映射与几何映射必须基于同一坐标系。
    let a = turn.affine(f64::from(w), f64::from(h));
    turn_layer_pixels(&mut doc.layers, turn, w, h);
    for layer in &mut doc.layers {
        turn_layer_geometry(layer, &a);
    }
    for named in &mut doc.paths {
        named.path = named.path.transform(&a);
    }
    if let Some(work) = doc.work_path.as_mut() {
        *work = work.transform(&a);
    }
    if let Some(selection) = doc.selection.take() {
        doc.selection = Some(remap_surface(&selection, turn, w, h));
    }
    if let Some(quick) = doc.quick_mask.as_mut() {
        quick.surface = remap_surface(&quick.surface, turn, w, h);
    }
    for channel in &mut doc.channels {
        channel.surface = remap_surface(&channel.surface, turn, w, h);
    }
    if turn.swaps_axes() {
        doc.size = photocraft_doc::Size::new(doc.size.height, doc.size.width);
    }
}

/// 画布旋转/翻转（Image › Image Rotation）：`turn` 0=水平翻转、1=垂直翻转、2=180°、3=顺时针 90°、4=逆时针 90°。
/// 像素（含文字/形状/智能对象的渲染缓存）、蒙版、通道、Quick Mask 与选区精确重映射；文字/形状/智能对象/
/// 矢量蒙版的几何通过同一仿射矩阵变换（形状的 `live` 参数降级为 None）。90° 系列交换宽高。单 checkpoint。
#[unsafe(no_mangle)]
pub extern "C" fn pc_document_turn(session: *mut Session, id: u64, turn: u32) -> *mut PcBuffer {
    guard(move || {
        if session.is_null() {
            return fail("null session");
        }
        let kind = match CanvasTurn::parse(turn) {
            Some(k) => k,
            None => return fail("invalid turn: expected 0..=4"),
        };
        // SAFETY: live session pointer.
        let s = unsafe { &mut *session };
        let entry = match s.entry_mut(id) {
            Ok(e) => e,
            Err(e) => return fail(e),
        };

        entry.checkpoint_named("旋转/翻转");
        turn_document(&mut entry.doc, kind);
        ok(json!({ "info": doc_info(entry) }))
    })
}

/// Closes a document.
#[unsafe(no_mangle)]
pub extern "C" fn pc_close(session: *mut Session, id: u64) -> *mut PcBuffer {
    guard(move || {
        if session.is_null() {
            return fail("null session");
        }
        // SAFETY: live session pointer.
        let s = unsafe { &mut *session };
        s.docs.remove(&id);
        ok(Value::Null)
    })
}

/// Metadata and layer tree of one document.
#[unsafe(no_mangle)]
pub extern "C" fn pc_info(session: *mut Session, id: u64) -> *mut PcBuffer {
    guard(move || {
        if session.is_null() {
            return fail("null session");
        }
        // SAFETY: live session pointer.
        let s = unsafe { &*session };
        match s.entry(id) {
            Ok(entry) => ok(doc_info(entry)),
            Err(e) => fail(e),
        }
    })
}

/// Renders a downscaled preview into `out` as `[width u32][height u32][RGBA8]`.
#[unsafe(no_mangle)]
pub extern "C" fn pc_preview(session: *mut Session, id: u64, max_side: u32, out: *mut *mut PcBuffer) -> *mut PcBuffer {
    guard(move || {
        if session.is_null() {
            return fail("null session");
        }
        // SAFETY: live session pointer.
        let s = unsafe { &*session };
        let entry = match s.entry(id) {
            Ok(e) => e,
            Err(e) => return fail(e),
        };
        let side = max_side.clamp(1, MAX_PREVIEW_SIDE);
        let img = photocraft_compose::thumbnail(&entry.doc, side);
        if !out.is_null() {
            // SAFETY: out is a caller-provided slot.
            unsafe { *out = PcBuffer::from_vec(preview_bytes(img)) };
        }
        ok(json!({ "width": side, "height": side }))
    })
}

/// Renders a small RGBA thumbnail of a pixel/shape/text layer into `out` (same 8-byte
/// width/height header as `pc_preview`). Scales to fit `side` without upscaling.
#[unsafe(no_mangle)]
pub extern "C" fn pc_layer_thumbnail(session: *mut Session, id: u64, layer: u64, side: u32, out: *mut *mut PcBuffer) -> *mut PcBuffer {
    guard(move || {
        if session.is_null() {
            return fail("null session");
        }
        // SAFETY: live session pointer.
        let s = unsafe { &*session };
        let entry = match s.entry(id) {
            Ok(e) => e,
            Err(e) => return fail(e),
        };
        let Some(target) = find_layer(&entry.doc.layers, layer) else {
            return fail("layer not found");
        };
        let Some(surface) = target.surface() else {
            return fail("layer has no pixels to thumbnail");
        };
        let bounds = surface.content_bounds().intersect(&entry.doc.bounds());
        if bounds.is_empty() {
            return fail("layer is empty");
        }
        let max_side = side.clamp(16, 256) as f64;
        let bounds_w = bounds.width() as f64;
        let bounds_h = bounds.height() as f64;
        let scale = (max_side / bounds_w).min(max_side / bounds_h).min(1.0);
        let thumb_w = ((bounds_w * scale).round() as u32).max(1);
        let thumb_h = ((bounds_h * scale).round() as u32).max(1);
        let format = surface.format();
        let channels = format.channels();
        let src_w = bounds.width() as usize;
        let src_h = bounds.height() as usize;
        let quant = |v: f32| -> u8 { (v.clamp(0.0, 1.0) * 255.0).round() as u8 };
        let mut img = Rgba8Image::new(thumb_w, thumb_h);
        let mut row: Vec<f32> = Vec::new();
        for y in 0..thumb_h as usize {
            let sy = (y * src_h / thumb_h as usize).min(src_h - 1);
            surface.read_region_into(
                Rect::new(bounds.x0, bounds.y0 + sy as i32, bounds.x1, bounds.y0 + sy as i32 + 1),
                &mut row,
            );
            for x in 0..thumb_w as usize {
                let sx = (x * src_w / thumb_w as usize).min(src_w - 1);
                let px = to_rgba(&format, &row[sx * channels..(sx + 1) * channels]);
                let o = (y * thumb_w as usize + x) * 4;
                img.pixels[o] = quant(px[0]);
                img.pixels[o + 1] = quant(px[1]);
                img.pixels[o + 2] = quant(px[2]);
                img.pixels[o + 3] = quant(px[3]);
            }
        }
        if !out.is_null() {
            // SAFETY: out is a caller-provided slot.
            unsafe { *out = PcBuffer::from_vec(preview_bytes(img)) };
        }
        ok(json!({ "width": thumb_w, "height": thumb_h }))
    })
}

/// Renders a 1:1 region of the document into `out`.
#[unsafe(no_mangle)]
pub extern "C" fn pc_render(session: *mut Session, id: u64, x: i32, y: i32, w: i32, h: i32, out: *mut *mut PcBuffer) -> *mut PcBuffer {
    guard(move || {
        if session.is_null() {
            return fail("null session");
        }
        if w <= 0 || h <= 0 {
            return fail("invalid render rectangle");
        }
        if u64::from(w as u32) * u64::from(h as u32) > MAX_RENDER_PIXELS {
            return fail("render rectangle is too large");
        }
        // SAFETY: live session pointer.
        let s = unsafe { &*session };
        let entry = match s.entry(id) {
            Ok(e) => e,
            Err(e) => return fail(e),
        };
        let rect = Rect::new(x, y, x.saturating_add(w), y.saturating_add(h));
        let buf = photocraft_compose::render(&entry.doc, rect);
        if !out.is_null() {
            // SAFETY: out is a caller-provided slot.
            unsafe { *out = PcBuffer::from_vec(buffer_bytes(buf)) };
        }
        ok(json!({ "width": w, "height": h }))
    })
}

/// Renders a 1:1 region into `out` as bare RGBA8 pixels (`w * h * 4` bytes, no size header).
/// The ArkTS side feeds the bytes straight to `PixelMap`, so the byte count must match exactly.
#[unsafe(no_mangle)]
pub extern "C" fn pc_render_pixels(session: *mut Session, id: u64, x: i32, y: i32, w: i32, h: i32, out: *mut *mut PcBuffer) -> *mut PcBuffer {
    guard(move || {
        if session.is_null() {
            return fail("null session");
        }
        if w <= 0 || h <= 0 {
            return fail("invalid render rectangle");
        }
        if u64::from(w as u32) * u64::from(h as u32) > MAX_RENDER_PIXELS {
            return fail("render rectangle is too large");
        }
        // SAFETY: live session pointer.
        let s = unsafe { &*session };
        let entry = match s.entry(id) {
            Ok(e) => e,
            Err(e) => return fail(e),
        };
        let rect = Rect::new(x, y, x.saturating_add(w), y.saturating_add(h));
        let img = photocraft_compose::render(&entry.doc, rect).to_rgba8();
        // `compose` returns an empty image when a rectangle is too large to allocate; a size
        // mismatch means the bytes cannot back a `PixelMap` of the requested region.
        if img.width != w as u32 || img.height != h as u32 {
            return fail(format!("render returned {}x{} for {}x{}", img.width, img.height, w, h));
        }
        if !out.is_null() {
            // SAFETY: out is a caller-provided slot.
            unsafe { *out = PcBuffer::from_vec(img.pixels) };
        }
        ok(json!({ "width": w, "height": h }))
    })
}

/// Document version: changes whenever a successful operation may have changed the document
/// (`checkpoint`, undo, redo).
#[unsafe(no_mangle)]
pub extern "C" fn pc_revision(session: *mut Session, id: u64) -> *mut PcBuffer {
    guard(move || {
        if session.is_null() {
            return fail("null session");
        }
        // SAFETY: live session pointer.
        let s = unsafe { &*session };
        match s.entry(id) {
            Ok(entry) => ok(json!({ "revision": entry.revision })),
            Err(e) => fail(e),
        }
    })
}

/// Paints a brush stroke. Reuses `layer` when it is a pixel layer, otherwise creates a new pixel
/// layer on top. `params` is a JSON [`BrushSettings`] and `points` a JSON array of
/// [`StrokePoint`]. Returns `{"ok":true,"data":{"layerId":..,"rect":{..}}}`.
#[unsafe(no_mangle)]
pub extern "C" fn pc_paint_stroke(
    session: *mut Session,
    id: u64,
    layer: u64,
    params_ptr: *const u8,
    params_len: usize,
    points_ptr: *const u8,
    points_len: usize,
) -> *mut PcBuffer {
    guard(move || {
        if session.is_null() {
            return fail("null session");
        }
        let params = match unsafe { slice(params_ptr, params_len) } {
            Ok(b) => b,
            Err(e) => return fail(e),
        };
        let raw_points = match unsafe { slice(points_ptr, points_len) } {
            Ok(b) => b,
            Err(e) => return fail(e),
        };
        let brush = match serde_json::from_slice::<BrushSettings>(params) {
            Ok(b) => b.bounded_for_render(),
            Err(e) => return fail(format!("invalid brush parameters: {e}")),
        };
        let points = match serde_json::from_slice::<Vec<StrokePoint>>(raw_points) {
            Ok(p) => p,
            Err(e) => return fail(format!("invalid stroke points: {e}")),
        };
        if points.is_empty() {
            return fail("stroke has no points");
        }
        // SAFETY: live session pointer.
        let s = unsafe { &mut *session };
        let entry = match s.docs.get_mut(&id) {
            Some(e) => e,
            None => return fail(format!("document {id} is not open")),
        };
        let selection = entry.doc.selection.clone();
        let format = entry.doc.pixel_format();
        let reusable = layer != 0 && find_layer(&entry.doc.layers, layer).is_some_and(|l| matches!(&l.content, LayerContent::Raster(_)));
        let target = if reusable {
            layer
        } else {
            let name = format!("Paint {}", entry.doc.layers.len() + 1);
            let created = Layer::new(name, LayerContent::Raster(Surface::new(format)));
            let new_id = created.id.0;
            entry.doc.layers.push(created);
            new_id
        };
        entry.checkpoint_named("画笔");
        let Some(l) = find_layer_mut(&mut entry.doc.layers, target) else {
            return fail("layer not found");
        };
        let lock_transparency = l.locks.transparency;
        let surface = match &mut l.content {
            LayerContent::Raster(s) => s,
            _ => return fail("paint target is not a pixel layer"),
        };
        let stroke = Stroke { brush, points };
        let rect = apply_stroke(surface, &stroke, selection.as_ref(), lock_transparency);
        ok(json!({
            "layerId": target,
            "rect": { "x0": rect.x0, "y0": rect.y0, "x1": rect.x1, "y1": rect.y1 },
        }))
    })
}

#[derive(Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct SelectParams {
    kind: String,
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
    mode: String,
    feather: f32,
    points: Vec<[f64; 2]>,
    x: i32,
    y: i32,
    tolerance: f32,
    contiguous: bool,
}

impl Default for SelectParams {
    fn default() -> Self {
        Self {
            kind: "rect".into(),
            x0: 0.0,
            y0: 0.0,
            x1: 0.0,
            y1: 0.0,
            mode: "replace".into(),
            feather: 0.0,
            points: Vec::new(),
            x: 0,
            y: 0,
            tolerance: 32.0,
            contiguous: true,
        }
    }
}

fn selection_json(doc: &Document) -> Value {
    let area = doc.bounds();
    let bounds = doc
        .selection
        .as_ref()
        .map(|sel| {
            let default = sel.default_pixel().first().copied().unwrap_or(0.0);
            if default > 0.0 { area } else { sel.content_bounds().intersect(&area) }
        })
        .filter(|b| !b.is_empty());
    match bounds {
        Some(b) => json!({ "active": true, "x0": b.x0, "y0": b.y0, "x1": b.x1, "y1": b.y1 }),
        None => json!({ "active": false, "x0": 0, "y0": 0, "x1": 0, "y1": 0 }),
    }
}

fn select_rect(p: &SelectParams, area: Rect) -> Rect {
    let (x0, x1) = if p.x0 <= p.x1 { (p.x0, p.x1) } else { (p.x1, p.x0) };
    let (y0, y1) = if p.y0 <= p.y1 { (p.y0, p.y1) } else { (p.y1, p.y0) };
    let clamp = |v: f64, lo: i32, hi: i32| -> i32 { v.clamp(lo as f64, hi as f64) as i32 };
    Rect::new(clamp(x0.floor(), area.x0, area.x1), clamp(y0.floor(), area.y0, area.y1), clamp(x1.ceil(), area.x0, area.x1), clamp(y1.ceil(), area.y0, area.y1))
}

/// Creates, combines or clears the document selection. `params` is a JSON object with
/// `kind` = `rect|ellipse|lasso|wand|all|invert|none`. Returns the selection bounds.
#[unsafe(no_mangle)]
pub extern "C" fn pc_select(session: *mut Session, id: u64, params_ptr: *const u8, params_len: usize) -> *mut PcBuffer {
    guard(move || {
        if session.is_null() {
            return fail("null session");
        }
        let params = match unsafe { slice(params_ptr, params_len) } {
            Ok(b) => b,
            Err(e) => return fail(e),
        };
        let p = match serde_json::from_slice::<SelectParams>(params) {
            Ok(p) => p,
            Err(e) => return fail(format!("invalid selection parameters: {e}")),
        };
        // SAFETY: live session pointer.
        let s = unsafe { &mut *session };
        let entry = match s.docs.get_mut(&id) {
            Some(e) => e,
            None => return fail(format!("document {id} is not open")),
        };
        let area = entry.doc.bounds();
        if area.is_empty() {
            return fail("document is empty");
        }
        let kind = p.kind.as_str();
        if kind == "none" {
            entry.checkpoint();
            entry.doc.selection = None;
            return ok(selection_json(&entry.doc));
        }
        let (w, h) = (area.width() as usize, area.height() as usize);
        let n = w.checked_mul(h).unwrap_or(0);
        if n == 0 {
            return fail("document is empty");
        }
        let mut mask: Vec<f32> = match kind {
            "rect" => {
                let r = select_rect(&p, area);
                let mut m = vec![0.0f32; n];
                for y in r.y0..r.y1 {
                    let row = (y - area.y0) as usize * w;
                    for x in r.x0..r.x1 {
                        m[row + (x - area.x0) as usize] = 1.0;
                    }
                }
                m
            }
            "ellipse" => {
                let shape = selection::ellipse_surface(select_rect(&p, area), area, true);
                selection::mask_from_surface(Some(&shape), area)
            }
            "lasso" => {
                let pts: Vec<(f32, f32)> = p.points.iter().map(|q| (q[0] as f32, q[1] as f32)).collect();
                selection::polygon(&pts, area, true)
            }
            "wand" => {
                if !area.contains(p.x, p.y) {
                    return fail("wand seed is outside the canvas");
                }
                let img = photocraft_compose::render(&entry.doc, area);
                selection::magic_wand(&img.px, area, (p.x, p.y), p.tolerance.clamp(0.0, 255.0), p.contiguous, true)
            }
            "all" => vec![1.0; n],
            "invert" => {
                let old = selection::mask_from_surface(entry.doc.selection.as_ref(), area);
                old.iter().map(|v| 1.0 - v).collect()
            }
            other => return fail(format!("unsupported selection kind: {other}")),
        };
        if mask.len() != n {
            return fail("selection mask has the wrong size");
        }
        if kind != "invert" {
            let radius = p.feather.clamp(0.0, 250.0);
            if radius > 0.0 {
                mask = selection::feather(&mask, w, h, radius);
            }
        }
        let mode = if kind == "invert" || kind == "all" { SelectionMode::Replace } else { SelectionMode::parse(&p.mode) };
        entry.checkpoint();
        let old = entry.doc.selection.clone();
        entry.doc.selection = selection::combine(old.as_ref(), &mask, area, mode);
        ok(selection_json(&entry.doc))
    })
}

/// Applies an `algo::FilterParams` JSON filter to a pixel layer. The selection is used as a mask,
/// so with an active selection only the selected pixels change. Returns the updated document info.
#[unsafe(no_mangle)]
pub extern "C" fn pc_filter_apply(session: *mut Session, id: u64, layer: u64, params_ptr: *const u8, params_len: usize) -> *mut PcBuffer {
    guard(move || {
        if session.is_null() {
            return fail("null session");
        }
        let params = match unsafe { slice(params_ptr, params_len) } {
            Ok(b) => b,
            Err(e) => return fail(e),
        };
        let filter = match serde_json::from_slice::<FilterParams>(params) {
            Ok(p) => p,
            Err(e) => return fail(format!("invalid filter parameters: {e}")),
        };
        // SAFETY: live session pointer.
        let s = unsafe { &mut *session };
        let entry = match s.docs.get_mut(&id) {
            Some(e) => e,
            None => return fail(format!("document {id} is not open")),
        };
        let bounds = entry.doc.bounds();
        if bounds.is_empty() {
            return fail("document is empty");
        }
        let Some(target) = find_layer(&entry.doc.layers, layer) else {
            return fail("layer not found");
        };
        if target.locks.pixels || target.locks.all {
            return fail("layer pixels are locked");
        }
        let content = match &target.content {
            LayerContent::Raster(surface) => surface.content_bounds().intersect(&bounds),
            _ => return fail("filter requires a pixel layer"),
        };
        let selection = entry.doc.selection.clone();
        let selection_bounds = selection.as_ref().and_then(|sel| {
            let default = sel.default_pixel().first().copied().unwrap_or(0.0);
            if default > 0.0 {
                Some(bounds)
            } else {
                let b = sel.content_bounds().intersect(&bounds);
                (!b.is_empty()).then_some(b)
            }
        });
        let area = output_area(&filter, content, bounds, selection_bounds);
        if area.is_empty() {
            return ok(doc_info(entry));
        }
        entry.checkpoint_named("滤镜");
        let Some(target_mut) = find_layer_mut(&mut entry.doc.layers, layer) else {
            return fail("layer not found");
        };
        let LayerContent::Raster(surface) = &mut target_mut.content else {
            return fail("filter requires a pixel layer");
        };
        let result = apply_in(surface, &filter, area, bounds, selection.as_ref(), bounds);
        *surface = result;
        ok(doc_info(entry))
    })
}

#[derive(Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct ShapeParams {
    kind: String,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    from_x: f64,
    from_y: f64,
    to_x: f64,
    to_y: f64,
    weight: f64,
    radius: f64,
    fill: [f32; 4],
    stroke: [f32; 4],
    stroke_width: f32,
    name: String,
}

impl Default for ShapeParams {
    fn default() -> Self {
        Self {
            kind: "rect".into(),
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0,
            from_x: 0.0,
            from_y: 0.0,
            to_x: 0.0,
            to_y: 0.0,
            weight: 12.0,
            radius: 0.0,
            fill: [0.9, 0.2, 0.3, 1.0],
            stroke: [1.0, 1.0, 1.0, 0.0],
            stroke_width: 0.0,
            name: String::new(),
        }
    }
}

fn shape_rect(p: &ShapeParams) -> [f64; 4] {
    let x = p.x.min(p.x + p.w);
    let y = p.y.min(p.y + p.h);
    [x, y, p.w.abs(), p.h.abs()]
}

fn shape_color(c: [f32; 4]) -> Color {
    Color::rgba(c[0].clamp(0.0, 1.0), c[1].clamp(0.0, 1.0), c[2].clamp(0.0, 1.0), c[3].clamp(0.0, 1.0))
}

/// Creates a live rectangle/ellipse/line shape layer on top of the document. `params` is JSON
/// with `kind`, geometry and straight RGBA colours. Returns `{"layerId":..,"info":{..}}`.
#[unsafe(no_mangle)]
pub extern "C" fn pc_shape_create(session: *mut Session, id: u64, params_ptr: *const u8, params_len: usize) -> *mut PcBuffer {
    guard(move || {
        if session.is_null() {
            return fail("null session");
        }
        let params = match unsafe { slice(params_ptr, params_len) } {
            Ok(b) => b,
            Err(e) => return fail(e),
        };
        let p = match serde_json::from_slice::<ShapeParams>(params) {
            Ok(p) => p,
            Err(e) => return fail(format!("invalid shape parameters: {e}")),
        };
        // SAFETY: live session pointer.
        let s = unsafe { &mut *session };
        let entry = match s.docs.get_mut(&id) {
            Some(e) => e,
            None => return fail(format!("document {id} is not open")),
        };
        let bounds = entry.doc.bounds();
        if bounds.is_empty() {
            return fail("document is empty");
        }
        let live = match p.kind.as_str() {
            "rect" => {
                let rect = shape_rect(&p);
                let max_radius = (rect[2].min(rect[3]) / 2.0).max(0.0);
                let radius = p.radius.clamp(0.0, max_radius);
                LiveShape::Rect { rect, radii: [radius, radius, radius, radius] }
            }
            "ellipse" => LiveShape::Ellipse { rect: shape_rect(&p) },
            "line" => LiveShape::Line {
                from: [p.from_x, p.from_y],
                to: [p.to_x, p.to_y],
                weight: p.weight.clamp(1.0, 10000.0),
            },
            other => return fail(format!("unsupported shape kind: {other}")),
        };
        let path = live_path(&live);
        let fill = if p.fill[3] > 0.0 { Some(Fill::Solid(shape_color(p.fill))) } else { None };
        let stroke = if p.stroke_width > 0.0 && p.stroke[3] > 0.0 {
            Some(ShapeStroke {
                width: p.stroke_width.clamp(0.0, 10000.0),
                paint: Fill::Solid(shape_color([p.stroke[0], p.stroke[1], p.stroke[2], 1.0])),
                opacity: p.stroke[3].clamp(0.0, 1.0),
                ..Default::default()
            })
        } else {
            None
        };
        if let Err(e) = validate_shape(&path, stroke.as_ref()) {
            return fail(e);
        }
        let label = match p.kind.as_str() {
            "ellipse" => "Ellipse",
            "line" => "Line",
            _ => "Rectangle",
        };
        let name = if p.name.trim().is_empty() { format!("{label} {}", entry.doc.layers.len() + 1) } else { p.name.clone() };
        let mut shape = ShapeLayer { path, fill, stroke, live: Some(live), ..Default::default() };
        shape.cache = Some(render_shape(&shape, entry.doc.pixel_format(), bounds));
        entry.checkpoint();
        let layer = Layer::new(name, LayerContent::Shape(shape));
        let layer_id = layer.id.0;
        entry.doc.layers.push(layer);
        ok(json!({ "layerId": layer_id, "info": doc_info(entry) }))
    })
}

static TEXT_ENGINE: OnceLock<Mutex<TextEngine>> = OnceLock::new();
static TEXT_FONT_FAMILIES: OnceLock<Vec<String>> = OnceLock::new();

fn text_engine() -> &'static Mutex<TextEngine> {
    TEXT_ENGINE.get_or_init(|| Mutex::new(TextEngine::new()))
}

/// Registers a system font (TTF/OTF/TTC/OTC bytes) with the shared text engine. Returns
/// `{"families":[..]}`; later calls return the families registered by the first one.
#[unsafe(no_mangle)]
pub extern "C" fn pc_font_register(data_ptr: *const u8, data_len: usize) -> *mut PcBuffer {
    guard(move || {
        let bytes = match unsafe { slice(data_ptr, data_len) } {
            Ok(b) => b,
            Err(e) => return fail(e),
        };
        if bytes.is_empty() {
            return fail("empty font data");
        }
        if let Some(names) = TEXT_FONT_FAMILIES.get() {
            return ok(json!({ "families": names }));
        }
        let mut engine = text_engine().lock().unwrap_or_else(|e| e.into_inner());
        let names = engine.fonts.register_font_data(bytes.to_vec());
        if names.is_empty() {
            return fail("font data contains no usable faces");
        }
        let names = TEXT_FONT_FAMILIES.get_or_init(|| names);
        ok(json!({ "families": names }))
    })
}

#[derive(Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct TextParams {
    text: String,
    font_family: String,
    size: f32,
    x: f64,
    y: f64,
    color: [f32; 4],
    name: String,
}

impl Default for TextParams {
    fn default() -> Self {
        Self { text: String::new(), font_family: "Inter".into(), size: 48.0, x: 0.0, y: 0.0, color: [0.0, 0.0, 0.0, 1.0], name: String::new() }
    }
}

/// Creates a point-text layer at `(x, y)` (document pixels, first baseline). `fontFamily` should
/// be a family returned by [`pc_font_register`], or `Inter` for the bundled font. Returns
/// `{"layerId":..,"info":{..}}`.
#[unsafe(no_mangle)]
pub extern "C" fn pc_text_create(session: *mut Session, id: u64, params_ptr: *const u8, params_len: usize) -> *mut PcBuffer {
    guard(move || {
        if session.is_null() {
            return fail("null session");
        }
        let params = match unsafe { slice(params_ptr, params_len) } {
            Ok(b) => b,
            Err(e) => return fail(e),
        };
        let p = match serde_json::from_slice::<TextParams>(params) {
            Ok(p) => p,
            Err(e) => return fail(format!("invalid text parameters: {e}")),
        };
        if p.text.trim().is_empty() {
            return fail("text is empty");
        }
        // SAFETY: live session pointer.
        let s = unsafe { &mut *session };
        let entry = match s.docs.get_mut(&id) {
            Some(e) => e,
            None => return fail(format!("document {id} is not open")),
        };
        if entry.doc.bounds().is_empty() {
            return fail("document is empty");
        }
        let mut text = TextLayer::default();
        text.text = p.text.clone();
        text.font_family = if p.font_family.trim().is_empty() { "Inter".into() } else { p.font_family.clone() };
        text.size_pt = p.size.clamp(1.0, 2000.0);
        text.color = shape_color(p.color);
        text.transform = Affine::translate(p.x, p.y);
        text.shape = TextShape::Point;
        text.antialias = AntiAlias::Smooth;
        text.sync_summary();
        let dpi = entry.doc.resolution_dpi;
        let format = entry.doc.pixel_format();
        let mut engine = text_engine().lock().unwrap_or_else(|e| e.into_inner());
        engine.render_layer(&mut text, dpi, format);
        let name = if p.name.trim().is_empty() { format!("Text {}", entry.doc.layers.len() + 1) } else { p.name.clone() };
        entry.checkpoint();
        let layer = Layer::new(name, LayerContent::Text(text));
        let layer_id = layer.id.0;
        entry.doc.layers.push(layer);
        ok(json!({ "layerId": layer_id, "info": doc_info(entry) }))
    })
}

// ===== 图层二次编辑（C-EDIT-API@1）=====
//
// 写入安全策略（计划 2.1.3）：先取拥有所有权的副本 → 校验 → 在副本上修改/重排/重渲染 →
// 与原始比较（无变化则不 checkpoint）→ checkpoint → 写回。任何失败都发生在 checkpoint
// 之前，因此失败路径对文档与 revision 零副作用。

/// `pc_layer_edit_apply` 的写模型（`C-EDIT-API@1 §C`）：全部字段可选，缺省 = 不改。
#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct EditParams {
    text: Option<String>,
    font_family: Option<String>,
    size: Option<f32>,
    /// 文字颜色 `[b, g, r, a]`：与 ArkTS `hexToRgba` 同序，禁止任何通道重排。
    color: Option<[f32; 4]>,
    fill_color: Option<[f32; 4]>,
    stroke_color: Option<[f32; 4]>,
    stroke_width: Option<f32>,
    dx: Option<f64>,
    dy: Option<f64>,
}

/// 可编辑图层类型：只有文字层与形状层有可编辑参数。
#[derive(Clone, Copy, PartialEq, Eq)]
enum EditKind {
    Text,
    Shape,
}

fn edit_kind(content: &LayerContent) -> Option<EditKind> {
    match content {
        LayerContent::Text(_) => Some(EditKind::Text),
        LayerContent::Shape(_) => Some(EditKind::Shape),
        _ => None,
    }
}

/// 颜色读回：`[c[0], c[1], c[2], alpha]`，与写入顺序严格一致（BGR 直通，§B 颜色铁律）。
fn edit_color_json(color: &Color) -> Value {
    json!([color.c[0], color.c[1], color.c[2], color.alpha])
}

/// 无填充/无描边时使用的零色值。
fn zero_color_json() -> Value {
    json!([0.0, 0.0, 0.0, 0.0])
}

/// 颜色分量必须有限且落在 `0..=1`。
fn valid_color(color: [f32; 4]) -> bool {
    color.iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v))
}

fn has_text_fields(p: &EditParams) -> bool {
    p.text.is_some() || p.font_family.is_some() || p.size.is_some() || p.color.is_some()
}

fn has_shape_fields(p: &EditParams) -> bool {
    p.fill_color.is_some() || p.stroke_color.is_some() || p.stroke_width.is_some()
}

/// 校验位移：有限且绝对值 ≤ 100000 px（R7.5）。
fn edit_offset(p: &EditParams) -> Result<(f64, f64), String> {
    let dx = p.dx.unwrap_or(0.0);
    let dy = p.dy.unwrap_or(0.0);
    for v in [dx, dy] {
        if !v.is_finite() || v.abs() > 100_000.0 {
            return Err("offset must be finite and within ±100000 px".into());
        }
    }
    Ok((dx, dy))
}

/// 形状几何读数（`C-EDIT-API@1 §B` 的 shape 分支）：
/// `live` 存在时取参数化几何，否则回落到 `path.control_bounds()`。
#[derive(Default)]
struct ShapeGeometry {
    kind: &'static str,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    from_x: f64,
    from_y: f64,
    to_x: f64,
    to_y: f64,
    weight: f64,
    radius: f64,
}

impl ShapeGeometry {
    fn read(shape: &ShapeLayer) -> Self {
        let mut g = ShapeGeometry::default();
        match &shape.live {
            Some(LiveShape::Rect { rect, radii }) => {
                g.kind = "rect";
                (g.x, g.y, g.w, g.h) = (rect[0], rect[1], rect[2], rect[3]);
                g.radius = radii[0];
            }
            Some(LiveShape::Ellipse { rect }) => {
                g.kind = "ellipse";
                (g.x, g.y, g.w, g.h) = (rect[0], rect[1], rect[2], rect[3]);
            }
            Some(LiveShape::Polygon { rect, .. }) => {
                g.kind = "polygon";
                (g.x, g.y, g.w, g.h) = (rect[0], rect[1], rect[2], rect[3]);
            }
            Some(LiveShape::Line { from, to, weight }) => g.segment("line", *from, *to, *weight),
            Some(LiveShape::ArrowLine { from, to, weight, .. }) => g.segment("arrowLine", *from, *to, *weight),
            // 导入的路径没有参数化几何，只能回落到控制点包围盒。
            None => {
                g.kind = "path";
                if let Some((x0, y0, x1, y1)) = shape.path.control_bounds() {
                    (g.x, g.y, g.w, g.h) = (x0, y0, x1 - x0, y1 - y0);
                }
            }
        }
        g
    }

    /// 线段的 `from/to` 与包围盒左上角（`x/y` 取两端最小值，`w/h` 取绝对差）。
    fn segment(&mut self, kind: &'static str, from: [f64; 2], to: [f64; 2], weight: f64) {
        self.kind = kind;
        self.from_x = from[0];
        self.from_y = from[1];
        self.to_x = to[0];
        self.to_y = to[1];
        self.weight = weight;
        self.x = from[0].min(to[0]);
        self.y = from[1].min(to[1]);
        self.w = (to[0] - from[0]).abs();
        self.h = (to[1] - from[1]).abs();
    }
}

/// 单个图层的可编辑参数（`C-EDIT-API@1 §B`）。
///
/// `pc_layer_edit_get` 与 `pc_layer_edit_apply` 共用本函数，保证两处字段集合永不漂移。
fn edit_json(content: &LayerContent) -> Result<Value, String> {
    match content {
        LayerContent::Text(text) => Ok(json!({
            "kind": "Type",
            "text": text.text,
            "fontFamily": text.font_family,
            "size": text.size_pt,
            "color": edit_color_json(&text.color),
            "x": text.transform.m[4],
            "y": text.transform.m[5],
            "w": 0.0,
            "h": 0.0,
            "shapeKind": "",
            "fromX": 0.0,
            "fromY": 0.0,
            "toX": 0.0,
            "toY": 0.0,
            "weight": 0.0,
            "radius": 0.0,
            "fillColor": zero_color_json(),
            "fillEditable": false,
            "strokeColor": zero_color_json(),
            "strokeEditable": false,
            "strokeWidth": 0.0,
        })),
        LayerContent::Shape(shape) => {
            let g = ShapeGeometry::read(shape);
            // 只有纯色可编辑；渐变/图案填充与描边输出零色值 + `Editable = false`。
            let (fill_color, fill_editable) = match &shape.fill {
                Some(Fill::Solid(color)) => (edit_color_json(color), true),
                Some(_) => (zero_color_json(), false),
                None => (zero_color_json(), true),
            };
            let (stroke_color, stroke_editable, stroke_width) = match &shape.stroke {
                Some(stroke) => {
                    let color = match &stroke.paint {
                        Fill::Solid(c) => edit_color_json(c),
                        _ => zero_color_json(),
                    };
                    (color, matches!(&stroke.paint, Fill::Solid(_)), stroke.width)
                }
                None => (zero_color_json(), false, 0.0),
            };
            Ok(json!({
                "kind": "Shape",
                "text": "",
                "fontFamily": "",
                "size": 0.0,
                "color": zero_color_json(),
                "x": g.x,
                "y": g.y,
                "w": g.w,
                "h": g.h,
                "shapeKind": g.kind,
                "fromX": g.from_x,
                "fromY": g.from_y,
                "toX": g.to_x,
                "toY": g.to_y,
                "weight": g.weight,
                "radius": g.radius,
                "fillColor": fill_color,
                "fillEditable": fill_editable,
                "strokeColor": stroke_color,
                "strokeEditable": stroke_editable,
                "strokeWidth": stroke_width,
            }))
        }
        _ => Err("not an editable layer".into()),
    }
}

/// 写回后的读模型：与随后的 `pc_layer_edit_get` 逐字段一致（§E.8）。
fn edit_state(entry: &DocEntry, layer: u64) -> Result<Value, String> {
    match find_layer(&entry.doc.layers, layer) {
        Some(target) => edit_json(&target.content),
        None => Err("layer not found".into()),
    }
}

/// 形状平移（§E.3）：只移动几何原点，`w/h/radii/weight/sides/star_ratio` 保持不变。
fn translate_live(live: &LiveShape, dx: f64, dy: f64) -> LiveShape {
    let mut moved = live.clone();
    match &mut moved {
        LiveShape::Rect { rect, .. } | LiveShape::Ellipse { rect } | LiveShape::Polygon { rect, .. } => {
            rect[0] += dx;
            rect[1] += dy;
        }
        LiveShape::Line { from, to, .. } | LiveShape::ArrowLine { from, to, .. } => {
            from[0] += dx;
            from[1] += dy;
            to[0] += dx;
            to[1] += dy;
        }
    }
    moved
}

/// 计算新的描边状态（§E.4）：只改 `width`/`paint`/`opacity`，其余样式字段沿用原值。
fn edited_stroke(current: Option<&ShapeStroke>, p: &EditParams) -> Option<ShapeStroke> {
    if p.stroke_width == Some(0.0) {
        return None;
    }
    if p.stroke_color.is_none() && current.is_none() {
        return None;
    }
    // `..base` 保留 align/cap/join/miter_limit/dashes/dash_offset，避免无意改变描边样式。
    let base = current.cloned().unwrap_or_default();
    let (paint, opacity) = match p.stroke_color {
        Some(c) => (Fill::Solid(shape_color([c[0], c[1], c[2], 1.0])), c[3].clamp(0.0, 1.0)),
        None => (base.paint.clone(), base.opacity),
    };
    // 只给颜色、且原本没有描边时按 1 px 构造，避免沿用 `ShapeStroke::default()` 的 3 px。
    let width = match p.stroke_width {
        Some(w) => w,
        None => match current {
            Some(stroke) if stroke.width > 0.0 => stroke.width,
            _ => 1.0,
        },
    };
    Some(ShapeStroke { width, paint, opacity, ..base })
}

/// 在【副本】上应用文字层编辑（§E.1/§E.2），返回 `Some(新内容)` 表示需要写回，`None` 表示
/// 与当前值完全相同（此时调用方不 checkpoint、不写回，§E.6）。
fn apply_text_edit(entry: &DocEntry, layer: u64, p: &EditParams) -> Result<Option<LayerContent>, String> {
    if has_shape_fields(p) {
        return Err("shape fields are not valid for a text layer".into());
    }
    let has_text = has_text_fields(p);
    if !has_text && p.dx.is_none() && p.dy.is_none() {
        return Err("no editable field provided".into());
    }
    if let Some(text) = &p.text
        && text.trim().is_empty()
    {
        return Err("text must not be empty".into());
    }
    if let Some(size) = p.size
        && (!size.is_finite() || !(1.0..=2000.0).contains(&size))
    {
        return Err("text size must be finite and between 1 and 2000 pt".into());
    }
    if let Some(color) = p.color
        && !valid_color(color)
    {
        return Err("text color components must be finite and between 0 and 1".into());
    }
    let (dx, dy) = edit_offset(p)?;

    // 取拥有所有权的副本，随后结束对 `entry.doc` 的不可变借用（§E.10 借用约束）。
    let mut text = match find_layer(&entry.doc.layers, layer) {
        Some(target) => match &target.content {
            LayerContent::Text(text) => text.clone(),
            _ => return Err("not an editable layer".into()),
        },
        None => return Err("layer not found".into()),
    };
    let original = text.clone();
    let dpi = entry.doc.resolution_dpi;
    let format = entry.doc.pixel_format();

    if let Some(v) = &p.text {
        text.text = v.clone();
    }
    if let Some(v) = p.size {
        text.size_pt = v;
    }
    if let Some(v) = p.color {
        // 与 `pc_text_create` 同序写入：`Color::rgba(c[0], c[1], c[2], c[3])`。
        text.color = Color::rgba(v[0], v[1], v[2], v[3]);
    }
    if has_text {
        // 全局属性编辑后归一化分段样式，使 `char_runs()` 回落到摘要字段（§E.1）。
        text.runs.clear();
    }
    if dx != 0.0 || dy != 0.0 {
        // `then` = 先原变换后平移（§E.2）；仅位移不清空 runs。
        text.transform = text.transform.then(&Affine::translate(dx, dy));
    }

    // 字体校验与重排必须在同一把 `Mutex<TextEngine>` 锁内（不可重入，§D）；先比较再渲染，
    // 避免"值完全相同也要重画一遍"。
    let mut engine = text_engine().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(family) = &p.font_family {
        let name = family.trim();
        if name.is_empty() || !engine.fonts.has_family(name) {
            return Err(format!("unknown font family: {name}"));
        }
        text.font_family = name.to_string();
    }
    let unchanged = text == original;
    if !unchanged {
        engine.render_layer(&mut text, dpi, format);
    }
    drop(engine);

    if unchanged {
        return Ok(None);
    }
    Ok(Some(LayerContent::Text(text)))
}

/// 在【副本】上应用形状层编辑（§E.3–§E.5），返回 `Some(新内容)` 表示需要写回，`None` 表示
/// 与当前值完全相同（此时调用方不 checkpoint、不写回，§E.6）。
fn apply_shape_edit(entry: &DocEntry, layer: u64, p: &EditParams) -> Result<Option<LayerContent>, String> {
    if has_text_fields(p) {
        return Err("text fields are not valid for a shape layer".into());
    }
    if !has_shape_fields(p) && p.dx.is_none() && p.dy.is_none() {
        return Err("no editable field provided".into());
    }
    if let Some(color) = p.fill_color
        && !valid_color(color)
    {
        return Err("fill color components must be finite and between 0 and 1".into());
    }
    if let Some(color) = p.stroke_color
        && !valid_color(color)
    {
        return Err("stroke color components must be finite and between 0 and 1".into());
    }
    if let Some(width) = p.stroke_width
        && (!width.is_finite() || !(0.0..=10000.0).contains(&width))
    {
        return Err("stroke width must be finite and between 0 and 10000 px".into());
    }
    let (dx, dy) = edit_offset(p)?;

    let mut shape = match find_layer(&entry.doc.layers, layer) {
        Some(target) => match &target.content {
            LayerContent::Shape(shape) => shape.clone(),
            _ => return Err("not an editable layer".into()),
        },
        None => return Err("layer not found".into()),
    };
    let original = shape.clone();
    let bounds = entry.doc.bounds();
    let format = entry.doc.pixel_format();

    // 渐变/图案的填充与描边没有单一颜色可编辑（§D）。
    if p.fill_color.is_some() && matches!(&shape.fill, Some(fill) if !matches!(fill, Fill::Solid(_))) {
        return Err("this layer's fill is not a solid color".into());
    }
    if (p.stroke_color.is_some() || p.stroke_width.is_some())
        && matches!(&shape.stroke, Some(stroke) if !matches!(&stroke.paint, Fill::Solid(_)))
    {
        return Err("this layer's stroke is not a solid color".into());
    }

    // 几何只允许平移（§E.3）：参数化形状先平移参数再重建 path，导入的形状直接平移 path。
    if dx != 0.0 || dy != 0.0 {
        match &shape.live {
            Some(live) => {
                let moved = translate_live(live, dx, dy);
                shape.path = live_path(&moved);
                shape.live = Some(moved);
            }
            None => shape.path = shape.path.transform(&Affine::translate(dx, dy)),
        }
    }

    if let Some(color) = p.fill_color {
        shape.fill = if color[3] > 0.0 { Some(Fill::Solid(shape_color(color))) } else { None };
    }
    if p.stroke_color.is_some() || p.stroke_width.is_some() {
        shape.stroke = edited_stroke(shape.stroke.as_ref(), p);
    }

    // `validate_shape` 必须在 `render_shape` 之前（与 `pc_shape_create` 一致）。
    if let Err(e) = validate_shape(&shape.path, shape.stroke.as_ref()) {
        return Err(e.to_string());
    }
    if shape == original {
        // 无变化：不重渲染、不 checkpoint（§E.6）。
        return Ok(None);
    }
    // cache 是文档坐标的分块画布，clip 必须取当前的 `Document::bounds()`（§E.5）。
    shape.cache = Some(render_shape(&shape, format, bounds));
    Ok(Some(LayerContent::Shape(shape)))
}

/// Reads the editable parameters of a single text/shape layer. Read-only: no checkpoint, no
/// revision change, no document write (`C-EDIT-API@1 §E.9`).
#[unsafe(no_mangle)]
pub extern "C" fn pc_layer_edit_get(session: *mut Session, id: u64, layer: u64) -> *mut PcBuffer {
    guard(move || {
        if session.is_null() {
            return fail("null session");
        }
        // SAFETY: live session pointer.
        let s = unsafe { &*session };
        let entry = match s.entry(id) {
            Ok(e) => e,
            Err(e) => return fail(e),
        };
        match find_layer(&entry.doc.layers, layer) {
            Some(target) => match edit_json(&target.content) {
                Ok(data) => ok(data),
                Err(e) => fail(e),
            },
            None => fail("layer not found"),
        }
    })
}

/// Edits a single text/shape layer. `params` is a UTF-8 [`EditParams`] JSON object; every field is
/// optional and a missing field is left unchanged. Exactly one checkpoint per successful change;
/// validation failures leave the document and its revision untouched (`C-EDIT-API@1 §E`).
#[unsafe(no_mangle)]
pub extern "C" fn pc_layer_edit_apply(
    session: *mut Session,
    id: u64,
    layer: u64,
    params_ptr: *const u8,
    params_len: usize,
) -> *mut PcBuffer {
    guard(move || {
        if session.is_null() {
            return fail("null session");
        }
        let params = match unsafe { slice(params_ptr, params_len) } {
            Ok(b) => b,
            Err(e) => return fail(e),
        };
        let p = match serde_json::from_slice::<EditParams>(params) {
            Ok(p) => p,
            Err(e) => return fail(format!("invalid edit parameters: {e}")),
        };
        // SAFETY: live session pointer.
        let s = unsafe { &mut *session };
        let entry = match s.docs.get_mut(&id) {
            Some(e) => e,
            None => return fail(format!("document {id} is not open")),
        };
        let kind = match find_layer(&entry.doc.layers, layer) {
            Some(target) => edit_kind(&target.content),
            None => return fail("layer not found"),
        };
        let Some(kind) = kind else {
            return fail("not an editable layer");
        };
        // 第 3–5 步：在副本上校验/修改/重排/重渲染，并按 `PartialEq` 判断是否真有变化。
        let updated = match kind {
            EditKind::Text => apply_text_edit(entry, layer, &p),
            EditKind::Shape => apply_shape_edit(entry, layer, &p),
        };
        let updated = match updated {
            Ok(v) => v,
            Err(e) => return fail(e),
        };
        let Some(updated) = updated else {
            // 无变化：不 checkpoint、不写回、revision 不变（§E.6 / R4.3）。
            return match edit_state(entry, layer) {
                Ok(data) => ok(data),
                Err(e) => fail(e),
            };
        };
        // 写回后的读模型在 checkpoint 之前算好，使 checkpoint 之后不存在任何失败路径（§E.8）。
        let payload = match edit_json(&updated) {
            Ok(data) => data,
            Err(e) => return fail(e),
        };
        // 第 6 步：唯一写前快照点。所有 `return fail(...)` 都在本行之前。
        entry.checkpoint();
        // 第 7 步：唯一写后落地点。第 2 步已确认该图层存在且此后不会删除任何节点，
        // 因此这里不可能落空；用 `if let` 而不是 `else return fail`，避免在 checkpoint
        // 之后引入失败路径（失败路径必须零副作用）。
        if let Some(target) = find_layer_mut(&mut entry.doc.layers, layer) {
            target.content = updated;
        }
        // 第 8 步：返回写回后的 `LayerEditInfo`。
        ok(payload)
    })
}

// ---------- layer masks (`C-MASK-API@1`) ----------

/// 蒙版深度对齐：与引擎的 `mask_at_depth` 等价。文档可为 U8/U16/F32，蒙版格式必须与文档深度
/// 一致，否则后续笔刷/选区写入会按不同精度量化。不改 `doc` / `compose`。
fn mask_at_depth(mut mask: LayerMask, depth: SampleType) -> Result<LayerMask, String> {
    let format = mask.surface.format().with_sample(depth);
    if mask.surface.format() != format {
        mask.surface = mask.surface.try_convert(format).map_err(|e| e.to_string())?;
    }
    Ok(mask)
}

/// 读模型（`C-MASK-API@1 §B`）：全字段必填；`present=false` 时其余字段取契约默认值，
/// 供 UI 草稿直接使用（不是「未知」）。
fn layer_mask_json(layer: &Layer) -> Value {
    match &layer.mask {
        Some(mask) => json!({
            "present": true,
            "enabled": mask.enabled,
            "linked": mask.linked,
            "density": mask.density,
            "feather": mask.feather,
        }),
        None => json!({ "present": false, "enabled": true, "linked": true, "density": 1.0, "feather": 0.0 }),
    }
}

/// 可挂蒙版的图层类型（`C-MASK-API@1 §D` 第 5 条）：像素 / 形状 / 文字。
fn mask_capable(content: &LayerContent) -> bool {
    matches!(content, LayerContent::Raster(_) | LayerContent::Shape(_) | LayerContent::Text(_))
}

/// 用当前选区构造蒙版值（`C-MASK-API@1 §E.4`）：`reveal` 取选区覆盖值，`hide` 取补值；
/// 写入区域为 `doc.bounds()`（文档坐标）。**不清除** `doc.selection`。
fn mask_from_selection(doc: &Document, mode: &str) -> Result<LayerMask, String> {
    let selection = doc.selection.as_ref().ok_or_else(|| "no selection".to_string())?;
    let area = doc.bounds();
    let raw = selection.read_region(area);
    let channels = selection.channels();
    // 选区 surface 就是灰度覆盖面（单通道）；多通道时取第一通道，避免长度断言失败。
    let mut values = if channels == 1 { raw } else { raw.chunks_exact(channels).map(|pixel| pixel[0]).collect() };
    if mode == "hide" {
        for value in &mut values {
            *value = 1.0 - *value;
        }
    }
    let mut mask = LayerMask::reveal_all();
    mask.surface.write_region(area, &values);
    mask.surface.prune();
    mask_at_depth(mask, doc.depth)
}

/// 把当前生效的蒙版（含 density 与 feather）烘焙进像素 alpha（`C-MASK-API@1 §E.5`）。
///
/// 只在**克隆**的 surface 上计算并返回，调用方确认成功后才 checkpoint + 写回，失败零副作用。
/// 仅支持像素图层；图层没有任何像素时无需烘焙，原样返回（移除蒙版仍由调用方完成）。
fn apply_mask_pixels(layer: &Layer, doc: &Document) -> Result<Surface, String> {
    let Some(mask) = layer.mask.as_ref() else {
        return Err("layer has no mask".into());
    };
    let surface = match &layer.content {
        LayerContent::Raster(surface) => surface,
        _ => return Err("apply layer mask supports pixel layers only".into()),
    };
    if !surface.format().alpha {
        return Err("layer pixels have no alpha channel".into());
    }
    // 不裁剪文档边界：画布外的像素也要烘焙（与引擎 #3043 一致）。
    let area = surface.content_bounds();
    if area.is_empty() {
        return Ok(surface.clone());
    }
    // 羽化必须复用合成器的 `combined_mask`：`values_into` 只含 density，不含 feather。
    let feathered = if mask.enabled && feather_sigma(mask.feather) > 0.0 {
        // 不把独立的矢量蒙版烘进像素（apply 后它仍然挂在图层上）。
        let mut mask_only = layer.clone();
        mask_only.vector_mask = None;
        // 区域取文档边界与图层内容边界的并集，覆盖被移出画布的像素。
        combined_mask(&mask_only, doc.bounds().union(&area))
    } else {
        None
    };
    let mut values = Vec::new();
    match &feathered {
        Some(softened) => softened.read_region_into(area, &mut values),
        None => mask.values_into(area, &mut values),
    }
    let mut pixels = surface.read_region(area);
    let channels = surface.channels();
    for (i, value) in values.iter().enumerate() {
        // alpha 是像素格式的最后一个通道。
        if let Some(alpha) = pixels.get_mut(i * channels + channels - 1) {
            *alpha *= *value;
        }
    }
    let mut baked = surface.clone();
    baked.write_region(area, &pixels);
    baked.prune();
    Ok(baked)
}

/// 读取图层蒙版状态。只读：不 checkpoint、不改 revision、不改文档（`C-MASK-API@1 §E.1`）。
/// 对任何存在的图层都成功（组/调整/填充层返回 `present=false`）。
#[unsafe(no_mangle)]
pub extern "C" fn pc_layer_mask_get(session: *mut Session, id: u64, layer: u64) -> *mut PcBuffer {
    guard(move || {
        if session.is_null() {
            return fail("null session");
        }
        // SAFETY: live session pointer.
        let s = unsafe { &*session };
        let entry = match s.entry(id) {
            Ok(e) => e,
            Err(e) => return fail(e),
        };
        match find_layer(&entry.doc.layers, layer) {
            Some(target) => ok(layer_mask_json(target)),
            None => fail("layer not found"),
        }
    })
}

/// 新增蒙版。`fill` = `"reveal"`（全白）| `"hide"`（全黑）。成功恰好一次 checkpoint。
#[unsafe(no_mangle)]
pub extern "C" fn pc_layer_mask_add(
    session: *mut Session,
    id: u64,
    layer: u64,
    fill: *const u8,
    fill_len: usize,
) -> *mut PcBuffer {
    guard(move || {
        if session.is_null() {
            return fail("null session");
        }
        let fill_bytes = match unsafe { slice(fill, fill_len) } {
            Ok(b) => b,
            Err(e) => return fail(e),
        };
        let fill = String::from_utf8_lossy(fill_bytes).into_owned();
        // SAFETY: live session pointer.
        let s = unsafe { &mut *session };
        let entry = match s.docs.get_mut(&id) {
            Some(e) => e,
            None => return fail(format!("document {id} is not open")),
        };
        // §D 校验顺序 3（图层存在）→ 4（fill 取值）→ 5（类型）→ 6（尚无蒙版）。
        let (capable, has_mask) = match find_layer(&entry.doc.layers, layer) {
            Some(target) => (mask_capable(&target.content), target.mask.is_some()),
            None => return fail("layer not found"),
        };
        let base = match fill.as_str() {
            "reveal" => LayerMask::reveal_all(),
            "hide" => LayerMask::hide_all(),
            _ => return fail("fill must be reveal or hide"),
        };
        if !capable {
            return fail("layer cannot take a pixel mask");
        }
        if has_mask {
            return fail("layer already has a mask");
        }
        let mask = match mask_at_depth(base, entry.doc.depth) {
            Ok(m) => m,
            Err(e) => return fail(e),
        };
        // 写回后的读模型在 checkpoint 之前算好，使 checkpoint 之后不存在失败路径。
        let payload = json!({
            "present": true,
            "enabled": mask.enabled,
            "linked": mask.linked,
            "density": mask.density,
            "feather": mask.feather,
        });
        // 唯一写前快照点：以上所有 `return fail(...)` 都在本行之前。
        entry.checkpoint();
        if let Some(target) = find_layer_mut(&mut entry.doc.layers, layer) {
            target.mask = Some(mask);
        }
        ok(payload)
    })
}

/// 修改启用 / 密度 / 羽化。`params` 是 JSON：`{"enabled":bool?,"density":0..1?,"feather":0..1000?}`。
/// 与现值全部相同则不 checkpoint、不写文档，直接返回当前状态（`C-MASK-API@1 §E.3`）。
#[unsafe(no_mangle)]
pub extern "C" fn pc_layer_mask_set(
    session: *mut Session,
    id: u64,
    layer: u64,
    params: *const u8,
    params_len: usize,
) -> *mut PcBuffer {
    guard(move || {
        if session.is_null() {
            return fail("null session");
        }
        let params = match unsafe { slice(params, params_len) } {
            Ok(b) => b,
            Err(e) => return fail(e),
        };
        // SAFETY: live session pointer.
        let s = unsafe { &mut *session };
        let entry = match s.docs.get_mut(&id) {
            Some(e) => e,
            None => return fail(format!("document {id} is not open")),
        };
        // §D 7：已有蒙版。
        let current = match find_layer(&entry.doc.layers, layer) {
            Some(target) => match target.mask.as_ref() {
                Some(mask) => mask.clone(),
                None => return fail("layer has no mask"),
            },
            None => return fail("layer not found"),
        };
        // §D 8：必须是 JSON 对象且至少一个已知键（未知键忽略）。
        let parsed: Value = match serde_json::from_slice(params) {
            Ok(v) => v,
            Err(_) => return fail("invalid mask parameters"),
        };
        let Some(object) = parsed.as_object() else {
            return fail("invalid mask parameters");
        };
        let enabled = match object.get("enabled") {
            Some(value) => match value.as_bool() {
                Some(flag) => Some(flag),
                None => return fail("invalid mask parameters"),
            },
            None => None,
        };
        // §D 9：数值必须有限且在范围内（非数字值同样按此拒绝）。
        let density = match object.get("density") {
            Some(value) => match value.as_f64().map(|n| n as f32) {
                Some(n) if n.is_finite() && (0.0..=1.0).contains(&n) => Some(n),
                _ => return fail("density must be a finite number within 0..1"),
            },
            None => None,
        };
        let feather = match object.get("feather") {
            Some(value) => match value.as_f64().map(|n| n as f32) {
                Some(n) if n.is_finite() && (0.0..=1000.0).contains(&n) => Some(n),
                _ => return fail("feather must be a finite number within 0..1000"),
            },
            None => None,
        };
        if enabled.is_none() && density.is_none() && feather.is_none() {
            return fail("no mask properties given");
        }
        // §E.3：逐项比较，全部相同 → 不 checkpoint、不写文档。
        let mut next = current;
        let mut changed = false;
        if let Some(value) = enabled
            && next.enabled != value
        {
            next.enabled = value;
            changed = true;
        }
        if let Some(value) = density
            && next.density != value
        {
            next.density = value;
            changed = true;
        }
        if let Some(value) = feather
            && next.feather != value
        {
            next.feather = value;
            changed = true;
        }
        let payload = json!({
            "present": true,
            "enabled": next.enabled,
            "linked": next.linked,
            "density": next.density,
            "feather": next.feather,
        });
        if !changed {
            return ok(payload);
        }
        // 唯一写前快照点：以上所有 `return fail(...)` 都在本行之前。
        entry.checkpoint();
        if let Some(target) = find_layer_mut(&mut entry.doc.layers, layer) {
            target.mask = Some(next);
        }
        ok(payload)
    })
}

/// 用当前选区生成 / 覆盖蒙版。`mode` = `"reveal"`（选区内显示）| `"hide"`（选区内隐藏）。
/// 总是 checkpoint（`C-MASK-API@1 §E.4`，即使蒙版值与旧值相同）；**不清除**选区。
#[unsafe(no_mangle)]
pub extern "C" fn pc_layer_mask_fill_from_selection(
    session: *mut Session,
    id: u64,
    layer: u64,
    mode: *const u8,
    mode_len: usize,
) -> *mut PcBuffer {
    guard(move || {
        if session.is_null() {
            return fail("null session");
        }
        let mode_bytes = match unsafe { slice(mode, mode_len) } {
            Ok(b) => b,
            Err(e) => return fail(e),
        };
        let mode = String::from_utf8_lossy(mode_bytes).into_owned();
        // SAFETY: live session pointer.
        let s = unsafe { &mut *session };
        let entry = match s.docs.get_mut(&id) {
            Some(e) => e,
            None => return fail(format!("document {id} is not open")),
        };
        // §D 校验顺序 3（图层存在）→ 4（mode 取值）→ 5（类型）→ 10（选区存在）。
        let (capable, existing) = match find_layer(&entry.doc.layers, layer) {
            Some(target) => (mask_capable(&target.content), target.mask.clone()),
            None => return fail("layer not found"),
        };
        if !matches!(mode.as_str(), "reveal" | "hide") {
            return fail("mode must be reveal or hide");
        }
        if !capable {
            return fail("layer cannot take a pixel mask");
        }
        let selection_mask = match mask_from_selection(&entry.doc, mode.as_str()) {
            Ok(m) => m,
            Err(e) => return fail(e),
        };
        // 已有蒙版只替换 surface，保留 enabled / linked / density / feather（R2.6）。
        let mask = match existing {
            Some(old) => LayerMask {
                surface: selection_mask.surface,
                enabled: old.enabled,
                linked: old.linked,
                density: old.density,
                feather: old.feather,
            },
            None => selection_mask,
        };
        let payload = json!({
            "present": true,
            "enabled": mask.enabled,
            "linked": mask.linked,
            "density": mask.density,
            "feather": mask.feather,
        });
        // 唯一写前快照点：以上所有 `return fail(...)` 都在本行之前。
        entry.checkpoint();
        if let Some(target) = find_layer_mut(&mut entry.doc.layers, layer) {
            target.mask = Some(mask);
        }
        ok(payload)
    })
}

/// 把当前生效的蒙版（含 density / feather）烘焙进像素并移除蒙版。仅像素图层（§E.5）。
#[unsafe(no_mangle)]
pub extern "C" fn pc_layer_mask_apply(session: *mut Session, id: u64, layer: u64) -> *mut PcBuffer {
    guard(move || {
        if session.is_null() {
            return fail("null session");
        }
        // SAFETY: live session pointer.
        let s = unsafe { &mut *session };
        let entry = match s.docs.get_mut(&id) {
            Some(e) => e,
            None => return fail(format!("document {id} is not open")),
        };
        // 第 2 步：在克隆上烘焙（不写文档）。§D 7（无蒙版）→ 11（非像素）→ 12（无 alpha）
        // 的校验都在 helper 内，全部发生在 checkpoint 之前。
        let baked = match find_layer(&entry.doc.layers, layer) {
            Some(target) => apply_mask_pixels(target, &entry.doc),
            None => return fail("layer not found"),
        };
        let baked = match baked {
            Ok(surface) => surface,
            Err(e) => return fail(e),
        };
        let payload = json!({ "present": false, "enabled": true, "linked": true, "density": 1.0, "feather": 0.0 });
        // 唯一写前快照点。
        entry.checkpoint();
        if let Some(target) = find_layer_mut(&mut entry.doc.layers, layer) {
            target.mask = None;
            if let Some(surface) = target.surface_mut() {
                *surface = baked;
            }
        }
        ok(payload)
    })
}

/// 删除蒙版（不烘焙像素）。成功恰好一次 checkpoint（§E.6）。
#[unsafe(no_mangle)]
pub extern "C" fn pc_layer_mask_remove(session: *mut Session, id: u64, layer: u64) -> *mut PcBuffer {
    guard(move || {
        if session.is_null() {
            return fail("null session");
        }
        // SAFETY: live session pointer.
        let s = unsafe { &mut *session };
        let entry = match s.docs.get_mut(&id) {
            Some(e) => e,
            None => return fail(format!("document {id} is not open")),
        };
        // §D 3（图层存在）→ 7（已有蒙版）。
        let has_mask = match find_layer(&entry.doc.layers, layer) {
            Some(target) => target.mask.is_some(),
            None => return fail("layer not found"),
        };
        if !has_mask {
            return fail("layer has no mask");
        }
        let payload = json!({ "present": false, "enabled": true, "linked": true, "density": 1.0, "feather": 0.0 });
        // 唯一写前快照点。
        entry.checkpoint();
        if let Some(target) = find_layer_mut(&mut entry.doc.layers, layer) {
            target.mask = None;
        }
        ok(payload)
    })
}

/// Current selection bounds (`{"active":..,"x0":..,"y0":..,"x1":..,"y1":..}`).
#[unsafe(no_mangle)]
pub extern "C" fn pc_selection_info(session: *mut Session, id: u64) -> *mut PcBuffer {
    guard(move || {
        if session.is_null() {
            return fail("null session");
        }
        // SAFETY: live session pointer.
        let s = unsafe { &*session };
        match s.entry(id) {
            Ok(entry) => ok(selection_json(&entry.doc)),
            Err(e) => fail(e),
        }
    })
}

/// Sets a layer's visibility (searches groups).
#[unsafe(no_mangle)]
pub extern "C" fn pc_layer_visible(session: *mut Session, id: u64, layer: u64, visible: bool) -> *mut PcBuffer {
    guard(move || {
        if session.is_null() {
            return fail("null session");
        }
        // SAFETY: live session pointer.
        let s = unsafe { &mut *session };
        let entry = match s.entry_mut(id) {
            Ok(e) => e,
            Err(e) => return fail(e),
        };
        if find_layer(&entry.doc.layers, layer).is_none() {
            return fail("layer not found");
        }
        entry.checkpoint();
        if let Some(l) = find_layer_mut(&mut entry.doc.layers, layer) {
            l.visible = visible;
        }
        ok(Value::Null)
    })
}

/// Sets a layer's opacity (0..1).
#[unsafe(no_mangle)]
pub extern "C" fn pc_layer_opacity(session: *mut Session, id: u64, layer: u64, opacity: f32) -> *mut PcBuffer {
    guard(move || {
        if session.is_null() {
            return fail("null session");
        }
        // SAFETY: live session pointer.
        let s = unsafe { &mut *session };
        let entry = match s.entry_mut(id) {
            Ok(e) => e,
            Err(e) => return fail(e),
        };
        if find_layer(&entry.doc.layers, layer).is_none() {
            return fail("layer not found");
        }
        entry.checkpoint();
        if let Some(l) = find_layer_mut(&mut entry.doc.layers, layer) {
            l.opacity = opacity.clamp(0.0, 1.0);
        }
        ok(Value::Null)
    })
}

fn parse_blend_mode(name: &str) -> Option<BlendMode> {
    Some(match name {
        "PassThrough" => BlendMode::PassThrough,
        "Normal" => BlendMode::Normal,
        "Dissolve" => BlendMode::Dissolve,
        "Darken" => BlendMode::Darken,
        "Multiply" => BlendMode::Multiply,
        "ColorBurn" => BlendMode::ColorBurn,
        "LinearBurn" => BlendMode::LinearBurn,
        "DarkerColor" => BlendMode::DarkerColor,
        "Lighten" => BlendMode::Lighten,
        "Screen" => BlendMode::Screen,
        "ColorDodge" => BlendMode::ColorDodge,
        "LinearDodge" => BlendMode::LinearDodge,
        "LighterColor" => BlendMode::LighterColor,
        "Overlay" => BlendMode::Overlay,
        "SoftLight" => BlendMode::SoftLight,
        "HardLight" => BlendMode::HardLight,
        "VividLight" => BlendMode::VividLight,
        "LinearLight" => BlendMode::LinearLight,
        "PinLight" => BlendMode::PinLight,
        "HardMix" => BlendMode::HardMix,
        "Difference" => BlendMode::Difference,
        "Exclusion" => BlendMode::Exclusion,
        "Subtract" => BlendMode::Subtract,
        "Divide" => BlendMode::Divide,
        "Hue" => BlendMode::Hue,
        "Saturation" => BlendMode::Saturation,
        "Color" => BlendMode::Color,
        "Luminosity" => BlendMode::Luminosity,
        _ => return None,
    })
}

/// Parameters of `pc_layer_effects_apply`. Every field is optional except `op`; a missing `op`
/// is rejected instead of defaulting to `add`, so a malformed request can never write effects.
#[derive(Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct EffectParams {
    op: String,
    kind: String,
    /// Explicit so a missing index is not mistaken for index `0`.
    index: Option<usize>,
    enabled: Option<bool>,
    blend: Option<String>,
    opacity: Option<f32>,
    /// RGB-like `[c0, c1, c2, arbitrary]`; the fourth component is ignored and alpha is forced
    /// to 1 (effect opacity carries transparency).
    color: Option<[f32; 4]>,
    angle: Option<f32>,
    distance: Option<f32>,
    spread: Option<f32>,
    size: Option<f32>,
    position: Option<String>,
    technique: Option<String>,
    knocks_out: Option<bool>,
    anti_alias: Option<bool>,
    // ---- C-EFFECTS-API@2：内阴影 / 内发光 / 渐变叠加 / 斜面与浮雕专用键 ----
    /// Inner glow source: `edge` / `center` (`GlowSource`).
    source: Option<String>,
    /// Gradient overlay stops, `[{p, color: [c0, c1, c2, (a)]}]`. Kept as a raw JSON value so a
    /// malformed shape reports `effects: invalid gradient stops` instead of the generic
    /// `invalid parameters` produced by a typed deserializer.
    gradient_stops: Option<Value>,
    gradient_style: Option<String>,
    /// Rust-native gradient scale (1.0 = 100 %).
    gradient_scale: Option<f32>,
    reverse: Option<bool>,
    align: Option<bool>,
    dither: Option<bool>,
    bevel_style: Option<String>,
    /// Bevel technique; kept apart from `technique` so the glow parser never sees it.
    bevel_technique: Option<String>,
    /// Bevel depth, `0..=10` (1.0 = 100 %).
    depth: Option<f32>,
    up: Option<bool>,
    soften: Option<f32>,
    /// Bevel altitude in degrees.
    altitude: Option<f32>,
    highlight_color: Option<[f32; 4]>,
    highlight_opacity: Option<f32>,
    shadow_color: Option<[f32; 4]>,
    shadow_opacity: Option<f32>,
}

impl Default for EffectParams {
    fn default() -> Self {
        Self {
            op: String::new(),
            kind: String::new(),
            index: None,
            enabled: None,
            blend: None,
            opacity: None,
            color: None,
            angle: None,
            distance: None,
            spread: None,
            size: None,
            position: None,
            technique: None,
            knocks_out: None,
            anti_alias: None,
            source: None,
            gradient_stops: None,
            gradient_style: None,
            gradient_scale: None,
            reverse: None,
            align: None,
            dither: None,
            bevel_style: None,
            bevel_technique: None,
            depth: None,
            up: None,
            soften: None,
            altitude: None,
            highlight_color: None,
            highlight_opacity: None,
            shadow_color: None,
            shadow_opacity: None,
        }
    }
}

/// The effect kinds this bridge exposes for editing. Everything else is preserved untouched and
/// reported as `unsupported`.
fn effect_kind(effect: &Effect) -> &'static str {
    match effect {
        Effect::DropShadow(_) => "dropShadow",
        Effect::Stroke(_) => "stroke",
        Effect::OuterGlow(_) => "outerGlow",
        Effect::InnerShadow(_) => "innerShadow",
        Effect::InnerGlow(_) => "innerGlow",
        Effect::GradientOverlay { .. } => "gradientOverlay",
        Effect::BevelEmboss(_) => "bevelEmboss",
        // PatternOverlay / ColorOverlay / Satin stay preserved and read-only (`unsupported`):
        // the bridge has no pattern data source, so an editable pattern overlay would render
        // nothing (plan §1.1.2).
        _ => "unsupported",
    }
}

fn is_editable(effect: &Effect) -> bool {
    effect_kind(effect) != "unsupported"
}

fn stroke_position_name(position: StrokePosition) -> &'static str {
    match position {
        StrokePosition::Outside => "outside",
        StrokePosition::Inside => "inside",
        StrokePosition::Center => "center",
    }
}

fn parse_stroke_position(name: &str) -> Option<StrokePosition> {
    Some(match name {
        "outside" => StrokePosition::Outside,
        "inside" => StrokePosition::Inside,
        "center" => StrokePosition::Center,
        _ => return None,
    })
}

fn glow_technique_name(technique: GlowTechnique) -> &'static str {
    match technique {
        GlowTechnique::Softer => "softer",
        GlowTechnique::Precise => "precise",
    }
}

fn parse_glow_technique(name: &str) -> Option<GlowTechnique> {
    Some(match name {
        "softer" => GlowTechnique::Softer,
        "precise" => GlowTechnique::Precise,
        _ => return None,
    })
}

fn glow_source_name(source: GlowSource) -> &'static str {
    match source {
        GlowSource::Edge => "edge",
        GlowSource::Center => "center",
    }
}

fn parse_glow_source(name: &str) -> Option<GlowSource> {
    Some(match name {
        "edge" => GlowSource::Edge,
        "center" => GlowSource::Center,
        _ => return None,
    })
}

fn gradient_style_name(style: GradientStyle) -> &'static str {
    match style {
        GradientStyle::Linear => "linear",
        GradientStyle::Radial => "radial",
        GradientStyle::Angle => "angle",
        GradientStyle::Reflected => "reflected",
        GradientStyle::Diamond => "diamond",
    }
}

fn parse_gradient_style(name: &str) -> Option<GradientStyle> {
    Some(match name {
        "linear" => GradientStyle::Linear,
        "radial" => GradientStyle::Radial,
        "angle" => GradientStyle::Angle,
        "reflected" => GradientStyle::Reflected,
        "diamond" => GradientStyle::Diamond,
        _ => return None,
    })
}

fn bevel_style_name(style: BevelStyle) -> &'static str {
    match style {
        BevelStyle::OuterBevel => "outerBevel",
        BevelStyle::InnerBevel => "innerBevel",
        BevelStyle::Emboss => "emboss",
        BevelStyle::PillowEmboss => "pillowEmboss",
        BevelStyle::StrokeEmboss => "strokeEmboss",
    }
}

fn parse_bevel_style(name: &str) -> Option<BevelStyle> {
    Some(match name {
        "outerBevel" => BevelStyle::OuterBevel,
        "innerBevel" => BevelStyle::InnerBevel,
        "emboss" => BevelStyle::Emboss,
        "pillowEmboss" => BevelStyle::PillowEmboss,
        "strokeEmboss" => BevelStyle::StrokeEmboss,
        _ => return None,
    })
}

fn bevel_technique_name(technique: BevelTechnique) -> &'static str {
    match technique {
        BevelTechnique::Smooth => "smooth",
        BevelTechnique::ChiselHard => "chiselHard",
        BevelTechnique::ChiselSoft => "chiselSoft",
    }
}

fn parse_bevel_technique(name: &str) -> Option<BevelTechnique> {
    Some(match name {
        "smooth" => BevelTechnique::Smooth,
        "chiselHard" => BevelTechnique::ChiselHard,
        "chiselSoft" => BevelTechnique::ChiselSoft,
        _ => return None,
    })
}

/// Smallest / largest accepted `gradientStops` length (contract `C-EFFECTS-API@2`).
const MIN_GRADIENT_STOPS: usize = 2;
const MAX_GRADIENT_STOPS: usize = 256;

/// Parses `gradientStops` (`[{p, color: [c0, c1, c2, (a)]}]`) into Rust stops sorted by `p`.
///
/// Colour channels keep the caller's order (BGR, no swapping) and alpha is forced to 1 like every
/// other effect colour. `None` means the shape must be rejected (`effects: invalid gradient
/// stops`); the caller validates before checkpointing.
fn parse_gradient_stops(value: &Value) -> Option<Vec<(f32, Color)>> {
    let entries = value.as_array()?;
    if !(MIN_GRADIENT_STOPS..=MAX_GRADIENT_STOPS).contains(&entries.len()) {
        return None;
    }
    let mut stops: Vec<(f32, Color)> = Vec::with_capacity(entries.len());
    for entry in entries {
        let position = entry.get("p")?.as_f64()? as f32;
        let channels = entry.get("color")?.as_array()?;
        if channels.len() < 3 {
            return None;
        }
        let mut rgb = [0.0_f32; 3];
        for (slot, channel) in rgb.iter_mut().zip(channels.iter()) {
            *slot = channel.as_f64()? as f32;
        }
        stops.push((position.clamp(0.0, 1.0), effect_color([rgb[0], rgb[1], rgb[2], 1.0])));
    }
    // Ascending by `p`; `total_cmp` avoids a panic on non-finite input while staying stable.
    stops.sort_by(|a, b| a.0.total_cmp(&b.0));
    Some(stops)
}

fn stops_json(stops: &[(f32, Color)]) -> Value {
    Value::Array(stops.iter().map(|(position, color)| json!({ "p": position, "color": color_json(color) })).collect())
}

/// `[c0, c1, c2, 1]`. The ArkTS side sends the component order it uses everywhere (`hexToRgba`
/// returns BGR), and this bridge writes it as-is: swapping here would flip the colours.
fn color_json(color: &Color) -> Value {
    json!([color.c[0], color.c[1], color.c[2], 1.0])
}

fn paint_color_json(paint: &FxPaint) -> Option<Value> {
    match paint {
        FxPaint::Color(color) => Some(color_json(color)),
        // Gradient / pattern paints have no single colour to edit.
        _ => None,
    }
}

fn effect_json(index: usize, effect: &Effect) -> Value {
    let mut out = serde_json::Map::new();
    out.insert("index".into(), json!(index));
    out.insert("kind".into(), json!(effect_kind(effect)));
    out.insert("label".into(), json!(effect.label()));
    out.insert("enabled".into(), json!(effect.enabled()));
    match effect {
        Effect::DropShadow(shadow) => {
            out.insert("blend".into(), json!(format!("{:?}", shadow.common.blend)));
            out.insert("opacity".into(), json!(shadow.common.opacity));
            out.insert("color".into(), color_json(&shadow.color));
            out.insert("angle".into(), json!(shadow.angle));
            out.insert("distance".into(), json!(shadow.distance));
            out.insert("spread".into(), json!(shadow.spread));
            out.insert("size".into(), json!(shadow.size));
            out.insert("knocksOut".into(), json!(shadow.knocks_out));
            out.insert("antiAlias".into(), json!(shadow.anti_alias));
        }
        Effect::Stroke(stroke) => {
            out.insert("blend".into(), json!(format!("{:?}", stroke.common.blend)));
            out.insert("opacity".into(), json!(stroke.common.opacity));
            if let Some(color) = paint_color_json(&stroke.paint) {
                out.insert("color".into(), color);
            }
            out.insert("size".into(), json!(stroke.size));
            out.insert("position".into(), json!(stroke_position_name(stroke.position)));
        }
        Effect::OuterGlow(glow) => {
            out.insert("blend".into(), json!(format!("{:?}", glow.common.blend)));
            out.insert("opacity".into(), json!(glow.common.opacity));
            if let Some(color) = paint_color_json(&glow.paint) {
                out.insert("color".into(), color);
            }
            out.insert("spread".into(), json!(glow.spread));
            out.insert("size".into(), json!(glow.size));
            out.insert("technique".into(), json!(glow_technique_name(glow.technique)));
            out.insert("antiAlias".into(), json!(glow.anti_alias));
        }
        // C-EFFECTS-API@2：内阴影。`knocksOut` 是投影专属语义，刻意不输出。
        Effect::InnerShadow(shadow) => {
            out.insert("blend".into(), json!(format!("{:?}", shadow.common.blend)));
            out.insert("opacity".into(), json!(shadow.common.opacity));
            out.insert("color".into(), color_json(&shadow.color));
            out.insert("angle".into(), json!(shadow.angle));
            out.insert("distance".into(), json!(shadow.distance));
            out.insert("spread".into(), json!(shadow.spread));
            out.insert("size".into(), json!(shadow.size));
            out.insert("antiAlias".into(), json!(shadow.anti_alias));
        }
        // C-EFFECTS-API@2：内发光。渐变/图案 paint 没有单一颜色可编辑，与 @1 一致省略 color。
        Effect::InnerGlow(glow) => {
            out.insert("blend".into(), json!(format!("{:?}", glow.common.blend)));
            out.insert("opacity".into(), json!(glow.common.opacity));
            if let Some(color) = paint_color_json(&glow.paint) {
                out.insert("color".into(), color);
            }
            out.insert("spread".into(), json!(glow.spread));
            out.insert("size".into(), json!(glow.size));
            out.insert("technique".into(), json!(glow_technique_name(glow.technique)));
            out.insert("source".into(), json!(glow_source_name(glow.source)));
            out.insert("antiAlias".into(), json!(glow.anti_alias));
        }
        // C-EFFECTS-API@2：渐变叠加。stops 顺序与 `Gradient.stops` 完全一致（不排序不去重）。
        Effect::GradientOverlay { common, gradient, dither, .. } => {
            out.insert("blend".into(), json!(format!("{:?}", common.blend)));
            out.insert("opacity".into(), json!(common.opacity));
            out.insert("dither".into(), json!(dither));
            out.insert(
                "gradient".into(),
                json!({
                    "stops": stops_json(&gradient.stops),
                    "style": gradient_style_name(gradient.style),
                    "angle": gradient.angle,
                    "scale": gradient.scale,
                    "reverse": gradient.reverse,
                    "align": gradient.align,
                }),
            );
        }
        // C-EFFECTS-API@2：斜面与浮雕。Bevel 没有 `common`，因此没有顶层 blend/opacity。
        Effect::BevelEmboss(bevel) => {
            out.insert("style".into(), json!(bevel_style_name(bevel.style)));
            out.insert("technique".into(), json!(bevel_technique_name(bevel.technique)));
            out.insert("depth".into(), json!(bevel.depth));
            out.insert("up".into(), json!(bevel.up));
            out.insert("size".into(), json!(bevel.size));
            out.insert("soften".into(), json!(bevel.soften));
            out.insert("angle".into(), json!(bevel.angle));
            out.insert("altitude".into(), json!(bevel.altitude));
            out.insert("highlightColor".into(), color_json(&bevel.highlight_color));
            out.insert("highlightOpacity".into(), json!(bevel.highlight.opacity));
            out.insert("shadowColor".into(), color_json(&bevel.shadow_color));
            out.insert("shadowOpacity".into(), json!(bevel.shadow.opacity));
            // PSD 名为 `antialiasGloss`：只映射 Gloss Contour 的抗锯齿，不是主体抗锯齿。
            out.insert("antiAlias".into(), json!(bevel.gloss_anti_alias));
        }
        // Unsupported effects only report what the UI can show.
        _ => {}
    }
    Value::Object(out)
}

fn effects_json(layer: &Layer) -> Value {
    json!({
        "enabled": layer.effects.enabled,
        "items": layer.effects.items.iter().enumerate().map(|(index, effect)| effect_json(index, effect)).collect::<Vec<Value>>(),
    })
}

/// A new instance of `kind` with the contract's factory defaults.
fn new_effect(kind: &str) -> Result<Effect, String> {
    match kind {
        "dropShadow" => {
            let mut effect = Effect::default_drop_shadow();
            if let Effect::DropShadow(shadow) = &mut effect {
                // The compositor would otherwise replace the user's angle with the document's
                // global light angle, which makes the angle slider look broken.
                shadow.use_global_light = false;
            }
            Ok(effect)
        }
        "stroke" => Ok(Effect::Stroke(StrokeFx {
            common: FxCommon::new(BlendMode::Normal, 1.0),
            size: 3.0,
            position: StrokePosition::Outside,
            paint: FxPaint::Color(Color::BLACK),
            overprint: false,
        })),
        "outerGlow" => Ok(Effect::OuterGlow(Glow {
            common: FxCommon::new(BlendMode::Screen, 0.75),
            paint: FxPaint::Color(Color::WHITE),
            technique: GlowTechnique::Softer,
            spread: 0.0,
            size: 5.0,
            contour: Contour::Linear,
            anti_alias: false,
            range: 1.0,
            jitter: 0.0,
            noise: 0.0,
            source: GlowSource::Edge,
        })),
        // C-EFFECTS-API@2：四类新效果的工厂默认值，逐项等于契约 §2.2 默认值表。
        // 内阴影/斜面浮雕都写 `use_global_light = false`，否则角度滑块会被文档全局光覆盖。
        "innerShadow" => Ok(Effect::InnerShadow(photocraft_doc::Shadow {
            common: FxCommon::new(BlendMode::Multiply, 0.75),
            color: Color::BLACK,
            angle: 120.0,
            use_global_light: false,
            distance: 5.0,
            spread: 0.0,
            size: 5.0,
            contour: Contour::Linear,
            anti_alias: false,
            noise: 0.0,
            knocks_out: false,
        })),
        "innerGlow" => Ok(Effect::InnerGlow(Glow {
            common: FxCommon::new(BlendMode::Screen, 0.75),
            paint: FxPaint::Color(Color::WHITE),
            technique: GlowTechnique::Softer,
            spread: 0.0,
            size: 5.0,
            contour: Contour::Linear,
            anti_alias: false,
            range: 1.0,
            jitter: 0.0,
            noise: 0.0,
            source: GlowSource::Edge,
        })),
        "gradientOverlay" => Ok(Effect::GradientOverlay {
            common: FxCommon::new(BlendMode::Normal, 1.0),
            gradient: Gradient {
                stops: vec![(0.0, Color::BLACK), (1.0, Color::WHITE)],
                opacity_stops: Vec::new(),
                style: GradientStyle::Linear,
                angle: 90.0,
                scale: 1.0,
                reverse: false,
                align: true,
                offset: (0.0, 0.0),
            },
            dither: false,
            // 新实例没有烘焙渐变曲线记录。
            interpolation: None,
        }),
        // `Bevel` 没有 `Default` 实现，必须写完整字面量（遗漏字段即编译失败）。
        "bevelEmboss" => Ok(Effect::BevelEmboss(Bevel {
            enabled: true,
            style: BevelStyle::InnerBevel,
            technique: BevelTechnique::Smooth,
            depth: 1.0,
            up: true,
            size: 5.0,
            soften: 0.0,
            angle: 120.0,
            altitude: 30.0,
            use_global_light: false,
            gloss_contour: Contour::Linear,
            gloss_anti_alias: false,
            highlight: FxCommon::new(BlendMode::Screen, 0.75),
            highlight_color: Color::WHITE,
            shadow: FxCommon::new(BlendMode::Multiply, 0.75),
            shadow_color: Color::BLACK,
            contour: None,
            texture: None,
        })),
        other => Err(format!("effects: unsupported effect kind: {other}")),
    }
}

/// The `common` block (enabled/blend/opacity) of effects that have one.
fn effect_common_mut(effect: &mut Effect) -> Option<&mut FxCommon> {
    match effect {
        // `Shadow` / `Glow` are shared by the outer and inner variants; dispatch by kind, never
        // by structure, or an inner effect would silently edit the outer one's fields.
        Effect::DropShadow(e) | Effect::InnerShadow(e) => Some(&mut e.common),
        Effect::Stroke(e) => Some(&mut e.common),
        Effect::OuterGlow(e) | Effect::InnerGlow(e) => Some(&mut e.common),
        Effect::GradientOverlay { common, .. } => Some(common),
        // `Bevel` has no `common`: the top-level `blend`/`opacity` keys are ignored for it and the
        // UI uses `highlightOpacity` / `shadowOpacity` instead (contract §2.2).
        _ => None,
    }
}

fn effect_color(c: [f32; 4]) -> Color {
    Color::rgba(c[0].clamp(0.0, 1.0), c[1].clamp(0.0, 1.0), c[2].clamp(0.0, 1.0), 1.0)
}

/// Applies the requested optional fields to an existing (or freshly created) effect. Fields that
/// do not belong to the effect's kind are ignored, as the contract requires.
fn apply_effect_params(effect: &mut Effect, p: &EffectParams) {
    if let Some(mode) = p.blend.as_deref().and_then(parse_blend_mode)
        && let Some(common) = effect_common_mut(effect)
    {
        common.blend = mode;
    }
    if let Some(opacity) = p.opacity
        && let Some(common) = effect_common_mut(effect)
    {
        common.opacity = opacity.clamp(0.0, 1.0);
    }
    let color = p.color.map(effect_color);
    match effect {
        Effect::DropShadow(shadow) => {
            if let Some(color) = color {
                shadow.color = color;
            }
            if let Some(angle) = p.angle {
                shadow.angle = angle;
                shadow.use_global_light = false;
            }
            if let Some(distance) = p.distance {
                shadow.distance = distance.clamp(0.0, 512.0);
            }
            if let Some(spread) = p.spread {
                shadow.spread = spread.clamp(0.0, 1.0);
            }
            if let Some(size) = p.size {
                shadow.size = size.clamp(0.0, 512.0);
            }
            if let Some(knocks_out) = p.knocks_out {
                shadow.knocks_out = knocks_out;
            }
            if let Some(anti_alias) = p.anti_alias {
                shadow.anti_alias = anti_alias;
            }
        }
        Effect::Stroke(stroke) => {
            if let Some(color) = color {
                stroke.paint = FxPaint::Color(color);
            }
            if let Some(size) = p.size {
                stroke.size = size.clamp(0.0, 512.0);
            }
            if let Some(position) = p.position.as_deref().and_then(parse_stroke_position) {
                stroke.position = position;
            }
        }
        Effect::OuterGlow(glow) => {
            if let Some(color) = color {
                glow.paint = FxPaint::Color(color);
            }
            if let Some(spread) = p.spread {
                glow.spread = spread.clamp(0.0, 1.0);
            }
            if let Some(size) = p.size {
                glow.size = size.clamp(0.0, 512.0);
            }
            if let Some(technique) = p.technique.as_deref().and_then(parse_glow_technique) {
                glow.technique = technique;
            }
            if let Some(anti_alias) = p.anti_alias {
                glow.anti_alias = anti_alias;
            }
        }
        // C-EFFECTS-API@2：四类新效果。未出现的键一律不改（R6.4）。
        Effect::InnerShadow(shadow) => apply_inner_shadow_params(shadow, p, color),
        Effect::InnerGlow(glow) => apply_inner_glow_params(glow, p, color),
        Effect::GradientOverlay { gradient, dither, interpolation, .. } => {
            apply_gradient_overlay_params(gradient, dither, p);
            if p.gradient_stops.is_some() {
                // 契约 §2.3.6：替换色标后必须清空烘焙曲线记录，否则旧曲线会覆盖用户新设色标。
                *interpolation = None;
            }
        }
        Effect::BevelEmboss(bevel) => apply_bevel_params(bevel, p),
        _ => {}
    }
}

/// `innerShadow`：`Shadow` 与投影共用结构，但内阴影没有挖空语义，因此从不写 `knocksOut`。
fn apply_inner_shadow_params(shadow: &mut photocraft_doc::Shadow, p: &EffectParams, color: Option<Color>) {
    if let Some(color) = color {
        shadow.color = color;
    }
    if let Some(angle) = p.angle {
        shadow.angle = angle;
        // 与 @1 投影同因：不强制关闭全局光，角度滑块会被文档全局角度覆盖。
        shadow.use_global_light = false;
    }
    if let Some(distance) = p.distance {
        shadow.distance = distance.clamp(0.0, 512.0);
    }
    if let Some(spread) = p.spread {
        shadow.spread = spread.clamp(0.0, 1.0);
    }
    if let Some(size) = p.size {
        shadow.size = size.clamp(0.0, 512.0);
    }
    if let Some(anti_alias) = p.anti_alias {
        shadow.anti_alias = anti_alias;
    }
}

/// `innerGlow`：`Glow` 与外发光共用结构，按 kind 分派保证不会改错合成位置。
fn apply_inner_glow_params(glow: &mut Glow, p: &EffectParams, color: Option<Color>) {
    if let Some(color) = color {
        glow.paint = FxPaint::Color(color);
    }
    if let Some(spread) = p.spread {
        glow.spread = spread.clamp(0.0, 1.0);
    }
    if let Some(size) = p.size {
        glow.size = size.clamp(0.0, 512.0);
    }
    if let Some(technique) = p.technique.as_deref().and_then(parse_glow_technique) {
        glow.technique = technique;
    }
    if let Some(source) = p.source.as_deref().and_then(parse_glow_source) {
        glow.source = source;
    }
    if let Some(anti_alias) = p.anti_alias {
        glow.anti_alias = anti_alias;
    }
}

/// `gradientOverlay`：只改请求中出现的键；色标整体替换（已按 `p` 升序）。
fn apply_gradient_overlay_params(gradient: &mut Gradient, dither: &mut bool, p: &EffectParams) {
    if let Some(value) = p.dither {
        *dither = value;
    }
    if let Some(style) = p.gradient_style.as_deref().and_then(parse_gradient_style) {
        gradient.style = style;
    }
    if let Some(angle) = p.angle {
        gradient.angle = angle;
    }
    if let Some(scale) = p.gradient_scale {
        gradient.scale = scale.clamp(0.0, 100.0);
    }
    if let Some(reverse) = p.reverse {
        gradient.reverse = reverse;
    }
    if let Some(align) = p.align {
        gradient.align = align;
    }
    // 形状已在 `validate_effect_fields` 校验，这里 `None` 分支不可达；用 `if let` 保持无 panic。
    if let Some(stops) = p.gradient_stops.as_ref().and_then(parse_gradient_stops) {
        gradient.stops = stops;
    }
}

/// `bevelEmboss`：`Bevel` 无 `common`，因此顶层 `blend`/`opacity` 不参与（契约明确忽略）。
fn apply_bevel_params(bevel: &mut Bevel, p: &EffectParams) {
    if let Some(style) = p.bevel_style.as_deref().and_then(parse_bevel_style) {
        bevel.style = style;
    }
    if let Some(technique) = p.bevel_technique.as_deref().and_then(parse_bevel_technique) {
        bevel.technique = technique;
    }
    if let Some(depth) = p.depth {
        bevel.depth = depth.clamp(0.0, 10.0);
    }
    if let Some(up) = p.up {
        bevel.up = up;
    }
    if let Some(size) = p.size {
        bevel.size = size.clamp(0.0, 512.0);
    }
    if let Some(soften) = p.soften {
        bevel.soften = soften.clamp(0.0, 512.0);
    }
    if let Some(angle) = p.angle {
        bevel.angle = angle;
        // 同上：角度写入时关闭全局光，否则滑块无效。
        bevel.use_global_light = false;
    }
    if let Some(altitude) = p.altitude {
        bevel.altitude = altitude.clamp(0.0, 90.0);
    }
    if let Some(color) = p.highlight_color {
        bevel.highlight_color = effect_color(color);
    }
    if let Some(opacity) = p.highlight_opacity {
        bevel.highlight.opacity = opacity.clamp(0.0, 1.0);
    }
    if let Some(color) = p.shadow_color {
        bevel.shadow_color = effect_color(color);
    }
    if let Some(opacity) = p.shadow_opacity {
        bevel.shadow.opacity = opacity.clamp(0.0, 1.0);
    }
    // PSD `antialiasGloss`：映射 Gloss Contour 的抗锯齿。
    if let Some(anti_alias) = p.anti_alias {
        bevel.gloss_anti_alias = anti_alias;
    }
}

fn effect_index(p: &EffectParams) -> Result<usize, String> {
    p.index.ok_or_else(|| "effects: effect index out of range: missing".to_string())
}

/// Rejects values that `apply_effect_params` cannot interpret, before any checkpoint is taken.
fn validate_effect_fields(p: &EffectParams) -> Result<(), String> {
    if let Some(name) = &p.blend
        && parse_blend_mode(name).is_none()
    {
        return Err(format!("unsupported blend mode: {name}"));
    }
    if let Some(name) = &p.position
        && parse_stroke_position(name).is_none()
    {
        return Err(format!("effects: unsupported position/technique: {name}"));
    }
    if let Some(name) = &p.technique
        && parse_glow_technique(name).is_none()
    {
        return Err(format!("effects: unsupported position/technique: {name}"));
    }
    // ---- C-EFFECTS-API@2 新增枚举/结构校验：全部在 checkpoint 之前返回（R8.2） ----
    if let Some(name) = &p.source
        && parse_glow_source(name).is_none()
    {
        return Err(format!("effects: unsupported glow source: {name}"));
    }
    if let Some(name) = &p.gradient_style
        && parse_gradient_style(name).is_none()
    {
        return Err(format!("effects: unsupported gradient style: {name}"));
    }
    if let Some(name) = &p.bevel_style
        && parse_bevel_style(name).is_none()
    {
        return Err(format!("effects: unsupported bevel style: {name}"));
    }
    if let Some(name) = &p.bevel_technique
        && parse_bevel_technique(name).is_none()
    {
        return Err(format!("effects: unsupported bevel technique: {name}"));
    }
    if let Some(value) = &p.gradient_stops
        && parse_gradient_stops(value).is_none()
    {
        // 非数组 / <2 项 / >256 项 / 元素缺 p 或 color：统一一条错误文本。
        return Err("effects: invalid gradient stops".to_string());
    }
    Ok(())
}

/// Full pre-flight check; a failure here must not checkpoint.
fn validate_effect_op(entry: &DocEntry, layer: u64, p: &EffectParams) -> Result<(), String> {
    let target = find_layer(&entry.doc.layers, layer).ok_or_else(|| "layer not found".to_string())?;
    validate_effect_fields(p)?;
    match p.op.as_str() {
        "add" => new_effect(&p.kind).map(|_| ()),
        "update" | "remove" | "enable" => {
            let index = effect_index(p)?;
            let effect = target.effects.items.get(index).ok_or_else(|| format!("effects: effect index out of range: {index}"))?;
            if !is_editable(effect) {
                return Err(format!("effects: effect at index {index} is not editable"));
            }
            if p.op == "update" && !p.kind.is_empty() && effect_kind(effect) != p.kind {
                return Err(format!("effects: effect at index {index} is not {}", p.kind));
            }
            if p.op == "enable" && p.enabled.is_none() {
                return Err("effects: missing enabled value".to_string());
            }
            Ok(())
        }
        "master" => {
            if p.enabled.is_none() {
                return Err("effects: missing enabled value".to_string());
            }
            Ok(())
        }
        "clear" => Ok(()),
        other => Err(format!("effects: unsupported op: {other}")),
    }
}

/// Performs an already validated operation. Never fails for validated input.
fn run_effect_op(entry: &mut DocEntry, layer: u64, p: &EffectParams) -> Result<(), String> {
    let target = find_layer_mut(&mut entry.doc.layers, layer).ok_or_else(|| "layer not found".to_string())?;
    match p.op.as_str() {
        "add" => {
            let mut effect = new_effect(&p.kind)?;
            apply_effect_params(&mut effect, p);
            target.effects.items.push(effect);
        }
        "update" => {
            let index = effect_index(p)?;
            if let Some(effect) = target.effects.items.get_mut(index) {
                apply_effect_params(effect, p);
            }
        }
        "remove" => {
            let index = effect_index(p)?;
            if index < target.effects.items.len() {
                target.effects.items.remove(index);
            }
        }
        "enable" => {
            let index = effect_index(p)?;
            if let (Some(effect), Some(on)) = (target.effects.items.get_mut(index), p.enabled) {
                effect.set_enabled(on);
            }
        }
        "master" => {
            if let Some(on) = p.enabled {
                target.effects.enabled = on;
            }
        }
        // C-EFFECTS-API@2：删除集合由 3 类扩到 7 类（投影/描边/外发光 + 内阴影/内发光/
        // 渐变叠加/斜面浮雕）；PatternOverlay、ColorOverlay、Satin 等 unsupported 保留
        // （删除后无法通过 UI 重建，构成数据丢失）。只含 @1 三类的旧文档结果与本扩展前一致。
        "clear" => target.effects.items.retain(|effect| !is_editable(effect)),
        other => return Err(format!("effects: unsupported op: {other}")),
    }
    Ok(())
}

/// Layer effects (master switch + instances). Read-only: no checkpoint, no revision change.
#[unsafe(no_mangle)]
pub extern "C" fn pc_layer_effects_get(session: *mut Session, id: u64, layer: u64) -> *mut PcBuffer {
    guard(move || {
        if session.is_null() {
            return fail("null session");
        }
        // SAFETY: live session pointer.
        let s = unsafe { &*session };
        let entry = match s.entry(id) {
            Ok(e) => e,
            Err(e) => return fail(e),
        };
        match find_layer(&entry.doc.layers, layer) {
            Some(target) => ok(effects_json(target)),
            None => fail("layer not found"),
        }
    })
}

/// Adds/updates/removes/enables layer effects. `params` is a UTF-8 [`EffectParams`] JSON object.
/// Every successful call checkpoints first; validation failures leave the document untouched.
#[unsafe(no_mangle)]
pub extern "C" fn pc_layer_effects_apply(
    session: *mut Session,
    id: u64,
    layer: u64,
    params_ptr: *const u8,
    params_len: usize,
) -> *mut PcBuffer {
    guard(move || {
        if session.is_null() {
            return fail("null session");
        }
        let params = match unsafe { slice(params_ptr, params_len) } {
            Ok(b) => b,
            Err(e) => return fail(e),
        };
        let p = match serde_json::from_slice::<EffectParams>(params) {
            Ok(p) => p,
            Err(e) => return fail(format!("effects: invalid parameters: {e}")),
        };
        // SAFETY: live session pointer.
        let s = unsafe { &mut *session };
        let entry = match s.docs.get_mut(&id) {
            Some(e) => e,
            None => return fail(format!("document {id} is not open")),
        };
        if let Err(e) = validate_effect_op(entry, layer, &p) {
            return fail(e);
        }
        entry.checkpoint();
        if let Err(e) = run_effect_op(entry, layer, &p) {
            return fail(e);
        }
        // Re-read so the caller gets fresh indexes after remove/clear.
        match find_layer(&entry.doc.layers, layer) {
            Some(target) => ok(effects_json(target)),
            None => fail("layer not found"),
        }
    })
}

/// Sets a layer's blend mode (`Normal`, `Multiply`, ...).
#[unsafe(no_mangle)]
pub extern "C" fn pc_layer_blend(session: *mut Session, id: u64, layer: u64, mode_ptr: *const u8, mode_len: usize) -> *mut PcBuffer {
    guard(move || {
        if session.is_null() {
            return fail("null session");
        }
        let mode_bytes = match unsafe { slice(mode_ptr, mode_len) } {
            Ok(b) => b,
            Err(e) => return fail(e),
        };
        let mode_name = match std::str::from_utf8(mode_bytes) {
            Ok(v) => v,
            Err(_) => return fail("blend mode must be UTF-8"),
        };
        let Some(mode) = parse_blend_mode(mode_name) else {
            return fail(format!("unsupported blend mode: {mode_name}"));
        };
        // SAFETY: live session pointer.
        let s = unsafe { &mut *session };
        let entry = match s.entry_mut(id) {
            Ok(e) => e,
            Err(e) => return fail(e),
        };
        if find_layer(&entry.doc.layers, layer).is_none() {
            return fail("layer not found");
        }
        entry.checkpoint();
        if let Some(l) = find_layer_mut(&mut entry.doc.layers, layer) {
            l.blend = mode;
        }
        ok(Value::Null)
    })
}

/// Sets a layer's fill opacity (0..=1), which affects content but not effects.
#[unsafe(no_mangle)]
pub extern "C" fn pc_layer_fill_opacity(session: *mut Session, id: u64, layer: u64, opacity: f32) -> *mut PcBuffer {
    guard(move || {
        if session.is_null() {
            return fail("null session");
        }
        // SAFETY: live session pointer.
        let s = unsafe { &mut *session };
        let entry = match s.entry_mut(id) {
            Ok(e) => e,
            Err(e) => return fail(e),
        };
        if find_layer(&entry.doc.layers, layer).is_none() {
            return fail("layer not found");
        }
        entry.checkpoint();
        if let Some(l) = find_layer_mut(&mut entry.doc.layers, layer) {
            l.fill_opacity = opacity.clamp(0.0, 1.0);
        }
        ok(Value::Null)
    })
}

/// 新建空白像素图层（桌面版 `layer.new.layer` 的鸿蒙子集）：在 `above` 之上插入一个空 Raster 层。
///
/// `above == 0` 表示插到最顶层之上；`above` 位于组内时插到该组（最外层祖先）之上，不进入组内部。
/// `name` 为空则自动命名「图层 N」（N = 现有「图层 k」中最大的 k + 1）。
/// 全部校验在 `checkpoint` 之前完成，失败零副作用。返回 `{"layerId":id,"info":{..}}`。
#[unsafe(no_mangle)]
pub extern "C" fn pc_layer_create(session: *mut Session, id: u64, above: u64, name_ptr: *const u8, name_len: usize) -> *mut PcBuffer {
    guard(move || {
        if session.is_null() {
            return fail("null session");
        }
        let name_bytes = match unsafe { slice(name_ptr, name_len) } {
            Ok(b) => b,
            Err(e) => return fail(e),
        };
        let name = match std::str::from_utf8(name_bytes) {
            Ok(v) => v.trim(),
            Err(_) => return fail("layer name must be UTF-8"),
        };
        if name.chars().count() > 200 {
            return fail("layer name is too long");
        }
        // SAFETY: live session pointer.
        let s = unsafe { &mut *session };
        let entry = match s.entry_mut(id) {
            Ok(e) => e,
            Err(e) => return fail(e),
        };
        let index = if above == 0 {
            entry.doc.layers.len()
        } else {
            match entry.doc.layers.iter().position(|l| l.id.0 == above) {
                Some(i) => i + 1,
                None => match top_index_containing(&entry.doc.layers, above) {
                    Some(i) => i + 1,
                    None => return fail(format!("layer not found: {above}")),
                },
            }
        };
        let layer_name = if name.is_empty() { next_layer_name(&entry.doc.layers) } else { name.to_string() };
        entry.checkpoint_named("新建图层");
        let format = entry.doc.pixel_format();
        let layer = Layer::raster(layer_name, format);
        let new_id = layer.id.0;
        entry.doc.layers.insert(index, layer);
        ok(json!({ "layerId": new_id, "info": doc_info(entry) }))
    })
}

/// Renames a layer (groups included).
#[unsafe(no_mangle)]
pub extern "C" fn pc_layer_rename(session: *mut Session, id: u64, layer: u64, name_ptr: *const u8, name_len: usize) -> *mut PcBuffer {
    guard(move || {
        if session.is_null() {
            return fail("null session");
        }
        let name_bytes = match unsafe { slice(name_ptr, name_len) } {
            Ok(b) => b,
            Err(e) => return fail(e),
        };
        let name = match std::str::from_utf8(name_bytes) {
            Ok(v) => v.trim(),
            Err(_) => return fail("layer name must be UTF-8"),
        };
        if name.is_empty() {
            return fail("layer name is empty");
        }
        if name.chars().count() > 200 {
            return fail("layer name is too long");
        }
        // SAFETY: live session pointer.
        let s = unsafe { &mut *session };
        let entry = match s.entry_mut(id) {
            Ok(e) => e,
            Err(e) => return fail(e),
        };
        if find_layer(&entry.doc.layers, layer).is_none() {
            return fail("layer not found");
        }
        entry.checkpoint();
        if let Some(l) = find_layer_mut(&mut entry.doc.layers, layer) {
            l.name = name.to_string();
        }
        ok(Value::Null)
    })
}

/// Moves a layer one step within its parent (`delta` +1 = up, -1 = down).
#[unsafe(no_mangle)]
pub extern "C" fn pc_layer_move(session: *mut Session, id: u64, layer: u64, delta: i32) -> *mut PcBuffer {
    guard(move || {
        if session.is_null() {
            return fail("null session");
        }
        if delta == 0 {
            return fail("delta must not be zero");
        }
        // SAFETY: live session pointer.
        let s = unsafe { &mut *session };
        let entry = match s.entry_mut(id) {
            Ok(e) => e,
            Err(e) => return fail(e),
        };
        let Some((pos, len)) = layer_position(&entry.doc.layers, layer) else {
            return fail("layer not found");
        };
        // 任意 delta：先夹到 [0, len-1]；夹后与原地相同即"已到边界"（沿用 ±1 时代的文案）。
        let target = (pos as i32 + delta).clamp(0, len as i32 - 1);
        if target == pos as i32 {
            return fail("layer is already at the edge");
        }
        entry.checkpoint();
        if !move_layer(&mut entry.doc.layers, layer, target - pos as i32) {
            return fail("layer not found");
        }
        ok(doc_info(entry))
    })
}

/// Duplicates a layer above itself, returning `{"layerId":..,"info":{..}}`.
#[unsafe(no_mangle)]
pub extern "C" fn pc_layer_duplicate(session: *mut Session, id: u64, layer: u64) -> *mut PcBuffer {
    guard(move || {
        if session.is_null() {
            return fail("null session");
        }
        // SAFETY: live session pointer.
        let s = unsafe { &mut *session };
        let entry = match s.entry_mut(id) {
            Ok(e) => e,
            Err(e) => return fail(e),
        };
        if find_layer(&entry.doc.layers, layer).is_none() {
            return fail("layer not found");
        }
        entry.checkpoint_named("复制图层");
        let Some(new_id) = duplicate_layer(&mut entry.doc.layers, layer) else {
            return fail("layer not found");
        };
        ok(json!({ "layerId": new_id, "info": doc_info(entry) }))
    })
}

/// Duplicates several layers with a single checkpoint, returning
/// `{"layerIds":[..],"info":{..}}`. Validates every id before writing, so failures leave the
/// document untouched.
#[unsafe(no_mangle)]
pub extern "C" fn pc_layer_duplicate_many(session: *mut Session, id: u64, ids_ptr: *const u8, ids_len: usize) -> *mut PcBuffer {
    guard(move || {
        if session.is_null() {
            return fail("null session");
        }
        let ids_bytes = match unsafe { slice(ids_ptr, ids_len) } {
            Ok(b) => b,
            Err(e) => return fail(e),
        };
        let ids: Vec<u64> = match serde_json::from_slice(ids_bytes) {
            Ok(v) => v,
            Err(e) => return fail(format!("duplicate: invalid layer ids: {e}")),
        };
        if ids.is_empty() {
            return fail("no layer to duplicate");
        }
        // SAFETY: live session pointer.
        let s = unsafe { &mut *session };
        let entry = match s.entry_mut(id) {
            Ok(e) => e,
            Err(e) => return fail(e),
        };
        for layer_id in &ids {
            if find_layer(&entry.doc.layers, *layer_id).is_none() {
                return fail(format!("layer not found: {layer_id}"));
            }
        }
        entry.checkpoint_named("批量复制");
        let mut new_ids = Vec::with_capacity(ids.len());
        for layer_id in ids {
            let Some(new_id) = duplicate_layer(&mut entry.doc.layers, layer_id) else {
                return fail(format!("layer not found: {layer_id}"));
            };
            new_ids.push(new_id);
        }
        ok(json!({ "layerIds": new_ids, "info": doc_info(entry) }))
    })
}

fn mergeable_layer(content: &LayerContent) -> bool {
    matches!(content, LayerContent::Raster(_) | LayerContent::Shape(_) | LayerContent::Text(_))
}

fn find_merge_pair(layers: &[Layer], id: u64) -> Option<(Layer, Layer)> {
    if let Some(pos) = layers.iter().position(|l| l.id.0 == id) {
        if pos == 0 {
            return None;
        }
        return Some((layers[pos].clone(), layers[pos - 1].clone()));
    }
    for l in layers {
        if let Some(children) = l.children()
            && let Some(found) = find_merge_pair(children, id)
        {
            return Some(found);
        }
    }
    None
}

fn apply_merge(layers: &mut Vec<Layer>, id: u64, surface: &Surface) -> Option<u64> {
    if let Some(pos) = layers.iter().position(|l| l.id.0 == id) {
        if pos == 0 {
            return None;
        }
        let mut lower = layers[pos - 1].clone();
        let lower_id = lower.id.0;
        lower.content = LayerContent::Raster(surface.clone());
        lower.effects = Default::default();
        lower.mask = None;
        lower.vector_mask = None;
        lower.fill_cache = None;
        lower.psd_blocks.clear();
        layers[pos - 1] = lower;
        layers.remove(pos);
        return Some(lower_id);
    }
    for l in layers.iter_mut() {
        if let Some(children) = l.children_mut()
            && let Some(new_id) = apply_merge(children, id, surface)
        {
            return Some(new_id);
        }
    }
    None
}

fn layer_content_bounds(layer: &Layer) -> Rect {
    layer.surface().map(|s| s.content_bounds()).unwrap_or(Rect::EMPTY)
}

fn union_rect(a: Rect, b: Rect) -> Rect {
    if a.is_empty() {
        return b;
    }
    if b.is_empty() {
        return a;
    }
    Rect::new(a.x0.min(b.x0), a.y0.min(b.y0), a.x1.max(b.x1), a.y1.max(b.y1))
}

/// Merges a layer down into the layer below it, keeping the lower layer's id/name/blend/opacity.
/// The upper layer's own blend and opacity are baked into the merged pixels; effects and masks are
/// baked as well. Supports pixel, shape and text layers of the same parent; clipping masks must be
/// released and groups/adjustment/fill layers are rejected.
#[unsafe(no_mangle)]
pub extern "C" fn pc_layer_merge_down(session: *mut Session, id: u64, layer: u64) -> *mut PcBuffer {
    guard(move || {
        if session.is_null() {
            return fail("null session");
        }
        // SAFETY: live session pointer.
        let s = unsafe { &mut *session };
        let entry = match s.entry_mut(id) {
            Ok(e) => e,
            Err(e) => return fail(e),
        };
        let Some((pos, _)) = layer_position(&entry.doc.layers, layer) else {
            return fail("layer not found");
        };
        if pos == 0 {
            return fail("no layer below to merge into");
        }
        let Some((upper, lower)) = find_merge_pair(&entry.doc.layers, layer) else {
            return fail("layer not found");
        };
        if upper.clipped {
            return fail("release the clipping mask before merging");
        }
        if !mergeable_layer(&upper.content) || !mergeable_layer(&lower.content) {
            return fail("merge supports pixel, shape and text layers only");
        }
        let doc_bounds = entry.doc.bounds();
        let content = union_rect(layer_content_bounds(&lower), layer_content_bounds(&upper)).intersect(&doc_bounds);
        if content.is_empty() {
            return fail("nothing to merge");
        }
        let mut upper_src = upper.clone();
        let blend = upper.blend;
        let opacity = (upper.opacity * upper.fill_opacity).clamp(0.0, 1.0);
        upper_src.blend = BlendMode::Normal;
        upper_src.opacity = 1.0;
        upper_src.fill_opacity = 1.0;
        let mut lower_src = lower.clone();
        lower_src.blend = BlendMode::Normal;
        lower_src.opacity = 1.0;
        lower_src.fill_opacity = 1.0;
        let upper_buf = render_layer_in(&entry.doc, &upper_src, content);
        let lower_buf = render_layer_in(&entry.doc, &lower_src, content);
        if upper_buf.rect.is_empty() || lower_buf.rect.is_empty() {
            return fail("merge area is too large");
        }
        let format = entry.doc.pixel_format();
        let channels = format.channels();
        let pixel_count = content.width() as usize * content.height() as usize;
        if upper_buf.px.len() != pixel_count || lower_buf.px.len() != pixel_count {
            return fail("merge buffers do not match the requested area");
        }
        let mut samples = vec![0.0f32; pixel_count * channels];
        let mut tmp = [0.0f32; 8];
        for i in 0..pixel_count {
            let merged = composite(blend, lower_buf.px[i], upper_buf.px[i], opacity);
            let len = from_rgba_into(&format, merged, &mut tmp);
            samples[i * channels..i * channels + len].copy_from_slice(&tmp[..len]);
        }
        let mut surface = Surface::new(format);
        surface.write_region(content, &samples);
        entry.checkpoint_named("合并图层");
        let Some(lower_id) = apply_merge(&mut entry.doc.layers, layer, &surface) else {
            return fail("layer not found");
        };
        ok(json!({ "layerId": lower_id, "info": doc_info(entry) }))
    })
}

/// Deletes a layer.
#[unsafe(no_mangle)]
pub extern "C" fn pc_layer_delete(session: *mut Session, id: u64, layer: u64) -> *mut PcBuffer {
    guard(move || {
        if session.is_null() {
            return fail("null session");
        }
        // SAFETY: live session pointer.
        let s = unsafe { &mut *session };
        let entry = match s.entry_mut(id) {
            Ok(e) => e,
            Err(e) => return fail(e),
        };
        entry.checkpoint_named("删除图层");
        if delete_layer(&mut entry.doc.layers, layer) { ok(Value::Null) } else { fail("layer not found") }
    })
}

/// Adds an adjustment layer above every other layer. `params` is a JSON object.
#[unsafe(no_mangle)]
pub extern "C" fn pc_adjustment_add(session: *mut Session, id: u64, kind_ptr: *const u8, kind_len: usize, params_ptr: *const u8, params_len: usize) -> *mut PcBuffer {
    guard(move || {
        if session.is_null() {
            return fail("null session");
        }
        let kind = match unsafe { slice(kind_ptr, kind_len) } {
            Ok(b) => String::from_utf8_lossy(b).to_string(),
            Err(e) => return fail(e),
        };
        let params = match unsafe { slice(params_ptr, params_len) } {
            Ok(b) => serde_json::from_slice::<Value>(b).unwrap_or(Value::Null),
            Err(e) => return fail(e),
        };
        let adj = match build_adjustment(&kind, &params) {
            Ok(a) => a,
            Err(e) => return fail(e),
        };
        // SAFETY: live session pointer.
        let s = unsafe { &mut *session };
        let entry = match s.entry_mut(id) {
            Ok(e) => e,
            Err(e) => return fail(e),
        };
        entry.checkpoint();
        let mut layer = Layer::new(layer_name(&kind), LayerContent::Adjustment(adj));
        layer.opacity = 1.0;
        entry.doc.layers.push(layer.clone());
        ok(json!({ "layerId": layer.id.0, "info": doc_info(entry) }))
    })
}

/// Replaces the parameters of an existing adjustment layer.
#[unsafe(no_mangle)]
pub extern "C" fn pc_adjustment_update(session: *mut Session, id: u64, layer: u64, params_ptr: *const u8, params_len: usize) -> *mut PcBuffer {
    guard(move || {
        if session.is_null() {
            return fail("null session");
        }
        let params = match unsafe { slice(params_ptr, params_len) } {
            Ok(b) => serde_json::from_slice::<Value>(b).unwrap_or(Value::Null),
            Err(e) => return fail(e),
        };
        // SAFETY: live session pointer.
        let s = unsafe { &mut *session };
        let entry = match s.entry_mut(id) {
            Ok(e) => e,
            Err(e) => return fail(e),
        };
        let kind = match find_layer(&entry.doc.layers, layer).map(|l| l.content.clone()) {
            Some(LayerContent::Adjustment(adj)) => match &adj {
                Adjustment::BrightnessContrast { .. } => "brightnessContrast",
                Adjustment::Exposure { .. } => "exposure",
                Adjustment::Vibrance { .. } => "vibrance",
                Adjustment::HueSaturation { .. } => "hueSaturation",
                Adjustment::BlackWhite { .. } => "blackWhite",
                Adjustment::Invert => "invert",
                Adjustment::Posterize { .. } => "posterize",
                Adjustment::Threshold { .. } => "threshold",
                _ => return fail("this adjustment kind is not editable here"),
            },
            _ => return fail("not an adjustment layer"),
        };
        let adj = match build_adjustment(kind, &params) {
            Ok(a) => a,
            Err(e) => return fail(e),
        };
        entry.checkpoint();
        if let Some(l) = find_layer_mut(&mut entry.doc.layers, layer) {
            l.content = LayerContent::Adjustment(adj);
        }
        ok(Value::Null)
    })
}

/// Parameters of one adjustment layer (for the editor panel).
#[unsafe(no_mangle)]
pub extern "C" fn pc_adjustment_get(session: *mut Session, id: u64, layer: u64) -> *mut PcBuffer {
    guard(move || {
        if session.is_null() {
            return fail("null session");
        }
        // SAFETY: live session pointer.
        let s = unsafe { &*session };
        let entry = match s.entry(id) {
            Ok(e) => e,
            Err(e) => return fail(e),
        };
        match find_layer(&entry.doc.layers, layer).map(|l| &l.content) {
            Some(LayerContent::Adjustment(adj)) => ok(adjustment_json(adj)),
            _ => fail("not an adjustment layer"),
        }
    })
}

/// Undoes the last document change.
#[unsafe(no_mangle)]
pub extern "C" fn pc_undo(session: *mut Session, id: u64) -> *mut PcBuffer {
    guard(move || {
        if session.is_null() {
            return fail("null session");
        }
        // SAFETY: live session pointer.
        let s = unsafe { &mut *session };
        let entry = match s.entry_mut(id) {
            Ok(e) => e,
            Err(e) => return fail(e),
        };
        match entry.undo.pop() {
            Some(prev) => {
                let label = entry.undo_labels.pop().unwrap_or_else(|| "编辑".to_string());
                let current = std::mem::replace(&mut entry.doc, prev);
                entry.redo.push(current);
                entry.redo_labels.push(label);
                entry.bump_revision();
                ok(doc_info(entry))
            }
            None => fail("nothing to undo"),
        }
    })
}

/// Redoes the last undone change.
#[unsafe(no_mangle)]
pub extern "C" fn pc_redo(session: *mut Session, id: u64) -> *mut PcBuffer {
    guard(move || {
        if session.is_null() {
            return fail("null session");
        }
        // SAFETY: live session pointer.
        let s = unsafe { &mut *session };
        let entry = match s.entry_mut(id) {
            Ok(e) => e,
            Err(e) => return fail(e),
        };
        match entry.redo.pop() {
            Some(next) => {
                let label = entry.redo_labels.pop().unwrap_or_else(|| "编辑".to_string());
                let current = std::mem::replace(&mut entry.doc, next);
                entry.undo.push(current);
                entry.undo_labels.push(label);
                entry.bump_revision();
                ok(doc_info(entry))
            }
            None => fail("nothing to redo"),
        }
    })
}

/// 历史记录：返回撤销/重做栈的操作标签（最近一步在前），供历史面板只读展示。
#[unsafe(no_mangle)]
pub extern "C" fn pc_history(session: *mut Session, id: u64) -> *mut PcBuffer {
    guard(move || {
        if session.is_null() {
            return fail("null session");
        }
        // SAFETY: live session pointer.
        let s = unsafe { &*session };
        let entry = match s.entry(id) {
            Ok(e) => e,
            Err(e) => return fail(e),
        };
        let undo: Vec<&str> = entry.undo_labels.iter().rev().map(String::as_str).collect();
        let redo: Vec<&str> = entry.redo_labels.iter().rev().map(String::as_str).collect();
        ok(json!({ "undo": undo, "redo": redo }))
    })
}

/// 一次跳转多步撤销：循环搬移文档与标签，**只在结束时递增一次 revision**，
/// 让历史面板点击触发的渲染失效只有一次。步数超过栈深时退化为「撤到底」，返回实际步数。
#[unsafe(no_mangle)]
pub extern "C" fn pc_undo_steps(session: *mut Session, id: u64, steps: u32) -> *mut PcBuffer {
    guard(move || {
        if session.is_null() {
            return fail("null session");
        }
        if steps == 0 {
            return fail("steps must be positive");
        }
        // SAFETY: live session pointer.
        let s = unsafe { &mut *session };
        let entry = match s.entry_mut(id) {
            Ok(e) => e,
            Err(e) => return fail(e),
        };
        if entry.undo.is_empty() {
            return fail("nothing to undo");
        }
        let mut done = 0u32;
        for _ in 0..steps {
            let Some(prev) = entry.undo.pop() else { break };
            let label = entry.undo_labels.pop().unwrap_or_else(|| "编辑".to_string());
            let current = std::mem::replace(&mut entry.doc, prev);
            entry.redo.push(current);
            entry.redo_labels.push(label);
            done += 1;
        }
        entry.bump_revision();
        ok(json!({ "steps": done, "info": doc_info(entry) }))
    })
}

/// 一次跳转多步重做；语义与 [`pc_undo_steps`] 对称。
#[unsafe(no_mangle)]
pub extern "C" fn pc_redo_steps(session: *mut Session, id: u64, steps: u32) -> *mut PcBuffer {
    guard(move || {
        if session.is_null() {
            return fail("null session");
        }
        if steps == 0 {
            return fail("steps must be positive");
        }
        // SAFETY: live session pointer.
        let s = unsafe { &mut *session };
        let entry = match s.entry_mut(id) {
            Ok(e) => e,
            Err(e) => return fail(e),
        };
        if entry.redo.is_empty() {
            return fail("nothing to redo");
        }
        let mut done = 0u32;
        for _ in 0..steps {
            let Some(next) = entry.redo.pop() else { break };
            let label = entry.redo_labels.pop().unwrap_or_else(|| "编辑".to_string());
            let current = std::mem::replace(&mut entry.doc, next);
            entry.undo.push(current);
            entry.undo_labels.push(label);
            done += 1;
        }
        entry.bump_revision();
        ok(json!({ "steps": done, "info": doc_info(entry) }))
    })
}

/// Encodes the document into `out`. `format` is `png`, `jpeg`, `webp`, `tiff` or `psd`.
#[unsafe(no_mangle)]
pub extern "C" fn pc_export(session: *mut Session, id: u64, format_ptr: *const u8, format_len: usize, quality: u8, out: *mut *mut PcBuffer) -> *mut PcBuffer {
    guard(move || {
        if session.is_null() {
            return fail("null session");
        }
        let format = match unsafe { slice(format_ptr, format_len) } {
            Ok(b) => String::from_utf8_lossy(b).to_lowercase(),
            Err(e) => return fail(e),
        };
        if !matches!(format.as_str(), "png" | "jpeg" | "jpg" | "webp" | "tiff" | "tif" | "psd") {
            return fail(format!("unsupported export format: {format}"));
        }
        // SAFETY: live session pointer.
        let s = unsafe { &*session };
        let entry = match s.entry(id) {
            Ok(e) => e,
            Err(e) => return fail(e),
        };
        let mut opts = photocraft_io::ExportOptions::default();
        opts.encode.jpeg_quality = quality.clamp(1, 100);
        opts.encode.webp_quality = quality.clamp(1, 100);
        let name = format!("export.{format}");
        match photocraft_io::export(&entry.doc, &name, &opts) {
            Ok(result) => {
                if !out.is_null() {
                    // SAFETY: out is a caller-provided slot.
                    unsafe { *out = PcBuffer::from_vec(result.bytes) };
                }
                ok(json!({ "warnings": result.warnings }))
            }
            Err(e) => fail(format!("{e}")),
        }
    })
}
// ===== 图层组（C-GROUP-API@1）=====
//
// 写模型与既有 `pc_layer_*` 一致：全部校验在 checkpoint 之前完成，成功恰好一次 checkpoint；
// `pc_layer_expanded` 只影响面板折叠，不 checkpoint、不动 revision（不占用撤销栈）。

/// 返回图层 `id` 所在父级（`None` = 顶层 `doc.layers`）与它在该父级中的索引。
fn locate_layer(layers: &[Layer], id: u64, parent: Option<u64>) -> Option<(Option<u64>, usize)> {
    if let Some(pos) = layers.iter().position(|l| l.id.0 == id) {
        return Some((parent, pos));
    }
    for l in layers {
        if let Some(children) = l.children()
            && let Some(found) = locate_layer(children, id, Some(l.id.0))
        {
            return Some(found);
        }
    }
    None
}

/// 按原顺序把图层从树中摘下并返回它。调用前必须已确认该图层存在。
fn detach_layer(layers: &mut Vec<Layer>, id: u64) -> Option<Layer> {
    if let Some(pos) = layers.iter().position(|l| l.id.0 == id) {
        return Some(layers.remove(pos));
    }
    for l in layers.iter_mut() {
        if let Some(children) = l.children_mut()
            && let Some(found) = detach_layer(children, id)
        {
            return Some(found);
        }
    }
    None
}

/// 把 `target` 的全部祖先 id 追加到 `path`（最外层在前、不含自身）；找到返回 `true`。
fn push_ancestor_ids(layers: &[Layer], target: u64, path: &mut Vec<u64>) -> bool {
    for l in layers {
        if l.id.0 == target {
            return true;
        }
        if let Some(children) = l.children() {
            path.push(l.id.0);
            if push_ancestor_ids(children, target, path) {
                return true;
            }
            path.pop();
        }
    }
    false
}

/// 把 `group` 插到 `parent` 的 `index` 处（越界时收到父级长度）；父级已不存在则落到顶层。
fn insert_into_parent(layers: &mut Vec<Layer>, parent: Option<u64>, index: usize, group: Layer) {
    if let Some(parent_id) = parent
        && let Some(target) = find_layer_mut(layers, parent_id)
        && let Some(children) = target.children_mut()
    {
        let at = index.min(children.len());
        children.insert(at, group);
        return;
    }
    let at = index.min(layers.len());
    layers.insert(at, group);
}

/// 用组的子层按原顺序替换该组层，返回提到父级的子层数。
fn explode_group(layers: &mut Vec<Layer>, id: u64) -> Option<usize> {
    if let Some(pos) = layers.iter().position(|l| l.id.0 == id) {
        let group = layers.remove(pos);
        let children = match group.content {
            LayerContent::Group(group) => group.children,
            _ => Vec::new(),
        };
        let count = children.len();
        layers.splice(pos..pos, children);
        return Some(count);
    }
    for l in layers.iter_mut() {
        if let Some(children) = l.children_mut()
            && let Some(count) = explode_group(children, id)
        {
            return Some(count);
        }
    }
    None
}

/// 把多个图层合并成一个新组。`ids_ptr/ids_len` 是 JSON 数组（`[3,5]`），`name` 是新组名。
///
/// 行为：去重保序 → 逐个确认存在 → 拒绝「ids 中包含某个 id 的祖先」的自包含 →
/// 按 ids 顺序从各自父级摘下 → 插到第一个 id 的原父级原索引处。
/// 成功恰好一次 checkpoint；失败对文档与 revision 零副作用。
#[unsafe(no_mangle)]
pub extern "C" fn pc_layer_group(
    session: *mut Session,
    id: u64,
    ids_ptr: *const u8,
    ids_len: usize,
    name_ptr: *const u8,
    name_len: usize,
) -> *mut PcBuffer {
    guard(move || {
        if session.is_null() {
            return fail("null session");
        }
        let ids_bytes = match unsafe { slice(ids_ptr, ids_len) } {
            Ok(b) => b,
            Err(e) => return fail(e),
        };
        let requested = match serde_json::from_slice::<Vec<u64>>(ids_bytes) {
            Ok(v) => v,
            Err(e) => return fail(format!("group: invalid layer ids: {e}")),
        };
        // 去重但保持调用方顺序（成组顺序 = children 顺序）。
        let mut ids: Vec<u64> = Vec::with_capacity(requested.len());
        for value in requested {
            if !ids.contains(&value) {
                ids.push(value);
            }
        }
        if ids.is_empty() {
            return fail("group needs at least one layer");
        }
        let name_bytes = match unsafe { slice(name_ptr, name_len) } {
            Ok(b) => b,
            Err(e) => return fail(e),
        };
        let name = match std::str::from_utf8(name_bytes) {
            Ok(v) => v,
            Err(_) => return fail("group name must be UTF-8"),
        };
        // SAFETY: live session pointer.
        let s = unsafe { &mut *session };
        let entry = match s.docs.get_mut(&id) {
            Some(e) => e,
            None => return fail(format!("document {id} is not open")),
        };
        // 第 1 步：逐个确认存在（错误文本带 id）。
        for &layer in &ids {
            if find_layer(&entry.doc.layers, layer).is_none() {
                return fail(format!("layer not found: {layer}"));
            }
        }
        // 第 2 步：拒绝自包含——ids 里不能出现另一个 id 的祖先。
        for &layer in &ids {
            let mut ancestors = Vec::new();
            push_ancestor_ids(&entry.doc.layers, layer, &mut ancestors);
            if ancestors.iter().any(|ancestor| ids.contains(ancestor)) {
                return fail("cannot group a group into itself");
            }
        }
        // 第 3 步：记录第一个 id 的原父级与原索引（此刻尚未写入任何内容）。
        let (parent, index) = match locate_layer(&entry.doc.layers, ids[0], None) {
            Some(found) => found,
            None => return fail(format!("layer not found: {}", ids[0])),
        };
        // 第 4 步：用克隆预构造新组与读模型，使 checkpoint 之后不存在失败路径。
        let mut children: Vec<Layer> = Vec::with_capacity(ids.len());
        for &layer in &ids {
            match find_layer(&entry.doc.layers, layer) {
                Some(target) => children.push(target.clone()),
                None => return fail(format!("layer not found: {layer}")),
            }
        }
        let group = Layer::group(name, children);
        let group_id = group.id.0;
        let payload = json!({ "groupId": group_id, "group": layer_json(&group, 0) });
        // 唯一写前快照点：以上所有 `return fail(...)` 都在本行之前。
        entry.checkpoint_named("建组");
        // 先全部摘下（可能来自不同父级），再统一插入，避免插入过程中索引被搅乱。
        for &layer in &ids {
            let _ = detach_layer(&mut entry.doc.layers, layer);
        }
        insert_into_parent(&mut entry.doc.layers, parent, index, group);
        ok(payload)
    })
}

/// 解散一个组：子层按原顺序替换组层。成功恰好一次 checkpoint。
#[unsafe(no_mangle)]
pub extern "C" fn pc_layer_ungroup(session: *mut Session, id: u64, layer: u64) -> *mut PcBuffer {
    guard(move || {
        if session.is_null() {
            return fail("null session");
        }
        // SAFETY: live session pointer.
        let s = unsafe { &mut *session };
        let entry = match s.docs.get_mut(&id) {
            Some(e) => e,
            None => return fail(format!("document {id} is not open")),
        };
        // 校验全部在 checkpoint 之前：目标存在且确实是组。
        let count = match find_layer(&entry.doc.layers, layer) {
            Some(target) if target.is_group() => target.children().map(|c| c.len()).unwrap_or(0),
            Some(_) => return fail("layer is not a group"),
            None => return fail("layer not found"),
        };
        let payload = json!({ "count": count });
        // 唯一写前快照点。
        entry.checkpoint();
        // 上方已确认存在，此处不会再失败。
        let _ = explode_group(&mut entry.doc.layers, layer);
        ok(payload)
    })
}

/// 设置组的展开/折叠。只影响面板，因此**不 checkpoint、不影响 revision**；
/// 同值直接返回，不做任何写入。
#[unsafe(no_mangle)]
pub extern "C" fn pc_layer_expanded(session: *mut Session, id: u64, layer: u64, expanded: u8) -> *mut PcBuffer {
    guard(move || {
        if session.is_null() {
            return fail("null session");
        }
        // SAFETY: live session pointer.
        let s = unsafe { &mut *session };
        let entry = match s.docs.get_mut(&id) {
            Some(e) => e,
            None => return fail(format!("document {id} is not open")),
        };
        let next = expanded != 0;
        let current = match find_layer(&entry.doc.layers, layer) {
            Some(target) => match &target.content {
                LayerContent::Group(group) => group.expanded,
                _ => return fail("layer is not a group"),
            },
            None => return fail("layer not found"),
        };
        if current != next
            && let Some(target) = find_layer_mut(&mut entry.doc.layers, layer)
            && let LayerContent::Group(group) = &mut target.content
        {
            group.expanded = next;
        }
        ok(json!({ "expanded": next }))
    })
}

// ===== 剪贴蒙版（C-CLIP-API@1）=====
//
// `Layer.clipped` 早已存在于 doc crate，合成器（`compose::advanced` / `composite` /
// `extents`）也完整支持：剪贴层与它下方最近的**非剪贴层**（base）构成一个剪贴组，
// 剪贴层的像素被收缩到 base 的 alpha 内。这里只补一个写入 ABI。

/// 同一父级内、位于 `id` 下方（栈序更靠底）是否存在非剪贴层，可作为剪贴蒙版的 base。
///
/// `doc.layers[0]` 是栈底，因此“下方”= 索引更小。返回 `None` 表示 `id` 不在该子树中；
/// `Some(false)` 表示图层存在但其下方全为剪贴层（或它已在父级最底层），不允许开启剪贴。
/// 判定口径与合成器一致：`compose::extents` 取 base 用的是
/// `layers[..k].iter().rev().find(|l| !l.clipped)`。
fn has_unclipped_base(layers: &[Layer], id: u64) -> Option<bool> {
    if let Some(pos) = layers.iter().position(|l| l.id.0 == id) {
        return Some(layers.get(..pos).is_some_and(|below| below.iter().any(|l| !l.clipped)));
    }
    for l in layers {
        if let Some(children) = l.children()
            && let Some(found) = has_unclipped_base(children, id)
        {
            return Some(found);
        }
    }
    None
}

/// 设置图层的剪贴蒙版（`Layer.clipped`）。`clipped` 非 0 即 `true`。
///
/// - 失败路径全部在 checkpoint 之前：目标不存在 → `layer not found`；开启剪贴但同一父级内
///   下方没有非剪贴层可作 base → `layer has no layer below to clip to`；
/// - 同值（重复开启或重复关闭）只回读当前值：不写文档、不 checkpoint、不动 revision；
/// - 值真的变化时恰好一次 checkpoint，随后写入。
#[unsafe(no_mangle)]
pub extern "C" fn pc_layer_clipped(session: *mut Session, id: u64, layer: u64, clipped: u8) -> *mut PcBuffer {
    guard(move || {
        if session.is_null() {
            return fail("null session");
        }
        // SAFETY: live session pointer.
        let s = unsafe { &mut *session };
        let entry = match s.docs.get_mut(&id) {
            Some(e) => e,
            None => return fail(format!("document {id} is not open")),
        };
        let next = clipped != 0;
        let current = match find_layer(&entry.doc.layers, layer) {
            Some(target) => target.clipped,
            None => return fail("layer not found"),
        };
        // 同值是空操作：返回值即当前值，不占撤销栈；此分支不做 base 校验（幂等语义）。
        if current == next {
            return ok(json!({ "clipped": current }));
        }
        if next && !has_unclipped_base(&entry.doc.layers, layer).unwrap_or(false) {
            return fail("layer has no layer below to clip to");
        }
        // 唯一写前快照点：以上所有 `return fail(...)` 都在本行之前。
        entry.checkpoint();
        if let Some(target) = find_layer_mut(&mut entry.doc.layers, layer) {
            target.clipped = next;
        }
        ok(json!({ "clipped": next }))
    })
}

// ===== 批量删除（C-DELETE-MANY@1）=====
//
// 逐个调用 `pc_layer_delete` 会让「删 N 个」产生 N 个撤销点。这里提供一次 checkpoint 的
// 批量删除：解析 JSON id 数组 → 去重保序 → 全量校验 → 剔除「祖先也被选中」的后代 →
// 按树深度倒序删除。所有失败路径都在 checkpoint 之前，因此失败对文档与 revision 零副作用。

/// 把 `layer` 的全部后代 id（深度优先、含孙层、不含自身）追加到 `out`。
fn push_descendant_ids(layer: &Layer, out: &mut Vec<u64>) {
    if let Some(children) = layer.children() {
        for child in children {
            out.push(child.id.0);
            push_descendant_ids(child, out);
        }
    }
}

/// 图层在树中的深度（顶层 = 0）；不存在返回 `None`。
fn layer_depth(layers: &[Layer], id: u64, depth: u32) -> Option<u32> {
    for layer in layers {
        if layer.id.0 == id {
            return Some(depth);
        }
        if let Some(children) = layer.children()
            && let Some(found) = layer_depth(children, id, depth + 1)
        {
            return Some(found);
        }
    }
    None
}

/// 解析 JSON 数组形式的图层 id 并去重保序；空载荷与 `[]` 同样返回空列表。
fn parse_layer_ids(bytes: &[u8]) -> Result<Vec<u64>, String> {
    if bytes.is_empty() {
        return Ok(Vec::new());
    }
    let requested = serde_json::from_slice::<Vec<u64>>(bytes).map_err(|e| format!("delete: invalid layer ids: {e}"))?;
    let mut ids: Vec<u64> = Vec::with_capacity(requested.len());
    for value in requested {
        if !ids.contains(&value) {
            ids.push(value);
        }
    }
    Ok(ids)
}

/// 从 `ids` 中剔除「祖先也被选中」的后代（保持输入顺序）。
///
/// 父组一旦被删除，其子层随父消失；若仍把子层留在待删列表里，之后的删除必然报
/// `layer not found`。因此先收集所有被选中图层的后代 id，再从待删列表移除。
fn drop_nested_ids(layers: &[Layer], ids: &[u64]) -> Vec<u64> {
    let mut nested: Vec<u64> = Vec::new();
    for &layer in ids {
        if let Some(target) = find_layer(layers, layer) {
            push_descendant_ids(target, &mut nested);
        }
    }
    ids.iter().copied().filter(|layer| !nested.contains(layer)).collect()
}

/// 按树深度倒序排列待删 id（深层先删）；同深度保持输入顺序。
///
/// 深度在删除开始前一次性读出，删除过程中不再重新计算，避免浅层先消失影响判断。
fn order_by_depth_desc(layers: &[Layer], ids: &[u64]) -> Vec<u64> {
    let mut ordered: Vec<(u32, u64)> = ids.iter().map(|&layer| (layer_depth(layers, layer, 0).unwrap_or(0), layer)).collect();
    // `sort_by` 是稳定排序：同深度图层维持调用方给出的顺序。
    ordered.sort_by(|a, b| b.0.cmp(&a.0));
    ordered.into_iter().map(|(_, layer)| layer).collect()
}

/// 一次 checkpoint 删除多个图层。`ids_ptr/ids_len` 是 JSON 数组字符串（`[3,5]`）。
///
/// 行为：解析 → 去重保序 → 空数组报 `no layer to delete` → 逐个确认存在
/// （任一缺失报 `layer not found: {id}`）→ 剔除「祖先也被选中」的后代 →
/// 一次 checkpoint → 按树深度倒序删除。成功返回 `{"deleted": N}`，N 是实际删除的
/// 图层数（父组被删时其子层已随父消失，不计入 N）。一次撤销即恢复整批。
#[unsafe(no_mangle)]
pub extern "C" fn pc_layer_delete_many(session: *mut Session, id: u64, ids_ptr: *const u8, ids_len: usize) -> *mut PcBuffer {
    guard(move || {
        if session.is_null() {
            return fail("null session");
        }
        let ids_bytes = match unsafe { slice(ids_ptr, ids_len) } {
            Ok(b) => b,
            Err(e) => return fail(e),
        };
        let ids = match parse_layer_ids(ids_bytes) {
            Ok(v) => v,
            Err(e) => return fail(e),
        };
        if ids.is_empty() {
            return fail("no layer to delete");
        }
        // SAFETY: live session pointer.
        let s = unsafe { &mut *session };
        let entry = match s.docs.get_mut(&id) {
            Some(e) => e,
            None => return fail(format!("document {id} is not open")),
        };
        // 第 1 步：逐个确认存在（错误文本带 id），全部在 checkpoint 之前。
        for &layer in &ids {
            if find_layer(&entry.doc.layers, layer).is_none() {
                return fail(format!("layer not found: {layer}"));
            }
        }
        // 第 2 步：剔除后会随父层一起消失的后代。
        let targets = drop_nested_ids(&entry.doc.layers, &ids);
        if targets.is_empty() {
            return fail("no layer to delete");
        }
        // 第 3 步：计算深度并倒序（只读，不写文档）。
        let ordered = order_by_depth_desc(&entry.doc.layers, &targets);
        // 唯一写前快照点：以上所有 `return fail(...)` 都在本行之前。
        entry.checkpoint_named("批量删除");
        // 已确认存在且深层先删，这里的 `delete_layer` 不会失败。
        let deleted = ordered.iter().filter(|layer| delete_layer(&mut entry.doc.layers, **layer)).count();
        ok(json!({ "deleted": deleted }))
    })
}

/// 裁剪到文档坐标矩形（`C-CANVAS-API@1` 第 2 段）。
///
/// 参数顺序与 `Rect::new(x0, y0, x1, y1)` 一致（`x1`/`y1` 是独占上界）。矩形规范化
/// （`x0 > x1` 时交换）与"与画布求交、判空"全部由 doc 层负责，桥接层不重复几何校验。
///
/// 返回 `{"ok":true,"data":<DocumentInfo>}`；失败 `{"ok":false,"error":"<原因>"}`。
/// **只有 `Ok(true)`（确已变更）才入栈恰好一次「变更前快照」并用 `checkpoint_snapshot`
/// 递增 revision**；`Ok(false)`（矩形与当前画布完全一致）与 `Err`（非法矩形）两条路径都
/// 不入栈、`revision` 不变。快照必须在 `crop_to` **之前**取：doc 层是"先算后写"，调用前
/// 无法预知结果，调用后文档已被裁掉。
#[unsafe(no_mangle)]
pub extern "C" fn pc_document_crop(session: *mut Session, id: u64, x0: i32, y0: i32, x1: i32, y1: i32) -> *mut PcBuffer {
    guard(move || {
        if session.is_null() {
            return fail("null session");
        }
        // SAFETY: 调用方保证 session 是指向存活 `Session` 的非空指针。
        let s = unsafe { &mut *session };
        let entry = match s.entry_mut(id) {
            Ok(e) => e,
            Err(e) => return fail(e),
        };
        // 变更前快照：撤销栈必须存"裁剪之前的文档"，否则 `pc_undo` 恢复不回原尺寸。
        let before = entry.doc.clone();
        match entry.doc.crop_to(Rect::new(x0, y0, x1, y1)) {
            Err(e) => fail(e),
            Ok(false) => ok(doc_info(entry)),
            Ok(true) => {
                entry.checkpoint_snapshot_named(before, "裁剪");
                ok(doc_info(entry))
            }
        }
    })
}

/// 按目标尺寸与 9 宫格锚点调整画布（`C-CANVAS-API@1` 第 2 段）。
///
/// `anchor` ∈ 0..=8，语义见 `CanvasAnchor::from_index`（`row * 3 + col`，4 = 中心）。
/// 校验顺序：空 session → 文档未打开 → `width/height < 1` → 锚点越界 → doc 层
/// （单边上限与总像素上限由 `photocraft_doc::canvas` 守门，桥接层不另立数值）。
/// 入栈时机与 `pc_document_crop` 相同：仅 `Ok(true)` 时用「变更前快照」恰好入栈一次。
#[unsafe(no_mangle)]
pub extern "C" fn pc_document_resize_canvas(session: *mut Session, id: u64, width: i32, height: i32, anchor: i32) -> *mut PcBuffer {
    guard(move || {
        if session.is_null() {
            return fail("null session");
        }
        // SAFETY: 同上，session 为存活 `Session` 的非空指针。
        let s = unsafe { &mut *session };
        let entry = match s.entry_mut(id) {
            Ok(e) => e,
            Err(e) => return fail(e),
        };
        // 非正尺寸在桥接层先拦（doc 层同样会拒绝，但这里给出统一文案并避免无谓的克隆）。
        if width < 1 || height < 1 {
            return fail("invalid canvas size");
        }
        let anchor = match photocraft_doc::canvas::CanvasAnchor::from_index(anchor) {
            Some(a) => a,
            None => return fail("invalid anchor"),
        };
        // 变更前快照：只有 resize_canvas 返回 Ok(true) 才入栈，撤销才能回到旧画布尺寸。
        let before = entry.doc.clone();
        match entry.doc.resize_canvas(width as u32, height as u32, anchor) {
            Err(e) => fail(e),
            Ok(false) => ok(doc_info(entry)),
            Ok(true) => {
                entry.checkpoint_snapshot_named(before, "画布尺寸");
                ok(doc_info(entry))
            }
        }
    })
}

// ===== 参考线 / 标尺原点（C-GUIDES-API@1）=====
//
// `Document::guides` 与 `Document::ruler_origin` 早已存在于 doc crate（含 `Guides::color` /
// `set_color` / `prune_colors`），桥接层此前完全未暴露。这里只补读写 ABI：
// - get 只读，不 checkpoint、不动 revision；
// - set 是参考线**全量替换**：三个键都可选，缺省键表示该轴保持当前值；
// - 所有校验都在 checkpoint 之前，失败对文档与 revision 零副作用；
// - 写入前排序（`total_cmp`）并按 1e-6 容差去重；`origin` 原样写入。

/// 单轴参考线数量上限。
const MAX_GUIDES_PER_AXIS: usize = 256;
/// 单个参考线位置 / 原点分量的绝对值上限（文档像素）。
const MAX_GUIDE_COORD: f64 = 100_000.0;
/// 排序去重时视为同一位置的距离阈值。
const GUIDE_EPSILON: f64 = 1e-6;

/// `pc_guides_set` 的请求体。三个键都可选，缺省即不改。
#[derive(Deserialize)]
struct GuidesParams {
    horizontal: Option<Vec<f64>>,
    vertical: Option<Vec<f64>>,
    origin: Option<[f64; 2]>,
}

/// 参考线 / 标尺原点的读模型，`get` 与 `set` 成功时返回同一形状。
fn guides_json(doc: &Document) -> Value {
    json!({
        "horizontal": doc.guides.horizontal,
        "vertical": doc.guides.vertical,
        "origin": doc.ruler_origin,
    })
}

fn valid_guide_axis(positions: &[f64]) -> bool {
    positions.len() <= MAX_GUIDES_PER_AXIS && positions.iter().all(|p| p.is_finite() && p.abs() <= MAX_GUIDE_COORD)
}

fn valid_guide_origin(origin: &[f64; 2]) -> bool {
    origin.iter().all(|v| v.is_finite() && v.abs() <= MAX_GUIDE_COORD)
}

/// 排序并按 1e-6 容差去重；相邻值只保留排序后的第一个。
fn sorted_unique(mut positions: Vec<f64>) -> Vec<f64> {
    positions.sort_by(f64::total_cmp);
    let mut out: Vec<f64> = Vec::with_capacity(positions.len());
    for p in positions {
        if out.last().is_none_or(|last| (*last - p).abs() > GUIDE_EPSILON) {
            out.push(p);
        }
    }
    out
}

/// 读取参考线与标尺原点。只读：不 checkpoint、不动 revision。
#[unsafe(no_mangle)]
pub extern "C" fn pc_guides_get(session: *mut Session, id: u64) -> *mut PcBuffer {
    guard(move || {
        if session.is_null() {
            return fail("null session");
        }
        // SAFETY: live session pointer.
        let s = unsafe { &*session };
        match s.entry(id) {
            Ok(entry) => ok(guides_json(&entry.doc)),
            Err(e) => fail(e),
        }
    })
}

/// 全量替换参考线与标尺原点（`C-GUIDES-API@1`）。
#[unsafe(no_mangle)]
pub extern "C" fn pc_guides_set(
    session: *mut Session,
    id: u64,
    params_ptr: *const u8,
    params_len: usize,
) -> *mut PcBuffer {
    guard(move || {
        if session.is_null() {
            return fail("null session");
        }
        let raw = match unsafe { slice(params_ptr, params_len) } {
            Ok(b) => b,
            Err(e) => return fail(e),
        };
        // 解析失败与数值校验失败共用同一文案，UI 只需判断一个错误分支。
        let params = match serde_json::from_slice::<GuidesParams>(raw) {
            Ok(p) => p,
            Err(_) => return fail("invalid guides"),
        };
        // SAFETY: live session pointer.
        let s = unsafe { &mut *session };
        let entry = match s.docs.get_mut(&id) {
            Some(e) => e,
            None => return fail(format!("document {id} is not open")),
        };
        let invalid = params.horizontal.as_ref().is_some_and(|v| !valid_guide_axis(v))
            || params.vertical.as_ref().is_some_and(|v| !valid_guide_axis(v))
            || params.origin.as_ref().is_some_and(|o| !valid_guide_origin(o));
        if invalid {
            return fail("invalid guides");
        }
        let current = guides_json(&entry.doc);
        let next_h = params.horizontal.map_or_else(|| entry.doc.guides.horizontal.clone(), sorted_unique);
        let next_v = params.vertical.map_or_else(|| entry.doc.guides.vertical.clone(), sorted_unique);
        let next_origin = params.origin.unwrap_or(entry.doc.ruler_origin);
        // 同值是空操作：不写、不 checkpoint、revision 不变。
        if next_h == entry.doc.guides.horizontal && next_v == entry.doc.guides.vertical && next_origin == entry.doc.ruler_origin {
            return ok(current);
        }
        // 唯一写前快照点：以上解析、校验与同值判定全部完成。
        entry.checkpoint();
        entry.doc.guides.horizontal = next_h;
        entry.doc.guides.vertical = next_v;
        entry.doc.ruler_origin = next_origin;
        ok(guides_json(&entry.doc))
    })
}

// ---------- 图层自由变换（`C-XFORM-API@1`） ----------

/// 变换参数（`C-XFORM-API@1 §2.2`）：全部键可选，缺省表示该维度不变。
#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct TransformParams {
    scale_x: Option<f64>,
    scale_y: Option<f64>,
    /// 旋转度数（增量）。
    angle: Option<f64>,
    dx: Option<f64>,
    dy: Option<f64>,
}

/// 重采样输出像素数上限：超过即拒绝，避免一次变换把内存打爆（`C-XFORM-API@1 §2.6`）。
const MAX_TRANSFORM_PIXELS: u64 = 64_000_000;

/// 图层内容包围盒：组取画板矩形，无画板时取子层并集（与 `comps::position_bounds` 同口径）。
fn transform_content_bounds(l: &Layer) -> Rect {
    match &l.content {
        LayerContent::Group(g) => {
            let board = g.artboard.as_ref().map_or(Rect::EMPTY, |ab| ab.rect);
            if board.is_empty() {
                g.children.iter().fold(Rect::EMPTY, |r, c| r.union(&transform_content_bounds(c)))
            } else {
                board
            }
        }
        _ => layer_content_bounds(l),
    }
}

/// 输出像素数低于此值时不进 Rayon：线程分派开销会盖过并行收益。
const PARALLEL_RESAMPLE_MIN_PIXELS: usize = 65_536;

/// 重采样性能累加器（进程级）：`pc_layer_transform` 按调用前后差值上报顶层 `perf`。
/// 真机 `eprintln!` 不进 hilog，所以性能数值必须经 C ABI 返回值带出。
static RESAMPLE_CALLS: AtomicU64 = AtomicU64::new(0);
static RESAMPLE_PIXELS: AtomicU64 = AtomicU64::new(0);
static RESAMPLE_NANOS: AtomicU64 = AtomicU64::new(0);
/// 重采样四段拆分耗时（read / fill / write / prune），与 `RESAMPLE_NANOS` 同为进程级累加。
static RESAMPLE_READ_NANOS: AtomicU64 = AtomicU64::new(0);
static RESAMPLE_FILL_NANOS: AtomicU64 = AtomicU64::new(0);
static RESAMPLE_WRITE_NANOS: AtomicU64 = AtomicU64::new(0);
static RESAMPLE_PRUNE_NANOS: AtomicU64 = AtomicU64::new(0);

/// 按 `inv` 把 `s` 重采样进 `dest`（文档坐标），单次双线性插值。
///
/// 输出像素中心反算到源坐标后取四个邻点：源包围盒之外按 `Surface` 的读语义取默认像素，
/// 因此内容边界不会向内渗色。带 alpha 的表面用预乘权重累加，避免透明像素把边缘颜色拖暗。
fn resample_affine(s: &Surface, inv: &Affine, dest: Rect) -> Surface {
    // 一次性环境探针：判定鸿蒙是否把进程限制到单核、以及 rayon 全局池实际线程数。
    static RESAMPLE_ENV_ONCE: std::sync::Once = std::sync::Once::new();
    RESAMPLE_ENV_ONCE.call_once(|| {
        let avail = std::thread::available_parallelism().map(|v| v.get()).unwrap_or(0);
        #[cfg(not(target_arch = "wasm32"))]
        let rayon_threads = rayon::current_num_threads();
        #[cfg(target_arch = "wasm32")]
        let rayon_threads = 0;
        eprintln!("[PC-RESAMPLE] env available_parallelism={avail} rayon_threads={rayon_threads}");
    });
    let mut out = Surface::with_default(s.format(), &s.default_pixel());
    let src = s.content_bounds();
    if src.is_empty() || dest.is_empty() {
        return out;
    }
    let n = s.channels().max(1);
    let (w, h) = (src.width() as usize, src.height() as usize);
    let t_read = Instant::now();
    let raw = s.read_region(src);
    let read_d = t_read.elapsed();
    let read_ms = read_d.as_secs_f64() * 1_000.0;
    let default = s.default_pixel();
    let at = |x: i64, y: i64, c: usize| -> f32 {
        let (dx, dy) = (x - i64::from(src.x0), y - i64::from(src.y0));
        if dx < 0 || dy < 0 || dx >= w as i64 || dy >= h as i64 {
            return default.get(c).copied().unwrap_or(0.0);
        }
        raw[(dy as usize * w + dx as usize) * n + c]
    };
    let alpha = s.format().alpha.then_some(n - 1);
    let (dw, dh) = (dest.width() as usize, dest.height() as usize);
    let stride = dw * n;
    let parallel = cfg!(not(target_arch = "wasm32")) && dw * dh >= PARALLEL_RESAMPLE_MIN_PIXELS;
    // 计时包含输出缓冲分配与默认值填充 + 逐行填充（即整段填充阶段）。
    let t_fill = Instant::now();
    // `Surface::write_pixel` 需要 `&mut self`，surface 本身不能按行拆分，所以先把结果算进行连续的
    // 浮点缓冲，再整块写回。反算坐标非有限的像素保持默认值，等价于串行路径的“跳过即默认”。
    let mut buf = vec![0.0f32; stride * dh];
    // 默认像素全零（绝大多数图层内容）时缓冲初值已等于默认值，省掉整块逐像素填充。
    if default.iter().take(n).any(|v| *v != 0.0) {
        for px in buf.chunks_mut(n) {
            for (c, slot) in px.iter_mut().enumerate() {
                *slot = default.get(c).copied().unwrap_or(0.0);
            }
        }
    }
    let fill_row = |row: &mut [f32], y: i32| {
        let mut px = vec![0.0f32; n];
        for (i, x) in (dest.x0..dest.x1).enumerate() {
            let p = inv.apply(photocraft_geom::Point::new(f64::from(x) + 0.5, f64::from(y) + 0.5));
            let (fx, fy) = (p.x - 0.5, p.y - 0.5);
            if !fx.is_finite() || !fy.is_finite() {
                continue;
            }
            let (ix, iy) = (fx.floor(), fy.floor());
            let (tx, ty) = ((fx - ix) as f32, (fy - iy) as f32);
            let (ix, iy) = (ix as i64, iy as i64);
            let sample = |c: usize, premul: bool| -> f32 {
                let (v00, v10, v01, v11) = (at(ix, iy, c), at(ix + 1, iy, c), at(ix, iy + 1, c), at(ix + 1, iy + 1, c));
                let (v00, v10, v01, v11) = if premul && alpha.is_some() {
                    let (a00, a10, a01, a11) = (at(ix, iy, n - 1), at(ix + 1, iy, n - 1), at(ix, iy + 1, n - 1), at(ix + 1, iy + 1, n - 1));
                    (v00 * a00, v10 * a10, v01 * a01, v11 * a11)
                } else {
                    (v00, v10, v01, v11)
                };
                let top = v00 + (v10 - v00) * tx;
                let bottom = v01 + (v11 - v01) * tx;
                top + (bottom - top) * ty
            };
            match alpha {
                Some(ac) => {
                    let a_sum = sample(ac, false);
                    px[ac] = a_sum;
                    for c in 0..n {
                        if c == ac {
                            continue;
                        }
                        // 全透明处没有可还原的颜色，退回未预乘的插值结果。
                        px[c] = if a_sum > 1e-6 { sample(c, true) / a_sum } else { sample(c, false) };
                    }
                }
                None => {
                    for (c, slot) in px.iter_mut().enumerate() {
                        *slot = sample(c, false);
                    }
                }
            }
            row[i * n..][..n].copy_from_slice(&px);
        }
    };
    if !parallel {
        for (row, data) in buf.chunks_mut(stride).enumerate() {
            fill_row(data, dest.y0 + row as i32);
        }
    } else {
        // 线程数沿用 rayon 默认（全局池按可用 CPU 核数），不在此处硬编码。
        #[cfg(not(target_arch = "wasm32"))]
        {
            use rayon::prelude::*;
            buf.par_chunks_mut(stride).enumerate().for_each(|(row, data)| fill_row(data, dest.y0 + row as i32));
        }
        #[cfg(target_arch = "wasm32")]
        {
            for (row, data) in buf.chunks_mut(stride).enumerate() {
                fill_row(data, dest.y0 + row as i32);
            }
        }
    }
    let fill_d = t_fill.elapsed();
    let fill_ms = fill_d.as_secs_f64() * 1_000.0;
    let t_write = Instant::now();
    out.write_region_par(dest, &buf);
    let write_d = t_write.elapsed();
    let write_ms = write_d.as_secs_f64() * 1_000.0;
    let t_prune = Instant::now();
    out.prune();
    let prune_d = t_prune.elapsed();
    let prune_ms = prune_d.as_secs_f64() * 1_000.0;
    // 只在此处（成功走到末尾）累加：空内容 / 提前 return 的失败路径不计入 `perf`。
    RESAMPLE_CALLS.fetch_add(1, Ordering::Relaxed);
    RESAMPLE_PIXELS.fetch_add((dw * dh) as u64, Ordering::Relaxed);
    RESAMPLE_NANOS.fetch_add(u64::try_from(t_read.elapsed().as_nanos()).unwrap_or(u64::MAX), Ordering::Relaxed);
    RESAMPLE_READ_NANOS.fetch_add(u64::try_from(read_d.as_nanos()).unwrap_or(u64::MAX), Ordering::Relaxed);
    RESAMPLE_FILL_NANOS.fetch_add(u64::try_from(fill_d.as_nanos()).unwrap_or(u64::MAX), Ordering::Relaxed);
    RESAMPLE_WRITE_NANOS.fetch_add(u64::try_from(write_d.as_nanos()).unwrap_or(u64::MAX), Ordering::Relaxed);
    RESAMPLE_PRUNE_NANOS.fetch_add(u64::try_from(prune_d.as_nanos()).unwrap_or(u64::MAX), Ordering::Relaxed);
    // 仅供本地排查；真机 stderr 不进 hilog，真值走 ABI 顶层 `perf`（见 `transform_perf`）。
    eprintln!(
        "[PC-RESAMPLE] px={} threads={} read={:.1}ms fill={:.1}ms write={:.1}ms prune={:.1}ms parallel={}",
        dw * dh,
        rayon_threads(),
        read_ms,
        fill_ms,
        write_ms,
        prune_ms,
        u8::from(parallel)
    );
    out
}

/// 当前 rayon 全局池线程数；wasm 无 rayon 池，按 0 上报。
#[cfg(not(target_arch = "wasm32"))]
fn rayon_threads() -> usize {
    rayon::current_num_threads()
}

#[cfg(target_arch = "wasm32")]
fn rayon_threads() -> usize {
    0
}

/// 本次 `pc_layer_transform` 期间的重采样性能（相对入口起始值的差值）。
///
/// 真机 `eprintln!` 不进 hilog，因此性能数值经 ABI 顶层 `perf` 返回，由 ArkTS 侧打日志。
/// `pc_layer_transform` 入口处的重采样累加器快照，用于按调用前后差值上报 `perf`。
#[derive(Clone, Copy)]
struct ResampleSnapshot {
    calls: u64,
    pixels: u64,
    nanos: u64,
    read_nanos: u64,
    fill_nanos: u64,
    write_nanos: u64,
    prune_nanos: u64,
}

fn resample_snapshot() -> ResampleSnapshot {
    ResampleSnapshot {
        calls: RESAMPLE_CALLS.load(Ordering::Relaxed),
        pixels: RESAMPLE_PIXELS.load(Ordering::Relaxed),
        nanos: RESAMPLE_NANOS.load(Ordering::Relaxed),
        read_nanos: RESAMPLE_READ_NANOS.load(Ordering::Relaxed),
        fill_nanos: RESAMPLE_FILL_NANOS.load(Ordering::Relaxed),
        write_nanos: RESAMPLE_WRITE_NANOS.load(Ordering::Relaxed),
        prune_nanos: RESAMPLE_PRUNE_NANOS.load(Ordering::Relaxed),
    }
}

fn transform_perf(t_start: Instant, base: ResampleSnapshot) -> Value {
    let calls = RESAMPLE_CALLS.load(Ordering::Relaxed).saturating_sub(base.calls);
    let pixels = RESAMPLE_PIXELS.load(Ordering::Relaxed).saturating_sub(base.pixels);
    let nanos = RESAMPLE_NANOS.load(Ordering::Relaxed).saturating_sub(base.nanos);
    let read_nanos = RESAMPLE_READ_NANOS.load(Ordering::Relaxed).saturating_sub(base.read_nanos);
    let fill_nanos = RESAMPLE_FILL_NANOS.load(Ordering::Relaxed).saturating_sub(base.fill_nanos);
    let write_nanos = RESAMPLE_WRITE_NANOS.load(Ordering::Relaxed).saturating_sub(base.write_nanos);
    let prune_nanos = RESAMPLE_PRUNE_NANOS.load(Ordering::Relaxed).saturating_sub(base.prune_nanos);
    json!({
        "totalMs": t_start.elapsed().as_secs_f64() * 1_000.0,
        "resampleMs": nanos as f64 / 1e6,
        "readMs": read_nanos as f64 / 1e6,
        "fillMs": fill_nanos as f64 / 1e6,
        "writeMs": write_nanos as f64 / 1e6,
        "pruneMs": prune_nanos as f64 / 1e6,
        "resamplePixels": pixels,
        "resampleCalls": calls,
        "threads": rayon_threads(),
    })
}

/// 大图保护：任何会被重采样的表面（内容 / 渲染缓存 / 蒙版 / 视频帧）输出超限即拒绝。
fn transform_exceeds_limit(l: &Layer, a: &Affine) -> bool {
    let too_big = |s: &Surface| {
        let b = turn_map_rect(a, s.content_bounds());
        u64::from(b.width()) * u64::from(b.height()) > MAX_TRANSFORM_PIXELS
    };
    if l.mask.as_ref().is_some_and(|m| too_big(&m.surface)) {
        return true;
    }
    if l.video.as_ref().is_some_and(|v| v.frames.iter().chain(v.original_frames.iter().flatten()).any(too_big)) {
        return true;
    }
    match &l.content {
        LayerContent::Raster(s) => too_big(s),
        LayerContent::Text(t) => t.cache.as_ref().is_some_and(too_big),
        LayerContent::Shape(sh) => sh.cache.as_ref().is_some_and(too_big),
        LayerContent::Smart(sm) => sm.cache.as_ref().is_some_and(too_big) || sm.filter_mask.as_ref().is_some_and(|m| too_big(&m.surface)),
        LayerContent::Group(g) => g.children.iter().any(|c| transform_exceeds_limit(c, a)),
        _ => false,
    }
}

/// 把像素（含文字/形状/智能对象的渲染缓存、蒙版、视频帧）按同一仿射重采样一次。
///
/// 新的表面只写 `dest` 包围盒内的像素，包围盒本身已由仿射算出，所以视觉位置不跳、不需要
/// 额外的平移补偿。
fn transform_layer_pixels(l: &mut Layer, a: &Affine, inv: &Affine) {
    let dest = |s: &Surface| turn_map_rect(a, s.content_bounds());
    if let Some(mask) = l.mask.as_mut() {
        let target = dest(&mask.surface);
        mask.surface = resample_affine(&mask.surface, inv, target);
    }
    if let Some(video) = l.video.as_mut() {
        for frame in video.frames.iter_mut().chain(video.original_frames.iter_mut().flatten()) {
            let target = dest(frame);
            *frame = resample_affine(frame, inv, target);
        }
    }
    match &mut l.content {
        LayerContent::Raster(s) => {
            let target = dest(s);
            *s = resample_affine(s, inv, target);
        }
        LayerContent::Text(t) => {
            if let Some(cache) = t.cache.as_mut() {
                let target = dest(cache);
                *cache = resample_affine(cache, inv, target);
            }
        }
        LayerContent::Shape(sh) => {
            if let Some(cache) = sh.cache.as_mut() {
                let target = dest(cache);
                *cache = resample_affine(cache, inv, target);
            }
        }
        LayerContent::Smart(sm) => {
            if let Some(cache) = sm.cache.as_mut() {
                let target = dest(cache);
                *cache = resample_affine(cache, inv, target);
            }
            if let Some(mask) = sm.filter_mask.as_mut() {
                let target = dest(&mask.surface);
                mask.surface = resample_affine(&mask.surface, inv, target);
            }
        }
        LayerContent::Group(g) => {
            for child in &mut g.children {
                transform_layer_pixels(child, a, inv);
            }
        }
        _ => {}
    }
    l.fill_cache = None;
}

/// 智能对象透视矩阵（行主序 3×3）与仿射组合：`P' = A × P`。
///
/// 投影除法与仿射可交换（`A` 的第三行是 `[0, 0, 1]`），所以矩阵乘积就是「先透视再仿射」。
fn compose_perspective(p: &mut [f64; 9], a: &Affine) {
    let q = *p;
    let m = a.m;
    p[0] = m[0] * q[0] + m[2] * q[3] + m[4] * q[6];
    p[1] = m[0] * q[1] + m[2] * q[4] + m[4] * q[7];
    p[2] = m[0] * q[2] + m[2] * q[5] + m[4] * q[8];
    p[3] = m[1] * q[0] + m[3] * q[3] + m[5] * q[6];
    p[4] = m[1] * q[1] + m[3] * q[4] + m[5] * q[7];
    p[5] = m[1] * q[2] + m[3] * q[5] + m[5] * q[8];
    p[6] = q[6];
    p[7] = q[7];
    p[8] = q[8];
}

/// 智能对象透视同步（组内递归）：`turn_layer_geometry` 不处理透视矩阵。
fn transform_smart_perspective(l: &mut Layer, a: &Affine) {
    match &mut l.content {
        LayerContent::Smart(sm) => {
            if let Some(p) = sm.perspective.as_mut() {
                compose_perspective(p, a);
            }
        }
        LayerContent::Group(g) => {
            for child in &mut g.children {
                transform_smart_perspective(child, a);
            }
        }
        _ => {}
    }
}

/// 图层自由变换（`C-XFORM-API@1`）：相对当前状态的缩放 / 旋转 / 平移，一次提交一次 checkpoint。
///
/// 语义：`scaleX/Y` 是倍率、`angle` 是度数增量、`dx/dy` 是像素增量；缩放与旋转共用图层内容
/// 包围盒中心，组合顺序为「先缩放、再旋转、最后平移」。像素层做**一次**双线性重采样；矢量 /
/// 文字 / 智能对象 / 组只变换几何（文字不重排，形状的参数化 `live` 按既有 `turn_layer_geometry`
/// 规则降级为 `None`）；蒙版与内容同步变换；不支持的 Adjustment / Fill 直接拒绝。
#[unsafe(no_mangle)]
pub extern "C" fn pc_layer_transform(session: *mut Session, id: u64, layer: u64, params_ptr: *const u8, params_len: usize) -> *mut PcBuffer {
    guard(move || {
        let t_start = Instant::now();

        let base_resample = resample_snapshot();
        if session.is_null() {
            return fail("null session");
        }
        let raw = match unsafe { slice(params_ptr, params_len) } {
            Ok(bytes) => bytes,
            Err(e) => return fail(e),
        };
        // 解析失败与数值越界共用同一文案（§2.3），UI 只需判断一个错误分支。
        let params = match serde_json::from_slice::<TransformParams>(raw) {
            Ok(p) => p,
            Err(_) => return fail("invalid transform"),
        };
        let (sx, sy, angle, dx, dy) = (
            params.scale_x.unwrap_or(1.0),
            params.scale_y.unwrap_or(1.0),
            params.angle.unwrap_or(0.0),
            params.dx.unwrap_or(0.0),
            params.dy.unwrap_or(0.0),
        );
        let in_range = |v: f64, lo: f64, hi: f64| v.is_finite() && (lo..=hi).contains(&v);
        if !in_range(sx, 0.01, 100.0)
            || !in_range(sy, 0.01, 100.0)
            || !in_range(angle, -3600.0, 3600.0)
            || !in_range(dx, -100_000.0, 100_000.0)
            || !in_range(dy, -100_000.0, 100_000.0)
        {
            return fail("invalid transform");
        }
        // SAFETY: live session pointer.
        let s = unsafe { &mut *session };
        let entry = match s.docs.get_mut(&id) {
            Some(e) => e,
            None => return fail(format!("document {id} is not open")),
        };
        let bounds = match find_layer(&entry.doc.layers, layer) {
            None => return fail("layer not found"),
            Some(target) => {
                if !matches!(
                    &target.content,
                    LayerContent::Raster(_) | LayerContent::Group(_) | LayerContent::Text(_) | LayerContent::Shape(_) | LayerContent::Smart(_)
                ) {
                    return fail("layer cannot be transformed");
                }
                transform_content_bounds(target)
            }
        };
        // 无实际变化（§2.5）：不写、不 checkpoint、revision 不变。
        if sx == 1.0 && sy == 1.0 && angle == 0.0 && dx == 0.0 && dy == 0.0 {
            let perf = transform_perf(t_start, base_resample);
            let data = json!({ "bounds": [bounds.x0, bounds.y0, bounds.x1, bounds.y1], "scaleX": sx, "scaleY": sy, "angle": angle });
            return ok_perf(data, perf);
        }
        // 缩放与旋转共用内容包围盒中心；无内容的图层退化到文档中心。
        let center = if bounds.is_empty() {
            let doc = entry.doc.bounds();
            [(f64::from(doc.x0) + f64::from(doc.x1)) / 2.0, (f64::from(doc.y0) + f64::from(doc.y1)) / 2.0]
        } else {
            [(f64::from(bounds.x0) + f64::from(bounds.x1)) / 2.0, (f64::from(bounds.y0) + f64::from(bounds.y1)) / 2.0]
        };
        // `then` = 先做 self 再做参数：T(-c) → 缩放 → 旋转 → T(c) → 平移。
        let a = Affine::translate(-center[0], -center[1])
            .then(&Affine { m: [sx, 0.0, 0.0, sy, 0.0, 0.0] })
            .then(&Affine::rotate(angle.to_radians()))
            .then(&Affine::translate(center[0], center[1]))
            .then(&Affine::translate(dx, dy));
        let Some(inv) = a.inverse() else {
            return fail("invalid transform");
        };
        // 大图保护：校验通过但输出过大的请求也必须在 checkpoint 之前失败。
        if let Some(target) = find_layer(&entry.doc.layers, layer)
            && transform_exceeds_limit(target, &a)
        {
            return fail("transform too large");
        }
        let mapped = turn_map_rect(&a, bounds);
        // 唯一写前快照点：解析、数值校验、类型判定、无变化判定与大图保护全部完成。
        entry.checkpoint();
        if let Some(target) = find_layer_mut(&mut entry.doc.layers, layer) {
            turn_layer_geometry(target, &a);
            transform_smart_perspective(target, &a);
            transform_layer_pixels(target, &a, &inv);
        }
        let perf = transform_perf(t_start, base_resample);
        let data = json!({ "bounds": [mapped.x0, mapped.y0, mapped.x1, mapped.y1], "scaleX": sx, "scaleY": sy, "angle": angle });
        ok_perf(data, perf)
    })
}

/// 桥接层 C ABI 回归测试（计划 T4）。
///
/// 全部用例走真实导出的 C ABI（`pc_open` / `pc_info` / `pc_layer_effects_*` /
/// `pc_revision` / `pc_render*` / `pc_undo` / `pc_redo`），不修改任何生产逻辑。
/// 少数场景需要构造“未暴露的效果类型”（如内阴影）作为前置状态，此时直接读取
/// 同 crate 内的 `Document`（`doc::Effect` 的公开结构），随后仍用 C ABI 断言结果。
#[cfg(test)]
mod effects_api_tests {
    //! T4：图层样式与纯像素渲染的 C ABI 行为测试。
    #![allow(clippy::panic, clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

    use super::*;

    /// 渐变叠加的烘焙曲线记录：桥接层不输出，只在测试里预置以验证"替换色标时清空"。
    use photocraft_doc::effects::GradientInterpolation;
    /// `clear` 保留语义的 `unsupported` 样本（缎光）。
    use photocraft_doc::Satin;


    /// 仓库自带的 JPEG 样图（2400x1500），足以覆盖 16x16 的渲染区域。
    const SAMPLE_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../entry/src/main/resources/rawfile/sample.jpg");

    // ---------------------------------------------------------------- 基础设施

    /// 读取并释放一个 `PcBuffer`。失败信息包含调用上下文。
    fn take_bytes(buf: *mut PcBuffer, ctx: &str) -> Vec<u8> {
        assert!(!buf.is_null(), "{ctx}: 期望非空 PcBuffer, 实际 null");
        // SAFETY: `buf` 由桥接层 `PcBuffer::from_vec` 分配，读取后立即释放且只释放一次。
        // 测试模块与 `PcBuffer` 同属一个 crate，按任务允许的方式直接读取 ptr/len。
        let bytes = unsafe {
            let b = &*buf;
            std::slice::from_raw_parts(b.ptr, b.len).to_vec()
        };
        pc_buffer_free(buf);
        bytes
    }

    /// 读取 JSON 响应并释放缓冲区。
    fn read_json(buf: *mut PcBuffer, ctx: &str) -> Value {
        let bytes = take_bytes(buf, ctx);
        match serde_json::from_slice::<Value>(&bytes) {
            Ok(value) => value,
            Err(e) => {
                let text = String::from_utf8_lossy(&bytes).into_owned();
                panic!("{ctx}: 期望合法 JSON, 实际解析失败 ({e}): {text}")
            }
        }
    }

    fn assert_ok(state: &Value, ctx: &str) {
        assert_eq!(state["ok"].as_bool(), Some(true), "{ctx}: 期望 ok=true, 实际 {state}");
    }

    fn assert_fail(state: &Value, ctx: &str, needle: &str) {
        assert_eq!(state["ok"].as_bool(), Some(false), "{ctx}: 期望 ok=false, 实际 {state}");
        let error = state["error"].as_str().unwrap_or("<缺失>");
        assert!(
            error.contains(needle),
            "{ctx}: 期望 error 包含 {needle:?}, 实际 {error:?}（完整响应 {state}）"
        );
    }

    fn assert_num_field(item: &Value, field: &str, expected: f32, op: &str, index: usize) {
        let ctx = format!("op={op} index={index} 字段 {field}");
        match item.get(field).and_then(Value::as_f64) {
            Some(actual) if ((actual as f32) - expected).abs() <= 1e-6 => {}
            Some(actual) => panic!("{ctx}: 期望 {expected}, 实际 {actual}"),
            None => panic!("{ctx}: 期望 {expected}, 实际字段缺失（item={item}）"),
        }
    }

    fn assert_str_field(item: &Value, field: &str, expected: &str, op: &str, index: usize) {
        let ctx = format!("op={op} index={index} 字段 {field}");
        match item.get(field).and_then(Value::as_str) {
            Some(actual) if actual == expected => {}
            Some(actual) => panic!("{ctx}: 期望 {expected:?}, 实际 {actual:?}"),
            None => panic!("{ctx}: 期望 {expected:?}, 实际字段缺失（item={item}）"),
        }
    }

    fn assert_bool_field(item: &Value, field: &str, expected: bool, op: &str, index: usize) {
        let ctx = format!("op={op} index={index} 字段 {field}");
        match item.get(field).and_then(Value::as_bool) {
            Some(actual) if actual == expected => {}
            Some(actual) => panic!("{ctx}: 期望 {expected}, 实际 {actual}"),
            None => panic!("{ctx}: 期望 {expected}, 实际字段缺失（item={item}）"),
        }
    }

    fn assert_color_field(item: &Value, expected: [f64; 4], op: &str, index: usize) {
        let ctx = format!("op={op} index={index} 字段 color");
        let array = match item.get("color").and_then(Value::as_array) {
            Some(array) => array,
            None => panic!("{ctx}: 期望 {expected:?}, 实际字段缺失（item={item}）"),
        };
        assert_eq!(array.len(), 4, "{ctx}: 期望长度 4, 实际 {array:?}");
        for (channel, want) in expected.iter().enumerate() {
            let got = array[channel].as_f64().unwrap_or(f64::NAN);
            assert!(
                (got - want).abs() <= 1e-6,
                "{ctx}: 期望 color[{channel}]={want}, 实际 {got}（color={array:?}）"
            );
        }
    }

    /// 断言任意颜色数组字段（如 `highlightColor` / `shadowColor`）等于期望值。
    fn assert_color_array_field(item: &Value, field: &str, expected: [f64; 4], op: &str, index: usize) {
        let ctx = format!("op={op} index={index} 字段 {field}");
        let array = match item.get(field).and_then(Value::as_array) {
            Some(array) => array,
            None => panic!("{ctx}: 期望 {expected:?}, 实际字段缺失（item={item}）"),
        };
        assert_eq!(array.len(), 4, "{ctx}: 期望长度 4, 实际 {array:?}");
        for (channel, want) in expected.iter().enumerate() {
            let got = array[channel].as_f64().unwrap_or(f64::NAN);
            assert!(
                (got - want).abs() <= 1e-6,
                "{ctx}: 期望 {field}[{channel}]={want}, 实际 {got}（{field}={array:?}）"
            );
        }
    }

    /// 断言对象字段（如 `gradient`）下确实没有 `inner`：契约要求未暴露字段整体省略。
    fn assert_absent_nested(item: &Value, outer: &str, inner: &str, op: &str, index: usize) {
        let ctx = format!("op={op} index={index} 字段 {outer}.{inner}");
        match item.get(outer).and_then(Value::as_object) {
            Some(map) => assert!(map.get(inner).is_none(), "{ctx}: 期望字段省略, 实际存在（item={item}）"),
            None => panic!("{ctx}: 期望 {outer} 为对象, 实际缺失或类型错误（item={item}）"),
        }
    }

    /// 读取 `gradient.stops` 数组。
    fn gradient_stops<'a>(item: &'a Value, op: &str, index: usize) -> &'a Vec<Value> {
        item.get("gradient")
            .and_then(|gradient| gradient.get("stops"))
            .and_then(Value::as_array)
            .unwrap_or_else(|| panic!("op={op} index={index}: 期望 gradient.stops 为数组, 实际 {item}"))
    }

    /// 契约要求字段按效果类型省略；这里断言确实没有该键。
    fn assert_absent(item: &Value, field: &str, op: &str, index: usize) {
        let ctx = format!("op={op} index={index} 字段 {field}");
        assert!(item.get(field).is_none(), "{ctx}: 期望字段省略, 实际存在（item={item}）");
    }

    fn items_of<'a>(state: &'a Value, op: &str, index: usize) -> &'a Vec<Value> {
        state["data"]["items"]
            .as_array()
            .unwrap_or_else(|| panic!("op={op} index={index}: 期望 data.items 为数组, 实际 {state}"))
    }

    fn assert_item_meta(items: &[Value], index: usize, kind: &str, op: &str) {
        let item = &items[index];
        assert_eq!(
            item["index"].as_u64(),
            Some(index as u64),
            "op={op} index={index} 字段 index: 期望 {index}, 实际 {}",
            item["index"]
        );
        assert_eq!(
            item["kind"].as_str(),
            Some(kind),
            "op={op} index={index} 字段 kind: 期望 {kind}, 实际 {}",
            item["kind"]
        );
    }

    struct Fixture {
        session: *mut Session,
        doc: u64,
        layer: u64,
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            pc_session_free(self.session);
        }
    }

    impl Fixture {
        fn apply(&self, params: &str) -> Value {
            let bytes = params.as_bytes();
            read_json(
                pc_layer_effects_apply(self.session, self.doc, self.layer, bytes.as_ptr(), bytes.len()),
                &format!("pc_layer_effects_apply {params}"),
            )
        }

        fn apply_ok(&self, params: &str) -> Value {
            let state = self.apply(params);
            assert_ok(&state, params);
            state
        }

        fn apply_for_layer(&self, layer: u64, params: &str) -> Value {
            let bytes = params.as_bytes();
            read_json(
                pc_layer_effects_apply(self.session, self.doc, layer, bytes.as_ptr(), bytes.len()),
                &format!("pc_layer_effects_apply layer={layer} {params}"),
            )
        }

        fn get(&self) -> Value {
            let state = read_json(pc_layer_effects_get(self.session, self.doc, self.layer), "pc_layer_effects_get");
            assert_ok(&state, "pc_layer_effects_get");
            state
        }

        fn info(&self) -> Value {
            let state = read_json(pc_info(self.session, self.doc), "pc_info");
            assert_ok(&state, "pc_info");
            state
        }

        fn revision(&self) -> u64 {
            let state = read_json(pc_revision(self.session, self.doc), "pc_revision");
            assert_ok(&state, "pc_revision");
            state["data"]["revision"]
                .as_u64()
                .unwrap_or_else(|| panic!("op=revision index=n/a: 期望 data.revision 为 u64, 实际 {state}"))
        }

        fn undo(&self) -> Value {
            let state = read_json(pc_undo(self.session, self.doc), "pc_undo");
            assert_ok(&state, "pc_undo");
            state
        }

        fn redo(&self) -> Value {
            let state = read_json(pc_redo(self.session, self.doc), "pc_redo");
            assert_ok(&state, "pc_redo");
            state
        }

        /// 直接读取文档数据：用于验证 JSON 未输出的内部字段（如 `use_global_light`）。
        fn with_layer<R>(&self, f: impl FnOnce(&Layer) -> R) -> R {
            // SAFETY: 测试独占该 session，且没有 C ABI 调用同时进行。
            let session = unsafe { &*self.session };
            let entry = session.docs.get(&self.doc).unwrap_or_else(|| panic!("文档 {} 不存在", self.doc));
            let layer = find_layer(&entry.doc.layers, self.layer).unwrap_or_else(|| panic!("图层 {} 不存在", self.layer));
            f(layer)
        }

        /// 当前图层上所有投影实例的 `use_global_light`，按实例顺序。
        fn drop_shadow_global_light(&self) -> Vec<bool> {
            self.with_layer(|layer| {
                layer
                    .effects
                    .items
                    .iter()
                    .filter_map(|effect| match effect {
                        Effect::DropShadow(shadow) => Some(shadow.use_global_light),
                        _ => None,
                    })
                    .collect()
            })
        }

        /// 前置状态：追加一个桥接层不暴露的图案叠加，用于验证 `clear` 的保留语义。
        /// （`C-EFFECTS-API@2` 起内阴影属于可编辑项，会被 `clear` 删除，不能再当保留样本。）
        fn push_pattern_overlay(&self) {
            // SAFETY: 见 `with_layer`；此处需要可变借用。
            let session = unsafe { &mut *self.session };
            let entry = session.docs.get_mut(&self.doc).expect("文档必须存在");
            let layer = find_layer_mut(&mut entry.doc.layers, self.layer).expect("图层必须存在");
            layer.effects.items.push(Effect::PatternOverlay {
                common: FxCommon::new(BlendMode::Normal, 1.0),
                name: "Bricks".to_string(),
                id: "bricks".to_string(),
                scale: 100.0,
                angle: 0.0,
                link: true,
                phase: (0.0, 0.0),
            });
        }

        /// 可变访问文档数据：用于构造桥接层不暴露的样本与预置 `interpolation` 记录。
        fn with_layer_mut<R>(&self, f: impl FnOnce(&mut Layer) -> R) -> R {
            // SAFETY: 测试独占该 session，且没有 C ABI 调用同时进行。
            let session = unsafe { &mut *self.session };
            let entry = session.docs.get_mut(&self.doc).unwrap_or_else(|| panic!("文档 {} 不存在", self.doc));
            let layer = find_layer_mut(&mut entry.doc.layers, self.layer).unwrap_or_else(|| panic!("图层 {} 不存在", self.layer));
            f(layer)
        }

        /// 前置状态：追加一个桥接层不暴露的颜色叠加（`unsupported`），用于验证 `clear` 的保留语义。
        fn push_color_overlay(&self) {
            self.with_layer_mut(|layer| {
                layer.effects.items.push(Effect::ColorOverlay {
                    common: FxCommon::new(BlendMode::Normal, 1.0),
                    color: Color::rgb(1.0, 0.0, 0.0),
                });
            });
        }

        /// 前置状态：追加一个桥接层不暴露的缎光（`unsupported`），用于验证 `clear` 的保留语义。
        fn push_satin(&self) {
            self.with_layer_mut(|layer| {
                layer.effects.items.push(Effect::Satin(Satin {
                    common: FxCommon::new(BlendMode::Normal, 1.0),
                    color: Color::rgb(0.5, 0.5, 0.5),
                    angle: 19.0,
                    distance: 11.0,
                    size: 14.0,
                    contour: Contour::Linear,
                    anti_alias: false,
                    invert: false,
                }));
            });
        }
    }

    /// 递归找第一个可编辑（非组）图层。
    fn first_layer_id(layers: &Value) -> Option<u64> {
        let list = layers.as_array()?;
        for node in list {
            if node["isGroup"].as_bool() == Some(false)
                && let Some(id) = node["id"].as_u64()
            {
                return Some(id);
            }
        }
        for node in list {
            if let Some(id) = first_layer_id(&node["children"]) {
                return Some(id);
            }
        }
        None
    }

    fn open_fixture() -> Fixture {
        let raw = std::fs::read(SAMPLE_PATH).unwrap_or_else(|e| panic!("读取样图失败 {SAMPLE_PATH}: {e}"));
        let session = pc_session_new();
        assert!(!session.is_null(), "op=sessionNew: 期望非空 session, 实际 null");
        let name = b"sample.jpg";
        let opened = read_json(
            pc_open(session, name.as_ptr(), name.len(), raw.as_ptr(), raw.len()),
            "pc_open",
        );
        assert_ok(&opened, "op=open");
        let doc = opened["data"]["id"]
            .as_u64()
            .unwrap_or_else(|| panic!("op=open index=n/a: 期望 data.id, 实际 {opened}"));
        let info = read_json(pc_info(session, doc), "pc_info");
        assert_ok(&info, "op=info");
        let layer = first_layer_id(&info["data"]["layers"])
            .unwrap_or_else(|| panic!("op=info index=n/a: 样图中没有可编辑图层, 实际 {info}"));
        Fixture { session, doc, layer }
    }

    // ---------------------------------------------------------------- 用例

    /// R4.2 / 契约默认状态：新打开的文档没有效果实例，总开关为开。
    #[test]
    fn initial_state_is_empty_and_enabled() {
        let f = open_fixture();
        let state = f.get();
        assert_ok(&state, "op=get index=n/a");
        assert_eq!(
            state["data"]["enabled"].as_bool(),
            Some(true),
            "op=get index=n/a 字段 enabled: 期望 true, 实际 {}",
            state["data"]["enabled"]
        );
        let items = items_of(&state, "get", 0);
        assert_eq!(items.len(), 0, "op=get index=n/a: 期望初始 items 为空, 实际 {items:?}");
        assert_eq!(f.revision(), 0, "op=revision index=n/a: 期望新文档 revision=0, 实际 {}", f.revision());
    }

    /// R1.1 / R2.1 / R3.1 + 计划 2.2 默认值表逐项校验。
    #[test]
    fn add_defaults_match_contract() {
        let f = open_fixture();
        // 投影
        let state = f.apply_ok(r#"{"op":"add","kind":"dropShadow"}"#);
        let items = items_of(&state, "add", 0);
        assert_eq!(items.len(), 1, "op=add index=0: 期望 1 个实例, 实际 {items:?}");
        let shadow = &items[0];
        assert_item_meta(items, 0, "dropShadow", "add");
        assert_str_field(shadow, "label", "Drop Shadow", "add", 0);
        assert_bool_field(shadow, "enabled", true, "add", 0);
        assert_str_field(shadow, "blend", "Multiply", "add", 0);
        assert_num_field(shadow, "opacity", 0.75, "add", 0);
        assert_color_field(shadow, [0.0, 0.0, 0.0, 1.0], "add", 0);
        assert_num_field(shadow, "angle", 120.0, "add", 0);
        assert_num_field(shadow, "distance", 5.0, "add", 0);
        assert_num_field(shadow, "spread", 0.0, "add", 0);
        assert_num_field(shadow, "size", 5.0, "add", 0);
        assert_bool_field(shadow, "knocksOut", true, "add", 0);
        assert_bool_field(shadow, "antiAlias", false, "add", 0);
        assert_absent(shadow, "position", "add", 0);
        assert_absent(shadow, "technique", "add", 0);
        // `use_global_light` 不在 JSON 中，直接读取文档数据验证（否则角度滑块会被全局光覆盖）。
        assert_eq!(
            f.drop_shadow_global_light(),
            vec![false],
            "op=add index=0 字段 use_global_light: 期望 false, 实际 {:?}",
            f.drop_shadow_global_light()
        );

        // 描边
        let state = f.apply_ok(r#"{"op":"add","kind":"stroke"}"#);
        let items = items_of(&state, "add", 1);
        assert_eq!(items.len(), 2, "op=add index=1: 期望 2 个实例, 实际 {items:?}");
        let stroke = &items[1];
        assert_item_meta(items, 1, "stroke", "add");
        assert_str_field(stroke, "label", "Stroke", "add", 1);
        assert_bool_field(stroke, "enabled", true, "add", 1);
        assert_str_field(stroke, "blend", "Normal", "add", 1);
        assert_num_field(stroke, "opacity", 1.0, "add", 1);
        assert_color_field(stroke, [0.0, 0.0, 0.0, 1.0], "add", 1);
        assert_num_field(stroke, "size", 3.0, "add", 1);
        assert_str_field(stroke, "position", "outside", "add", 1);
        assert_absent(stroke, "angle", "add", 1);
        assert_absent(stroke, "distance", "add", 1);
        assert_absent(stroke, "spread", "add", 1);
        assert_absent(stroke, "technique", "add", 1);
        assert_absent(stroke, "knocksOut", "add", 1);

        // 外发光
        let state = f.apply_ok(r#"{"op":"add","kind":"outerGlow"}"#);
        let items = items_of(&state, "add", 2);
        assert_eq!(items.len(), 3, "op=add index=2: 期望 3 个实例, 实际 {items:?}");
        let glow = &items[2];
        assert_item_meta(items, 2, "outerGlow", "add");
        assert_str_field(glow, "label", "Outer Glow", "add", 2);
        assert_bool_field(glow, "enabled", true, "add", 2);
        assert_str_field(glow, "blend", "Screen", "add", 2);
        assert_num_field(glow, "opacity", 0.75, "add", 2);
        assert_color_field(glow, [1.0, 1.0, 1.0, 1.0], "add", 2);
        assert_num_field(glow, "spread", 0.0, "add", 2);
        assert_num_field(glow, "size", 5.0, "add", 2);
        assert_str_field(glow, "technique", "softer", "add", 2);
        assert_bool_field(glow, "antiAlias", false, "add", 2);
        assert_absent(glow, "angle", "add", 2);
        assert_absent(glow, "distance", "add", 2);
        assert_absent(glow, "position", "add", 2);
        assert_absent(glow, "knocksOut", "add", 2);

        // 三种实例按 add 顺序排列。
        assert_eq!(f.get()["data"]["items"].as_array().map(Vec::len), Some(3));
    }

    /// R1.4 / R2.2：update 只覆盖传入字段，其余保持原值；传 angle 时 use_global_light 仍为 false。
    #[test]
    fn update_only_overrides_provided_fields() {
        let f = open_fixture();
        f.apply_ok(r#"{"op":"add","kind":"dropShadow"}"#);
        let state = f.apply_ok(r#"{"op":"update","index":0,"size":12,"distance":7,"opacity":0.4}"#);
        let items = items_of(&state, "update", 0);
        let shadow = &items[0];
        assert_item_meta(items, 0, "dropShadow", "update");
        assert_num_field(shadow, "size", 12.0, "update", 0);
        assert_num_field(shadow, "distance", 7.0, "update", 0);
        assert_num_field(shadow, "opacity", 0.4, "update", 0);
        // 未传入字段保持默认值。
        assert_str_field(shadow, "blend", "Multiply", "update", 0);
        assert_color_field(shadow, [0.0, 0.0, 0.0, 1.0], "update", 0);
        assert_num_field(shadow, "angle", 120.0, "update", 0);
        assert_num_field(shadow, "spread", 0.0, "update", 0);
        assert_bool_field(shadow, "knocksOut", true, "update", 0);
        assert_bool_field(shadow, "antiAlias", false, "update", 0);

        // 只改角度：其他值不变，且 use_global_light 不会被改回 true。
        let state = f.apply_ok(r#"{"op":"update","index":0,"angle":-45}"#);
        let items = items_of(&state, "update", 0);
        assert_num_field(&items[0], "angle", -45.0, "update", 0);
        assert_num_field(&items[0], "size", 12.0, "update", 0);
        assert_num_field(&items[0], "distance", 7.0, "update", 0);
        assert_eq!(
            f.drop_shadow_global_light(),
            vec![false],
            "op=update index=0 字段 use_global_light: 期望 false, 实际 {:?}",
            f.drop_shadow_global_light()
        );

        // 描边：只改位置，宽度与颜色不变。
        f.apply_ok(r#"{"op":"add","kind":"stroke"}"#);
        let state = f.apply_ok(r#"{"op":"update","index":1,"position":"inside"}"#);
        let items = items_of(&state, "update", 1);
        assert_str_field(&items[1], "position", "inside", "update", 1);
        assert_num_field(&items[1], "size", 3.0, "update", 1);
        assert_color_field(&items[1], [0.0, 0.0, 0.0, 1.0], "update", 1);
    }

    /// R4.4：remove 后剩余实例按原顺序重排，返回的 index 是最新下标。
    #[test]
    fn remove_reindexes_remaining_items() {
        let f = open_fixture();
        f.apply_ok(r#"{"op":"add","kind":"dropShadow"}"#);
        f.apply_ok(r#"{"op":"add","kind":"stroke"}"#);
        f.apply_ok(r#"{"op":"add","kind":"outerGlow"}"#);

        let state = f.apply_ok(r#"{"op":"remove","index":1}"#);
        let items = items_of(&state, "remove", 1);
        assert_eq!(items.len(), 2, "op=remove index=1: 期望剩余 2 个实例, 实际 {items:?}");
        assert_item_meta(items, 0, "dropShadow", "remove");
        assert_item_meta(items, 1, "outerGlow", "remove");

        let state = f.get();
        let items = items_of(&state, "get", 0);
        assert_eq!(items.len(), 2, "op=get index=n/a: 期望 remove 后仍为 2 个实例, 实际 {items:?}");
        assert_item_meta(items, 0, "dropShadow", "get");
        assert_item_meta(items, 1, "outerGlow", "get");

        let state = f.apply_ok(r#"{"op":"remove","index":0}"#);
        let items = items_of(&state, "remove", 0);
        assert_eq!(items.len(), 1, "op=remove index=0: 期望剩余 1 个实例, 实际 {items:?}");
        assert_item_meta(items, 0, "outerGlow", "remove");
    }

    /// R4.3：enable 只影响目标实例，同类另一实例的参数与开关不变。
    #[test]
    fn enable_targets_only_one_instance() {
        let f = open_fixture();
        f.apply_ok(r#"{"op":"add","kind":"dropShadow"}"#);
        f.apply_ok(r#"{"op":"add","kind":"dropShadow"}"#);
        let state = f.apply_ok(r#"{"op":"enable","index":1,"enabled":false}"#);
        let items = items_of(&state, "enable", 1);
        assert_eq!(items.len(), 2, "op=enable index=1: 期望 2 个实例, 实际 {items:?}");
        assert_bool_field(&items[0], "enabled", true, "enable", 1);
        assert_bool_field(&items[1], "enabled", false, "enable", 1);

        // 持久化到文档，重新读取仍是同一状态。
        let persisted = f.get();
        let items = items_of(&persisted, "get", 0);
        assert_bool_field(&items[0], "enabled", true, "get", 0);
        assert_bool_field(&items[1], "enabled", false, "get", 1);
        // 关闭实例的参数不受影响。
        assert_num_field(&items[1], "size", 5.0, "get", 1);
        assert_num_field(&items[1], "opacity", 0.75, "get", 1);
    }

    /// R4.2：master 只切换 effects.enabled，各实例参数与自身开关不变。
    #[test]
    fn master_switch_keeps_instance_settings() {
        let f = open_fixture();
        f.apply_ok(r#"{"op":"add","kind":"dropShadow"}"#);
        f.apply_ok(r#"{"op":"update","index":0,"size":9}"#);

        let state = f.apply_ok(r#"{"op":"master","enabled":false}"#);
        assert_eq!(
            state["data"]["enabled"].as_bool(),
            Some(false),
            "op=master index=n/a 字段 enabled: 期望 false, 实际 {}",
            state["data"]["enabled"]
        );
        let items = items_of(&state, "master", 0);
        assert_eq!(items.len(), 1, "op=master index=n/a: 期望实例保留, 实际 {items:?}");
        assert_bool_field(&items[0], "enabled", true, "master", 0);
        assert_num_field(&items[0], "size", 9.0, "master", 0);

        let state = f.apply_ok(r#"{"op":"master","enabled":true}"#);
        assert_eq!(
            state["data"]["enabled"].as_bool(),
            Some(true),
            "op=master index=n/a 字段 enabled: 期望 true, 实际 {}",
            state["data"]["enabled"]
        );
        assert_num_field(&items_of(&state, "master", 0)[0], "size", 9.0, "master", 0);
    }

    /// R4.5 / D1：clear 删除 7 类可编辑效果，保留 unsupported 的图案叠加。
    ///
    /// `C-EFFECTS-API@2` 起内阴影属于可编辑项、`clear` 会删除它，因此"保留语义"的样本改用
    /// unsupported 的 `PatternOverlay`（@1 的 3 类删除集对旧文档仍逐项相同）。
    #[test]
    fn clear_removes_editable_kinds_only() {
        let f = open_fixture();
        f.apply_ok(r#"{"op":"add","kind":"dropShadow"}"#);
        f.apply_ok(r#"{"op":"add","kind":"stroke"}"#);
        f.apply_ok(r#"{"op":"add","kind":"outerGlow"}"#);
        // 前置状态：一个桥接层不暴露的图案叠加（模拟从 PSD 带入的既有样式）。
        f.push_pattern_overlay();
        assert_eq!(
            items_of(&f.get(), "get", 0).len(),
            4,
            "op=get index=n/a: 期望前置状态含 4 个实例, 实际 {:?}",
            f.get()["data"]["items"]
        );

        let state = f.apply_ok(r#"{"op":"clear"}"#);
        let items = items_of(&state, "clear", 0);
        assert_eq!(items.len(), 1, "op=clear index=n/a: 期望只保留 1 个不可编辑实例, 实际 {items:?}");
        assert_eq!(
            items[0]["kind"].as_str(),
            Some("unsupported"),
            "op=clear index=0 字段 kind: 期望 unsupported, 实际 {}",
            items[0]["kind"]
        );
        assert_str_field(&items[0], "label", "Pattern Overlay", "clear", 0);

        // 再次读取仍然一致，说明不是只在返回值上过滤。
        let persisted = f.get();
        let items = items_of(&persisted, "get", 0);
        assert_eq!(items.len(), 1, "op=get index=n/a: 期望 clear 后仅剩图案叠加, 实际 {items:?}");
        assert_eq!(items[0]["index"].as_u64(), Some(0));
    }

    /// R4.6 / 错误语义：所有失败都返回 ok:false，且不建立 checkpoint（revision 不变）。
    #[test]
    fn invalid_requests_fail_without_revision_bump() {
        let f = open_fixture();
        f.apply_ok(r#"{"op":"add","kind":"dropShadow"}"#);
        let baseline = f.revision();

        let cases: [(&str, &str); 12] = [
            (r#"{"op":"explode"}"#, "unsupported op"),
            (r#"{"op":"add","kind":"glow"}"#, "unsupported effect kind"),
            (r#"{"op":"add"}"#, "unsupported effect kind"),
            (r#"{"op":"update","index":99,"kind":"dropShadow","size":4}"#, "out of range"),
            (r#"{"op":"remove","index":99}"#, "out of range"),
            (r#"{"op":"enable","index":99,"enabled":false}"#, "out of range"),
            (r#"{"op":"update","index":0,"kind":"stroke","size":4}"#, "is not stroke"),
            (r#"{"op":"enable","index":0}"#, "missing enabled"),
            (r#"{"op":"master"}"#, "missing enabled"),
            (r#"{"op":"update","index":0,"blend":"Sparkle"}"#, "unsupported blend mode"),
            (r#"{"op":"update","index":0,"position":"middle"}"#, "unsupported position/technique"),
            (r#"{"op":"update","index":0,"technique":"blurry"}"#, "unsupported position/technique"),
        ];
        for (params, needle) in cases {
            let state = f.apply(params);
            assert_fail(&state, params, needle);
            let actual = f.revision();
            assert_eq!(
                actual, baseline,
                "op={params} index=n/a 字段 revision: 期望失败后保持 {baseline}, 实际 {actual}"
            );
        }

        // JSON 解析失败与图层不存在同样不得改文档。
        let state = f.apply("{\"op\":");
        assert_fail(&state, "invalid json", "invalid parameters");
        assert_eq!(f.revision(), baseline, "op=invalidJson index=n/a 字段 revision: 期望保持 {baseline}");

        let state = f.apply_for_layer(999_999, r#"{"op":"add","kind":"dropShadow"}"#);
        assert_fail(&state, "missing layer", "layer not found");
        assert_eq!(f.revision(), baseline, "op=add index=n/a 字段 revision: 期望图层不存在时保持 {baseline}");

        // 失败后状态未被破坏：仍只有一个实例。
        assert_eq!(
            items_of(&f.get(), "get", 0).len(),
            1,
            "op=get index=n/a: 期望失败操作未追加实例, 实际 {:?}",
            f.get()["data"]["items"]
        );
    }

    /// R6 / 契约 revision：成功 apply、undo、redo 各 +1；只读接口不递增。
    #[test]
    fn revision_increments_on_apply_undo_redo() {
        let f = open_fixture();
        let r0 = f.revision();
        assert_eq!(r0, 0, "op=revision index=n/a: 期望 0, 实际 {r0}");

        f.apply_ok(r#"{"op":"add","kind":"dropShadow"}"#);
        let r1 = f.revision();
        assert_eq!(r1, r0 + 1, "op=add index=0 字段 revision: 期望 {}, 实际 {r1}", r0 + 1);

        f.apply_ok(r#"{"op":"update","index":0,"size":9}"#);
        let r2 = f.revision();
        assert_eq!(r2, r1 + 1, "op=update index=0 字段 revision: 期望 {}, 实际 {r2}", r1 + 1);

        // 只读调用不改变版本号。
        let _ = f.get();
        let _ = f.info();
        let r3 = f.revision();
        assert_eq!(r3, r2, "op=get/info index=n/a 字段 revision: 期望保持 {r2}, 实际 {r3}");

        // undo → 回到 update 之前。
        f.undo();
        let r4 = f.revision();
        assert_eq!(r4, r3 + 1, "op=undo index=n/a 字段 revision: 期望 {}, 实际 {r4}", r3 + 1);
        assert_num_field(&items_of(&f.get(), "get", 0)[0], "size", 5.0, "get", 0);

        // redo → 再次应用 update。
        f.redo();
        let r5 = f.revision();
        assert_eq!(r5, r4 + 1, "op=redo index=n/a 字段 revision: 期望 {}, 实际 {r5}", r4 + 1);
        assert_num_field(&items_of(&f.get(), "get", 0)[0], "size", 9.0, "get", 0);

        // undo → 回到 update 之前（redo 后 undo 栈为 [空文档, 默认投影]）。
        f.undo();
        let r6 = f.revision();
        assert_eq!(r6, r5 + 1, "op=undo index=n/a 字段 revision: 期望 {}, 实际 {r6}", r5 + 1);
        assert_num_field(&items_of(&f.get(), "get", 0)[0], "size", 5.0, "get", 0);

        // 再 undo → 回到 add 之前，列表为空。
        f.undo();
        let r7 = f.revision();
        assert_eq!(r7, r6 + 1, "op=undo index=n/a 字段 revision: 期望 {}, 实际 {r7}", r6 + 1);
        let emptied = f.get();
        assert_eq!(
            items_of(&emptied, "get", 0).len(),
            0,
            "op=get index=n/a: 期望撤销 add 后无实例, 实际 {}",
            emptied["data"]["items"]
        );

        // 无历史可撤销：失败且版本号不变。
        let state = read_json(pc_undo(f.session, f.doc), "pc_undo");
        assert_fail(&state, "undo-empty", "nothing to undo");
        assert_eq!(f.revision(), r7, "op=undo index=n/a 字段 revision: 期望失败保持 {r7}");
    }

    /// R10.1 / R10.2：`pc_render_pixels` 返回裸 RGBA8，长度恰好 w*h*4。
    #[test]
    fn render_pixels_returns_bare_rgba() {
        let f = open_fixture();
        let mut out: *mut PcBuffer = std::ptr::null_mut();
        let state = read_json(pc_render_pixels(f.session, f.doc, 0, 0, 16, 16, &mut out), "pc_render_pixels");
        assert_ok(&state, "op=renderPixels index=n/a 16x16");
        assert!(!out.is_null(), "op=renderPixels index=n/a: 期望 out 非空, 实际 null");
        let pixels = take_bytes(out, "pc_render_pixels 16x16");
        assert_eq!(
            pixels.len(),
            16 * 16 * 4,
            "op=renderPixels index=n/a: 期望长度 1024, 实际 {}",
            pixels.len()
        );

        // 与带 8 字节头的 `pc_render` 同区域对比：内容应完全一致（仅少了头）。
        let mut raw: *mut PcBuffer = std::ptr::null_mut();
        let state = read_json(pc_render(f.session, f.doc, 0, 0, 16, 16, &mut raw), "pc_render");
        assert_ok(&state, "op=render index=n/a 16x16");
        let raw_bytes = take_bytes(raw, "pc_render 16x16");
        assert_eq!(
            raw_bytes.len(),
            16 * 16 * 4 + 8,
            "op=render index=n/a: 期望长度 1032（1024+8 头）, 实际 {}",
            raw_bytes.len()
        );
        assert_eq!(
            &raw_bytes[8..],
            &pixels[..],
            "op=renderPixels index=n/a: 期望纯像素与 pc_render 去掉 8 字节头后一致"
        );

        // 非法区域：失败且不写 out。
        let mut bad: *mut PcBuffer = std::ptr::null_mut();
        let state = read_json(pc_render_pixels(f.session, f.doc, 0, 0, 0, 16, &mut bad), "pc_render_pixels 0x16");
        assert_fail(&state, "op=renderPixels index=n/a w=0", "invalid render rectangle");
        assert!(bad.is_null(), "op=renderPixels index=n/a w=0: 期望 out 保持 null, 实际非空");

        // 版本号查询不影响文档；不存在的文档返回可读错误。
        assert_eq!(f.revision(), 0, "op=revision index=n/a: 期望渲染不改变 revision, 实际 {}", f.revision());
        let missing = read_json(pc_revision(f.session, f.doc + 999), "pc_revision missing");
        assert_fail(&missing, "op=revision index=n/a doc=missing", "is not open");
    }

    /// R12.5：颜色分量按调用方顺序写入与读出，不做通道交换；alpha 强制为 1。
    #[test]
    fn color_channels_are_not_swapped() {
        let f = open_fixture();
        // hexToRgba 的 BGR 红：分量顺序必须原样往返。
        let state = f.apply_ok(r#"{"op":"add","kind":"dropShadow","color":[0,0,1,1]}"#);
        assert_color_field(&items_of(&state, "add", 0)[0], [0.0, 0.0, 1.0, 1.0], "add", 0);

        let state = f.apply_ok(r#"{"op":"update","index":0,"color":[1,0,0,1]}"#);
        assert_color_field(&items_of(&state, "update", 0)[0], [1.0, 0.0, 0.0, 1.0], "update", 0);

        // alpha 输入被忽略，恒为 1.0。
        let state = f.apply_ok(r#"{"op":"update","index":0,"color":[0.2,0.4,0.6,0.3]}"#);
        assert_color_field(&items_of(&state, "update", 0)[0], [0.2, 0.4, 0.6, 1.0], "update", 0);

        // 描边与外发光同样不交换通道。
        let state = f.apply_ok(r#"{"op":"add","kind":"stroke","color":[0,0,1,1]}"#);
        assert_color_field(&items_of(&state, "add", 1)[1], [0.0, 0.0, 1.0, 1.0], "add", 1);
        let state = f.apply_ok(r#"{"op":"add","kind":"outerGlow","color":[0,1,0,1]}"#);
        assert_color_field(&items_of(&state, "add", 2)[2], [0.0, 1.0, 0.0, 1.0], "add", 2);

        // 重新读取仍保持相同顺序。
        let state = f.get();
        let items = items_of(&state, "get", 0);
        assert_color_field(&items[0], [0.2, 0.4, 0.6, 1.0], "get", 0);
        assert_color_field(&items[1], [0.0, 0.0, 1.0, 1.0], "get", 1);
        assert_color_field(&items[2], [0.0, 1.0, 0.0, 1.0], "get", 2);
    }

    // ---------------------------------------------------------------- C-EFFECTS-API@2 新增用例

    /// T3-① / 内阴影工厂默认值与 JSON 形状（`C-EFFECTS-API@2` §2.2 默认值表）。
    #[test]
    fn t3_inner_shadow_defaults_and_shape() {
        let f = open_fixture();
        let state = f.apply_ok(r#"{"op":"add","kind":"innerShadow"}"#);
        let items = items_of(&state, "add", 0);
        assert_eq!(items.len(), 1, "op=add index=0: 期望 1 个实例, 实际 {items:?}");
        let item = &items[0];
        assert_item_meta(items, 0, "innerShadow", "add");
        assert_str_field(item, "label", "Inner Shadow", "add", 0);
        assert_bool_field(item, "enabled", true, "add", 0);
        assert_str_field(item, "blend", "Multiply", "add", 0);
        assert_num_field(item, "opacity", 0.75, "add", 0);
        assert_color_field(item, [0.0, 0.0, 0.0, 1.0], "add", 0);
        assert_num_field(item, "angle", 120.0, "add", 0);
        assert_num_field(item, "distance", 5.0, "add", 0);
        assert_num_field(item, "spread", 0.0, "add", 0);
        assert_num_field(item, "size", 5.0, "add", 0);
        assert_bool_field(item, "antiAlias", false, "add", 0);
        // 契约：内阴影没有挖空语义，刻意不输出 `knocksOut`。
        assert_absent(item, "knocksOut", "add", 0);
        // `use_global_light` 不在 JSON 中：直接读文档数据验证（否则角度滑块会被全局光覆盖）。
        let global_light = f.with_layer(|layer| {
            layer
                .effects
                .items
                .iter()
                .filter_map(|effect| match effect {
                    Effect::InnerShadow(shadow) => Some(shadow.use_global_light),
                    _ => None,
                })
                .collect::<Vec<bool>>()
        });
        assert_eq!(
            global_light,
            vec![false],
            "op=add index=0 字段 use_global_light: 期望 false, 实际 {global_light:?}"
        );
    }

    /// T3-② / 内发光默认值（`source=edge` / `technique=softer`）与 `update` 后两项切换。
    #[test]
    fn t3_inner_glow_defaults_and_source_switch() {
        let f = open_fixture();
        let state = f.apply_ok(r#"{"op":"add","kind":"innerGlow"}"#);
        let items = items_of(&state, "add", 0);
        assert_eq!(items.len(), 1, "op=add index=0: 期望 1 个实例, 实际 {items:?}");
        let item = &items[0];
        assert_item_meta(items, 0, "innerGlow", "add");
        assert_str_field(item, "label", "Inner Glow", "add", 0);
        assert_bool_field(item, "enabled", true, "add", 0);
        assert_str_field(item, "blend", "Screen", "add", 0);
        assert_num_field(item, "opacity", 0.75, "add", 0);
        assert_color_field(item, [1.0, 1.0, 1.0, 1.0], "add", 0);
        assert_num_field(item, "spread", 0.0, "add", 0);
        assert_num_field(item, "size", 5.0, "add", 0);
        assert_str_field(item, "technique", "softer", "add", 0);
        assert_str_field(item, "source", "edge", "add", 0);
        assert_bool_field(item, "antiAlias", false, "add", 0);
        assert_absent(item, "angle", "add", 0);
        assert_absent(item, "distance", "add", 0);
        assert_absent(item, "knocksOut", "add", 0);

        // 只改 source/technique：其余字段必须保持 add 时的值（R6.4）。
        let state = f.apply_ok(r#"{"op":"update","index":0,"source":"center","technique":"precise"}"#);
        let items = items_of(&state, "update", 0);
        let item = &items[0];
        assert_str_field(item, "source", "center", "update", 0);
        assert_str_field(item, "technique", "precise", "update", 0);
        assert_str_field(item, "blend", "Screen", "update", 0);
        assert_num_field(item, "opacity", 0.75, "update", 0);
        assert_color_field(item, [1.0, 1.0, 1.0, 1.0], "update", 0);
        assert_num_field(item, "size", 5.0, "update", 0);
        assert_num_field(item, "spread", 0.0, "update", 0);
        assert_bool_field(item, "enabled", true, "update", 0);
    }

    /// T3-③ / 渐变叠加默认值、stops 按 `p` 升序写入、替换色标后 `interpolation` 清空。
    #[test]
    fn t3_gradient_overlay_defaults_and_stops_sorted() {
        let f = open_fixture();
        let state = f.apply_ok(r#"{"op":"add","kind":"gradientOverlay"}"#);
        let items = items_of(&state, "add", 0);
        assert_eq!(items.len(), 1, "op=add index=0: 期望 1 个实例, 实际 {items:?}");
        let item = &items[0];
        assert_item_meta(items, 0, "gradientOverlay", "add");
        assert_str_field(item, "label", "Gradient Overlay", "add", 0);
        assert_bool_field(item, "enabled", true, "add", 0);
        assert_str_field(item, "blend", "Normal", "add", 0);
        assert_num_field(item, "opacity", 1.0, "add", 0);
        assert_bool_field(item, "dither", false, "add", 0);
        // 颜色与角度都在 `gradient` 下，顶层没有这两个键。
        assert_absent(item, "color", "add", 0);
        assert_absent(item, "angle", "add", 0);

        let stops = gradient_stops(item, "add", 0);
        assert_eq!(stops.len(), 2, "op=add index=0 字段 gradient.stops: 期望 2 项, 实际 {stops:?}");
        assert_num_field(&stops[0], "p", 0.0, "add", 0);
        assert_color_field(&stops[0], [0.0, 0.0, 0.0, 1.0], "add", 0);
        assert_num_field(&stops[1], "p", 1.0, "add", 0);
        assert_color_field(&stops[1], [1.0, 1.0, 1.0, 1.0], "add", 0);

        let gradient = &item["gradient"];
        assert_str_field(gradient, "style", "linear", "add", 0);
        assert_num_field(gradient, "angle", 90.0, "add", 0);
        assert_num_field(gradient, "scale", 1.0, "add", 0);
        assert_bool_field(gradient, "reverse", false, "add", 0);
        assert_bool_field(gradient, "align", true, "add", 0);
        // 契约：gradient 下不输出 opacityStops / offset / interpolation。
        assert_absent_nested(item, "gradient", "opacityStops", "add", 0);
        assert_absent_nested(item, "gradient", "offset", "add", 0);
        assert_absent_nested(item, "gradient", "interpolation", "add", 0);

        // 预置一条烘焙曲线记录，验证"替换 stops 时清空 interpolation"（契约 §2.3.6）。
        f.with_layer_mut(|layer| {
            if let Some(Effect::GradientOverlay { interpolation, .. }) = layer.effects.items.get_mut(0) {
                *interpolation = Some(GradientInterpolation {
                    method: "linear".to_string(),
                    smoothness: 1.0,
                    stops: Vec::new(),
                });
            }
        });
        assert!(
            f.with_layer(|layer| matches!(
                layer.effects.items.first(),
                Some(Effect::GradientOverlay { interpolation: Some(_), .. })
            )),
            "op=preset index=0 字段 interpolation: 期望前置状态为 Some"
        );

        // 逆序写入（p=1 在前）：回读必须按 p 升序。
        let state = f.apply_ok(r#"{"op":"update","index":0,"gradientStops":[{"p":1,"color":[1,1,1,1]},{"p":0,"color":[0,0,0,1]}]}"#);
        let items = items_of(&state, "update", 0);
        let stops = gradient_stops(&items[0], "update", 0);
        assert_eq!(stops.len(), 2, "op=update index=0 字段 gradient.stops: 期望 2 项, 实际 {stops:?}");
        assert_num_field(&stops[0], "p", 0.0, "update", 0);
        assert_color_field(&stops[0], [0.0, 0.0, 0.0, 1.0], "update", 0);
        assert_num_field(&stops[1], "p", 1.0, "update", 0);
        assert_color_field(&stops[1], [1.0, 1.0, 1.0, 1.0], "update", 0);
        assert!(
            f.with_layer(|layer| matches!(
                layer.effects.items.first(),
                Some(Effect::GradientOverlay { interpolation: None, .. })
            )),
            "op=update index=0 字段 interpolation: 期望替换 stops 后为 None"
        );

        // 三项乱序（0.75 / 0.25 / 0.5）：整体替换并按 p 升序，中间色标不丢失。
        let state = f.apply_ok(r#"{"op":"update","index":0,"gradientStops":[{"p":0.75,"color":[0,1,0,1]},{"p":0.25,"color":[1,0,0,1]},{"p":0.5,"color":[0,0,1,1]}]}"#);
        let items = items_of(&state, "update", 0);
        let stops = gradient_stops(&items[0], "update", 0);
        assert_eq!(stops.len(), 3, "op=update index=0 字段 gradient.stops: 期望 3 项, 实际 {stops:?}");
        assert_num_field(&stops[0], "p", 0.25, "update", 0);
        assert_color_field(&stops[0], [1.0, 0.0, 0.0, 1.0], "update", 0);
        assert_num_field(&stops[1], "p", 0.5, "update", 0);
        assert_color_field(&stops[1], [0.0, 0.0, 1.0, 1.0], "update", 0);
        assert_num_field(&stops[2], "p", 0.75, "update", 0);
        assert_color_field(&stops[2], [0.0, 1.0, 0.0, 1.0], "update", 0);
        // 未出现的键保持 add 时的值。
        assert_str_field(&items[0]["gradient"], "style", "linear", "update", 0);
        assert_num_field(&items[0]["gradient"], "angle", 90.0, "update", 0);
    }

    /// T3-④ / 斜面与浮雕默认值，以及方向/深度/高光不透明度的单次更新。
    #[test]
    fn t3_bevel_defaults_and_direction() {
        let f = open_fixture();
        let state = f.apply_ok(r#"{"op":"add","kind":"bevelEmboss"}"#);
        let items = items_of(&state, "add", 0);
        assert_eq!(items.len(), 1, "op=add index=0: 期望 1 个实例, 实际 {items:?}");
        let item = &items[0];
        assert_item_meta(items, 0, "bevelEmboss", "add");
        assert_str_field(item, "label", "Bevel & Emboss", "add", 0);
        assert_bool_field(item, "enabled", true, "add", 0);
        assert_str_field(item, "style", "innerBevel", "add", 0);
        assert_str_field(item, "technique", "smooth", "add", 0);
        assert_num_field(item, "depth", 1.0, "add", 0);
        assert_bool_field(item, "up", true, "add", 0);
        assert_num_field(item, "size", 5.0, "add", 0);
        assert_num_field(item, "soften", 0.0, "add", 0);
        assert_num_field(item, "angle", 120.0, "add", 0);
        assert_num_field(item, "altitude", 30.0, "add", 0);
        assert_color_array_field(item, "highlightColor", [1.0, 1.0, 1.0, 1.0], "add", 0);
        assert_num_field(item, "highlightOpacity", 0.75, "add", 0);
        assert_color_array_field(item, "shadowColor", [0.0, 0.0, 0.0, 1.0], "add", 0);
        assert_num_field(item, "shadowOpacity", 0.75, "add", 0);
        assert_bool_field(item, "antiAlias", false, "add", 0);
        // Bevel 没有 `common`：顶层 blend/opacity 不输出。
        assert_absent(item, "blend", "add", 0);
        assert_absent(item, "opacity", "add", 0);

        // 全局光关闭：否则角度滑块会被文档全局角度覆盖。
        let global_light = f.with_layer(|layer| {
            layer
                .effects
                .items
                .iter()
                .filter_map(|effect| match effect {
                    Effect::BevelEmboss(bevel) => Some(bevel.use_global_light),
                    _ => None,
                })
                .collect::<Vec<bool>>()
        });
        assert_eq!(
            global_light,
            vec![false],
            "op=add index=0 字段 use_global_light: 期望 false, 实际 {global_light:?}"
        );

        let state = f.apply_ok(r#"{"op":"update","index":0,"up":false,"depth":3.5,"highlightOpacity":0.4}"#);
        let items = items_of(&state, "update", 0);
        let item = &items[0];
        assert_bool_field(item, "up", false, "update", 0);
        assert_num_field(item, "depth", 3.5, "update", 0);
        assert_num_field(item, "highlightOpacity", 0.4, "update", 0);
        // 未出现的键保持默认值。
        assert_str_field(item, "style", "innerBevel", "update", 0);
        assert_str_field(item, "technique", "smooth", "update", 0);
        assert_num_field(item, "size", 5.0, "update", 0);
        assert_num_field(item, "soften", 0.0, "update", 0);
        assert_num_field(item, "angle", 120.0, "update", 0);
        assert_num_field(item, "altitude", 30.0, "update", 0);
        assert_num_field(item, "shadowOpacity", 0.75, "update", 0);
        assert_color_array_field(item, "shadowColor", [0.0, 0.0, 0.0, 1.0], "update", 0);
    }

    /// T3-⑤ / 新增字段的非法值在 checkpoint 前返回，`revision` 与 items 均不变（R8.2）。
    #[test]
    fn t3_invalid_new_fields_fail_without_revision_bump() {
        let f = open_fixture();
        f.apply_ok(r#"{"op":"add","kind":"innerShadow"}"#);
        f.apply_ok(r#"{"op":"add","kind":"innerGlow"}"#);
        f.apply_ok(r#"{"op":"add","kind":"gradientOverlay"}"#);
        f.apply_ok(r#"{"op":"add","kind":"bevelEmboss"}"#);
        let baseline = f.revision();
        let baseline_items = items_of(&f.get(), "get", 0).clone();

        let cases: [(&str, &str); 6] = [
            (r#"{"op":"update","index":1,"source":"sideways"}"#, "effects: unsupported glow source: sideways"),
            (r#"{"op":"update","index":2,"gradientStyle":"swirl"}"#, "effects: unsupported gradient style: swirl"),
            (r#"{"op":"update","index":3,"bevelStyle":"wave"}"#, "effects: unsupported bevel style: wave"),
            (r#"{"op":"update","index":3,"bevelTechnique":"laser"}"#, "effects: unsupported bevel technique: laser"),
            (
                r#"{"op":"update","index":2,"gradientStops":[{"p":0,"color":[0,0,0,1]}]}"#,
                "effects: invalid gradient stops",
            ),
            // `update` 缺 `index`：validate 阶段失败，不发生 checkpoint。
            (r#"{"op":"update","source":"center"}"#, "effect index out of range: missing"),
        ];
        for (params, needle) in cases {
            let state = f.apply(params);
            assert_fail(&state, params, needle);
            let actual = f.revision();
            assert_eq!(
                actual, baseline,
                "op={params} index=n/a 字段 revision: 期望失败后保持 {baseline}, 实际 {actual}"
            );
        }

        // 失败后文档内容完整：四个实例与 add 时逐字段一致。
        let persisted = f.get();
        let items = items_of(&persisted, "get", 0);
        assert_eq!(items.len(), 4, "op=get index=n/a: 期望失败后仍为 4 个实例, 实际 {items:?}");
        assert_eq!(items, &baseline_items, "op=get index=n/a: 期望失败后 items 与基线一致");
    }

    /// T3-⑥ / 四个新 kind 各发一次只含单字段的 `update`：其余字段保持 add 时的值（R6.4）。
    #[test]
    fn t3_update_only_touches_present_fields_for_new_kinds() {
        let f = open_fixture();
        f.apply_ok(r#"{"op":"add","kind":"innerShadow"}"#);
        f.apply_ok(r#"{"op":"add","kind":"innerGlow"}"#);
        f.apply_ok(r#"{"op":"add","kind":"gradientOverlay"}"#);
        f.apply_ok(r#"{"op":"add","kind":"bevelEmboss"}"#);

        // 每类只提交一个字段。
        f.apply_ok(r#"{"op":"update","index":0,"size":12}"#);
        f.apply_ok(r#"{"op":"update","index":1,"spread":0.5}"#);
        f.apply_ok(r#"{"op":"update","index":2,"gradientScale":1.5}"#);
        f.apply_ok(r#"{"op":"update","index":3,"soften":2}"#);

        let persisted = f.get();
        let items = items_of(&persisted, "get", 0);
        assert_eq!(items.len(), 4, "op=get index=n/a: 期望 4 个实例, 实际 {items:?}");

        let inner_shadow = &items[0];
        assert_item_meta(items, 0, "innerShadow", "get");
        assert_num_field(inner_shadow, "size", 12.0, "get", 0);
        assert_str_field(inner_shadow, "blend", "Multiply", "get", 0);
        assert_num_field(inner_shadow, "opacity", 0.75, "get", 0);
        assert_color_field(inner_shadow, [0.0, 0.0, 0.0, 1.0], "get", 0);
        assert_num_field(inner_shadow, "angle", 120.0, "get", 0);
        assert_num_field(inner_shadow, "distance", 5.0, "get", 0);
        assert_num_field(inner_shadow, "spread", 0.0, "get", 0);

        let inner_glow = &items[1];
        assert_num_field(inner_glow, "spread", 0.5, "get", 1);
        assert_str_field(inner_glow, "source", "edge", "get", 1);
        assert_str_field(inner_glow, "technique", "softer", "get", 1);
        assert_num_field(inner_glow, "size", 5.0, "get", 1);
        assert_str_field(inner_glow, "blend", "Screen", "get", 1);
        assert_num_field(inner_glow, "opacity", 0.75, "get", 1);

        let gradient = &items[2];
        assert_num_field(&gradient["gradient"], "scale", 1.5, "get", 2);
        assert_num_field(&gradient["gradient"], "angle", 90.0, "get", 2);
        assert_str_field(&gradient["gradient"], "style", "linear", "get", 2);
        assert_bool_field(&gradient["gradient"], "reverse", false, "get", 2);
        assert_bool_field(&gradient["gradient"], "align", true, "get", 2);
        let stops = gradient_stops(gradient, "get", 2);
        assert_eq!(stops.len(), 2, "op=get index=2 字段 gradient.stops: 期望 2 项, 实际 {stops:?}");
        assert_num_field(&stops[0], "p", 0.0, "get", 2);
        assert_num_field(&stops[1], "p", 1.0, "get", 2);

        let bevel = &items[3];
        assert_num_field(bevel, "soften", 2.0, "get", 3);
        assert_num_field(bevel, "depth", 1.0, "get", 3);
        assert_bool_field(bevel, "up", true, "get", 3);
        assert_num_field(bevel, "angle", 120.0, "get", 3);
        assert_num_field(bevel, "altitude", 30.0, "get", 3);
        assert_num_field(bevel, "highlightOpacity", 0.75, "get", 3);
        assert_num_field(bevel, "shadowOpacity", 0.75, "get", 3);
    }

    /// T3-⑦ / `clear` 删除 7 类可编辑效果，保留 Pattern/Color Overlay 与 Satin（unsupported）。
    #[test]
    fn t3_clear_removes_seven_kinds_and_keeps_unsupported() {
        let f = open_fixture();
        for kind in ["dropShadow", "stroke", "outerGlow", "innerShadow", "innerGlow", "gradientOverlay", "bevelEmboss"] {
            let params = format!(r#"{{"op":"add","kind":"{kind}"}}"#);
            f.apply_ok(&params);
        }
        // 手工追加三类桥接层不暴露的效果（模拟从 PSD 带入的既有样式）。
        f.push_pattern_overlay();
        f.push_color_overlay();
        f.push_satin();
        assert_eq!(
            items_of(&f.get(), "get", 0).len(),
            10,
            "op=get index=n/a: 期望前置状态含 10 个实例, 实际 {:?}",
            f.get()["data"]["items"]
        );

        let before = f.revision();
        let state = f.apply_ok(r#"{"op":"clear"}"#);
        let items = items_of(&state, "clear", 0);
        assert_eq!(items.len(), 3, "op=clear index=n/a: 期望只保留 3 个不可编辑实例, 实际 {items:?}");
        for (position, label) in ["Pattern Overlay", "Color Overlay", "Satin"].into_iter().enumerate() {
            assert_eq!(
                items[position]["kind"].as_str(),
                Some("unsupported"),
                "op=clear index={position} 字段 kind: 期望 unsupported, 实际 {}",
                items[position]["kind"]
            );
            assert_str_field(&items[position], "label", label, "clear", position);
        }
        let after = f.revision();
        assert_eq!(after, before + 1, "op=clear index=n/a 字段 revision: 期望 {}, 实际 {after}", before + 1);

        // 持久化后一致：不是只在返回值上过滤。
        let cleared = f.get();
        let items = items_of(&cleared, "get", 0);
        assert_eq!(items.len(), 3, "op=get index=n/a: 期望 clear 后仅剩 3 个不可编辑实例, 实际 {items:?}");
        for (position, label) in ["Pattern Overlay", "Color Overlay", "Satin"].into_iter().enumerate() {
            assert_str_field(&items[position], "label", label, "get", position);
        }
    }

    /// T3-⑧ / `add` 新效果后 revision 递增，`undo` 回到 add 前再递增，`redo` 恢复实例。
    #[test]
    fn t3_revision_and_undo_restore_new_effect() {
        let f = open_fixture();
        let r0 = f.revision();
        f.apply_ok(r#"{"op":"add","kind":"gradientOverlay"}"#);
        let r1 = f.revision();
        assert_eq!(r1, r0 + 1, "op=add index=0 字段 revision: 期望 {}, 实际 {r1}", r0 + 1);
        assert_eq!(items_of(&f.get(), "get", 0).len(), 1, "op=get index=n/a: 期望 add 后 1 个实例");

        f.undo();
        let r2 = f.revision();
        assert_eq!(r2, r1 + 1, "op=undo index=n/a 字段 revision: 期望 {}, 实际 {r2}", r1 + 1);
        assert_eq!(
            items_of(&f.get(), "get", 0).len(),
            0,
            "op=get index=n/a: 期望 undo 后无实例, 实际 {:?}",
            f.get()["data"]["items"]
        );

        f.redo();
        let r3 = f.revision();
        assert_eq!(r3, r2 + 1, "op=redo index=n/a 字段 revision: 期望 {}, 实际 {r3}", r2 + 1);
        let restored = f.get();
        let items = items_of(&restored, "get", 0);
        assert_eq!(items.len(), 1, "op=get index=n/a: 期望 redo 后恢复 1 个实例, 实际 {items:?}");
        assert_item_meta(items, 0, "gradientOverlay", "get");
        let stops = gradient_stops(&items[0], "get", 0);
        assert_eq!(stops.len(), 2, "op=get index=0 字段 gradient.stops: 期望 2 项, 实际 {stops:?}");
        assert_str_field(&items[0]["gradient"], "style", "linear", "get", 0);
    }

    /// T3-⑨ / 新增四类颜色 BGR 直通：JSON 原样往返，文档内 `Color.c` 保持调用方分量顺序。
    #[test]
    fn t3_colour_channels_are_not_swapped_for_new_kinds() {
        let f = open_fixture();
        f.apply_ok(r#"{"op":"add","kind":"innerShadow","color":[0,0,1,1]}"#);
        f.apply_ok(r#"{"op":"add","kind":"innerGlow","color":[0,0,1,1]}"#);
        f.apply_ok(r#"{"op":"add","kind":"bevelEmboss","highlightColor":[0,0,1,1],"shadowColor":[0,0,1,1]}"#);

        // JSON 回读完全相等（alpha 恒为 1.0）。
        let readback = f.get();
        let items = items_of(&readback, "get", 0);
        assert_eq!(items.len(), 3, "op=get index=n/a: 期望 3 个实例, 实际 {items:?}");
        assert_color_field(&items[0], [0.0, 0.0, 1.0, 1.0], "get", 0);
        assert_color_field(&items[1], [0.0, 0.0, 1.0, 1.0], "get", 1);
        assert_color_array_field(&items[2], "highlightColor", [0.0, 0.0, 1.0, 1.0], "get", 2);
        assert_color_array_field(&items[2], "shadowColor", [0.0, 0.0, 1.0, 1.0], "get", 2);

        // 文档内部：输入按调用方顺序写进 `c[0..3]`，不做通道交换（蓝保存在 c[2]），alpha=1.0。
        let stored = f.with_layer(|layer| {
            let mut out: Vec<(&'static str, [f32; 4], f32)> = Vec::new();
            for effect in &layer.effects.items {
                match effect {
                    Effect::InnerShadow(shadow) => out.push(("innerShadow.color", shadow.color.c, shadow.color.alpha)),
                    Effect::InnerGlow(glow) => {
                        if let FxPaint::Color(color) = &glow.paint {
                            out.push(("innerGlow.color", color.c, color.alpha));
                        }
                    }
                    Effect::BevelEmboss(bevel) => {
                        out.push(("bevelEmboss.highlightColor", bevel.highlight_color.c, bevel.highlight_color.alpha));
                        out.push(("bevelEmboss.shadowColor", bevel.shadow_color.c, bevel.shadow_color.alpha));
                    }
                    _ => {}
                }
            }
            out
        });
        assert_eq!(stored.len(), 4, "op=get index=n/a: 期望 4 条内部颜色, 实际 {stored:?}");
        for (field, actual, alpha) in stored {
            assert_eq!(
                actual,
                [0.0, 0.0, 1.0, 0.0],
                "op=get index=n/a 字段 {field}: 期望 [0, 0, 1, 0]（蓝在 c[2]）, 实际 {actual:?}"
            );
            assert_eq!(alpha, 1.0, "op=get index=n/a 字段 {field}.alpha: 期望 1.0, 实际 {alpha}");
        }
    }

    /// T3-⑩ / `gradientStops` 形状非法（非数组/空/缺 color/缺 p/仅 1 项/超上限）一律拒绝且不 checkpoint。
    #[test]
    fn t3_gradient_stops_reject_bad_shapes() {
        let f = open_fixture();
        f.apply_ok(r#"{"op":"add","kind":"gradientOverlay"}"#);
        let baseline = f.revision();

        let cases: [&str; 5] = [
            // 非数组
            r#"{"op":"update","index":0,"gradientStops":"nope"}"#,
            // 空数组（<2 项）
            r#"{"op":"update","index":0,"gradientStops":[]}"#,
            // 元素缺 color
            r#"{"op":"update","index":0,"gradientStops":[{"p":0},{"p":1}]}"#,
            // 元素缺 p
            r#"{"op":"update","index":0,"gradientStops":[{"color":[0,0,0,1]},{"color":[1,1,1,1]}]}"#,
            // 仅 1 项
            r#"{"op":"update","index":0,"gradientStops":[{"p":0,"color":[0,0,0,1]}]}"#,
        ];
        for params in cases {
            let state = f.apply(params);
            assert_fail(&state, params, "effects: invalid gradient stops");
            let actual = f.revision();
            assert_eq!(
                actual, baseline,
                "op={params} index=0 字段 revision: 期望失败后保持 {baseline}, 实际 {actual}"
            );
        }

        // 超上限（257 项 > 256 项）同样拒绝，且不 checkpoint。
        let too_many = serde_json::json!({
            "op": "update",
            "index": 0,
            "gradientStops": (0..257)
                .map(|i| serde_json::json!({ "p": f64::from(i) / 256.0, "color": [0, 0, 0, 1] }))
                .collect::<Vec<Value>>(),
        })
        .to_string();
        let state = f.apply(&too_many);
        assert_fail(&state, "gradientStops 257 项", "effects: invalid gradient stops");
        assert_eq!(
            f.revision(),
            baseline,
            "op=update gradientStops 257 项 字段 revision: 期望失败后保持 {baseline}, 实际 {}",
            f.revision()
        );

        // 失败后色标未被破坏：仍是 add 时的两项 0/1。
        let persisted = f.get();
        let items = items_of(&persisted, "get", 0);
        let stops = gradient_stops(&items[0], "get", 0);
        assert_eq!(stops.len(), 2, "op=get index=0 字段 gradient.stops: 期望 2 项, 实际 {stops:?}");
        assert_num_field(&stops[0], "p", 0.0, "get", 0);
        assert_num_field(&stops[1], "p", 1.0, "get", 0);
    }

    // ---------------------------------------------------------------- T3-EXT 渲染生效

    /// 形状承载层的矩形 `(x, y, w, h)`：落在 2400x1500 画布正中央、明显小于画布。
    const CARRIER_RECT: (i32, i32, i32, i32) = (800, 450, 800, 600);
    /// 渲染区域在形状包围盒外扩的像素数：既要覆盖形状内部的边缘带（内阴影 / 内发光 /
    /// 斜面浮雕的着力点），也要覆盖外侧背景以观察合成结果。
    const CARRIER_MARGIN: i32 = 40;
    /// 判定阈值：差异像素数至少占渲染区域像素的 0.5%，避免"只差几个抗锯齿像素"被误判成渲染生效。
    const MIN_CHANGED_RATIO: f64 = 0.005;

    /// 一块渲染区域的逐像素差异统计。
    struct RegionDiff {
        /// 区域像素总数（即基线像素数）。
        pixels: usize,
        /// 至少一个 RGBA 通道发生变化的像素数。
        changed: usize,
        /// 首个差异像素：线性索引、基线 RGBA、渲染 RGBA。
        first: Option<(usize, [u8; 4], [u8; 4])>,
    }

    impl RegionDiff {
        fn ratio(&self) -> f64 {
            if self.pixels == 0 { 0.0 } else { self.changed as f64 / self.pixels as f64 }
        }
    }

    /// 逐像素比较两份裸 RGBA8（任一通道不同即算差异）。
    fn diff_pixels(baseline: &[u8], after: &[u8], ctx: &str) -> RegionDiff {
        assert_eq!(
            baseline.len(),
            after.len(),
            "{ctx}: 期望两张像素长度一致, 实际 {} vs {}",
            baseline.len(),
            after.len()
        );
        assert_eq!(baseline.len() % 4, 0, "{ctx}: 期望像素字节数为 4 的倍数, 实际 {}", baseline.len());
        let pixels = baseline.len() / 4;
        let mut changed = 0usize;
        let mut first: Option<(usize, [u8; 4], [u8; 4])> = None;
        for index in 0..pixels {
            let base = [baseline[index * 4], baseline[index * 4 + 1], baseline[index * 4 + 2], baseline[index * 4 + 3]];
            let cur = [after[index * 4], after[index * 4 + 1], after[index * 4 + 2], after[index * 4 + 3]];
            if base != cur {
                changed += 1;
                if first.is_none() {
                    first = Some((index, base, cur));
                }
            }
        }
        RegionDiff { pixels, changed, first }
    }

    /// 把首个差异像素格式化成可定位的诊断文本（坐标 + 两侧 RGBA）。
    fn describe_first_pixel(first: Option<(usize, [u8; 4], [u8; 4])>, w: i32) -> String {
        match first {
            Some((index, base, cur)) => {
                let x = index as i32 % w.max(1);
                let y = index as i32 / w.max(1);
                format!("首个差异像素 (x={x}, y={y}) 基线 RGBA={base:?} 渲染 RGBA={cur:?}")
            }
            None => "首个差异像素: 无".to_string(),
        }
    }

    /// 渲染一块 1:1 区域并返回裸 RGBA8 字节；顺带断言 `pc_render_pixels` 的长度契约。
    fn render_region(f: &Fixture, x: i32, y: i32, w: i32, h: i32, ctx: &str) -> Vec<u8> {
        let mut out: *mut PcBuffer = std::ptr::null_mut();
        let state = read_json(pc_render_pixels(f.session, f.doc, x, y, w, h, &mut out), ctx);
        assert_ok(&state, ctx);
        assert!(!out.is_null(), "{ctx}: 期望 out 非空, 实际 null");
        let pixels = take_bytes(out, ctx);
        let expected = w as usize * h as usize * 4;
        assert_eq!(pixels.len(), expected, "{ctx}: 期望 {w}x{h} 的裸 RGBA 共 {expected} 字节, 实际 {}", pixels.len());
        pixels
    }

    /// 断言渲染结果真的被效果改变：差异像素数达到区域像素的 [`MIN_CHANGED_RATIO`]。
    ///
    /// 失败信息包含 kind、区域尺寸、基线像素数、差异像素数与占比、首个差异像素的坐标与两侧 RGBA。
    fn assert_effect_changed(kind: &str, w: i32, h: i32, baseline: &[u8], rendered: &[u8]) -> RegionDiff {
        let diff = diff_pixels(baseline, rendered, &format!("kind={kind} {w}x{h}"));
        let threshold = (diff.pixels as f64 * MIN_CHANGED_RATIO).ceil() as usize;
        assert!(
            diff.changed >= threshold.max(1),
            "kind={kind} 期望渲染像素改变 ≥ {:.2}%：区域 {w}x{h}，基线像素数 {}，实际差异 {} 像素（{:.4}%，阈值 {threshold} 像素）；{}",
            MIN_CHANGED_RATIO * 100.0,
            diff.pixels,
            diff.changed,
            diff.ratio() * 100.0,
            describe_first_pixel(diff.first, w)
        );
        diff
    }

    /// 断言效果移除后像素与无效果基线逐字节相等（不留残影）。
    fn assert_pixels_identical(kind: &str, w: i32, h: i32, baseline: &[u8], restored: &[u8]) {
        let diff = diff_pixels(baseline, restored, &format!("kind={kind} remove 后恢复性比较 {w}x{h}"));
        assert!(
            diff.changed == 0,
            "kind={kind} 期望 remove 后像素与基线逐字节相等：区域 {w}x{h}，基线像素数 {}，实际差异 {} 像素（{:.4}%）；{}",
            diff.pixels,
            diff.changed,
            diff.ratio() * 100.0,
            describe_first_pixel(diff.first, w)
        );
    }

    /// 在画布中央新建一个不透明灰色矩形形状图层，返回其图层 id。
    ///
    /// **为什么必须用形状图层而不是 Background**：`sample.jpg` 的 Background 铺满整张
    /// 2400x1500 画布，其不透明区域的边缘落在画布之外；而内阴影 / 内发光 / 斜面浮雕只沿
    /// 图层形状的边缘向内绘制，铺满画布的图层在可见区域里没有"内部边缘"，加效果后像素
    /// 不会有任何变化（设备端 T5 的观察点 2/3/4 无法定论正是踩到了这个陷阱）。
    /// 这里改用 800x600 的居中矩形作为效果载体，边界完整落在画布内、可见可测。
    fn create_carrier_shape(f: &Fixture) -> u64 {
        let (x, y, w, h) = CARRIER_RECT;
        // 中灰填充：向内变暗（内阴影 / 斜面的阴影面）与向内变亮（内发光 / 斜面的高光面）
        // 都能产生明显差异。
        let params = format!(r#"{{"kind":"rect","x":{x},"y":{y},"w":{w},"h":{h},"fill":[0.5,0.5,0.5,1]}}"#);
        let state = read_json(
            pc_shape_create(f.session, f.doc, params.as_ptr(), params.len()),
            &format!("pc_shape_create {params}"),
        );
        assert_ok(&state, &format!("op=shapeCreate {params}"));
        let layer = state["data"]["layerId"]
            .as_u64()
            .unwrap_or_else(|| panic!("op=shapeCreate 字段 data.layerId: 期望 u64, 实际 {state}"));
        assert_ne!(
            layer, f.layer,
            "op=shapeCreate 字段 data.layerId: 期望新建形状图层, 实际复用了 Background 图层 {layer}"
        );
        layer
    }

    /// T3-EXT / 四类新效果必须真实改变渲染像素，且 remove 后回到基线（无残影）。
    ///
    /// 与既有 21 个用例的区别：那些只验证参数读写与 revision，没有任何一条证明效果**真的
    /// 改了像素**。本用例在 host 侧用真实 C ABI 走完整链路
    /// `pc_shape_create → pc_layer_effects_apply(add) → pc_render_pixels → 逐像素比对`，
    /// 给出不依赖设备截图的确定性证据。
    #[test]
    fn t3_ext_new_effects_change_rendered_pixels() {
        let f = open_fixture();
        // 用形状图层承载效果（原因见 `create_carrier_shape` 的注释）。
        let carrier = create_carrier_shape(&f);
        let revision_after_carrier = f.revision();

        let (sx, sy, sw, sh) = CARRIER_RECT;
        let (rx, ry, rw, rh) =
            (sx - CARRIER_MARGIN, sy - CARRIER_MARGIN, sw + 2 * CARRIER_MARGIN, sh + 2 * CARRIER_MARGIN);
        let baseline = render_region(&f, rx, ry, rw, rh, "baseline 形状区域");
        assert_eq!(
            f.revision(),
            revision_after_carrier,
            "op=renderPixels 字段 revision: 期望渲染是只读的、保持 {revision_after_carrier}, 实际 {}",
            f.revision()
        );

        let kinds: [(&str, &str); 4] = [
            ("innerShadow", r#"{"op":"add","kind":"innerShadow"}"#),
            ("innerGlow", r#"{"op":"add","kind":"innerGlow"}"#),
            ("gradientOverlay", r#"{"op":"add","kind":"gradientOverlay"}"#),
            ("bevelEmboss", r#"{"op":"add","kind":"bevelEmboss"}"#),
        ];
        for (kind, add_params) in kinds {
            let revision_before = f.revision();
            let state = f.apply_for_layer(carrier, add_params);
            assert_ok(&state, &format!("kind={kind} {add_params}"));
            assert_eq!(
                items_of(&state, "add", 0).len(),
                1,
                "kind={kind} op=add: 期望形状图层上有 1 个实例, 实际 {state}"
            );
            let revision_after_add = f.revision();
            assert_eq!(
                revision_after_add,
                revision_before + 1,
                "kind={kind} op=add 字段 revision: 期望 {}, 实际 {revision_after_add}",
                revision_before + 1
            );

            let rendered = render_region(&f, rx, ry, rw, rh, &format!("kind={kind} add 后渲染区域"));
            let diff = assert_effect_changed(kind, rw, rh, &baseline, &rendered);
            // `--nocapture` 时输出实测数值，便于取证；默认被测试框架捕获。
            eprintln!(
                "kind={kind} 区域={rw}x{rh} 基线像素数={} 差异像素数={} 占比={:.4}% {}",
                diff.pixels,
                diff.changed,
                diff.ratio() * 100.0,
                describe_first_pixel(diff.first, rw)
            );

            let removed = f.apply_for_layer(carrier, r#"{"op":"remove","index":0}"#);
            assert_ok(&removed, &format!("kind={kind} remove"));
            assert_eq!(
                items_of(&removed, "remove", 0).len(),
                0,
                "kind={kind} op=remove: 期望实例被删除, 实际 {removed}"
            );
            let revision_after_remove = f.revision();
            assert_eq!(
                revision_after_remove,
                revision_after_add + 1,
                "kind={kind} op=remove 字段 revision: 期望 {}, 实际 {revision_after_remove}",
                revision_after_add + 1
            );
            let restored = render_region(&f, rx, ry, rw, rh, &format!("kind={kind} remove 后渲染区域"));
            assert_pixels_identical(kind, rw, rh, &baseline, &restored);
        }

        // 渐变叠加跨整个图层（整帧），额外在未被形状遮挡的 Background 区域验证一次：
        // 若它只改了形状而不改背景，说明渐变没有按图层范围铺开。
        let (bx, by, bw, bh) = (120, 120, 200, 200);
        let background_baseline = render_region(&f, bx, by, bw, bh, "baseline Background 区域");
        let revision_before_overlay = f.revision();
        let state = f.apply_ok(r#"{"op":"add","kind":"gradientOverlay"}"#);
        assert_eq!(
            items_of(&state, "add", 0).len(),
            1,
            "kind=gradientOverlay/background op=add: 期望 Background 上有 1 个实例, 实际 {state}"
        );
        let revision_after_overlay = f.revision();
        assert_eq!(
            revision_after_overlay,
            revision_before_overlay + 1,
            "kind=gradientOverlay/background op=add 字段 revision: 期望 {}, 实际 {revision_after_overlay}",
            revision_before_overlay + 1
        );
        let background_rendered = render_region(&f, bx, by, bw, bh, "kind=gradientOverlay/background add 后");
        let background_diff =
            assert_effect_changed("gradientOverlay/background", bw, bh, &background_baseline, &background_rendered);
        eprintln!(
            "kind=gradientOverlay/background 区域={bw}x{bh} 基线像素数={} 差异像素数={} 占比={:.4}% {}",
            background_diff.pixels,
            background_diff.changed,
            background_diff.ratio() * 100.0,
            describe_first_pixel(background_diff.first, bw)
        );

        let removed = f.apply_ok(r#"{"op":"remove","index":0}"#);
        assert_ok(&removed, "kind=gradientOverlay/background remove");
        let background_restored = render_region(&f, bx, by, bw, bh, "kind=gradientOverlay/background remove 后");
        assert_pixels_identical(
            "gradientOverlay/background",
            bw,
            bh,
            &background_baseline,
            &background_restored,
        );
    }
}

// ===== 图层二次编辑 C ABI 回归测试（C-EDIT-TEST@1 / T3）=====
//
// 全部用例只走真实导出的 C ABI，不修改任何生产逻辑。像素证据统一用 `pc_render_pixels`
// 取裸 RGBA8 后**逐字节**比较：编辑必须改变渲染结果，撤销必须逐字节回到基线。
#[cfg(test)]
mod edit_api_tests {
    //! T3：`C-EDIT-API@1` 的 C ABI 回归测试（E1–E8）。
    #![allow(clippy::panic, clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

    use super::*;
    /// E8 直接构造"渐变填充"只读样本时需要（桥接层不提供创建渐变填充的入口）。
    use photocraft_doc::effects::GradientStyle;

    /// 仓库自带样图（2400x1500），与既有测试同一路径。
    const SAMPLE_PATH: &str =
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../entry/src/main/resources/rawfile/sample.jpg");
    /// E1 的文字渲染区域 `(x, y, w, h)`（`C-EDIT-TEST@1 §C`）。
    const TEXT_REGION: (i32, i32, i32, i32) = (120, 300, 520, 220);
    /// E2/E3/E5 的形状渲染区域 `(x, y, w, h)`（`C-EDIT-TEST@1 §C`）。
    const SHAPE_REGION: (i32, i32, i32, i32) = (260, 260, 300, 240);
    /// 浮点字段（颜色 / 几何）比较容差。
    const EPS: f64 = 1e-6;

    /// 向下合并：复制同内容层后合并，整图像素逐字节不变；撤销恢复两层，再撤销回一层；
    /// 对底层合并必须失败。
    #[test]
    fn merge_down_keeps_pixels_and_undo_restores() {
        let ctx = open_sample();
        let base = sample_base_layer(&ctx);
        let region = (0, 0, 2400, 1500);
        let before = ctx.render(region, "merge 基线");

        let dup = read_json(pc_layer_duplicate(ctx.session, ctx.doc, base), "pc_layer_duplicate");
        assert_ok(&dup, "pc_layer_duplicate");
        let copy = dup["data"]["layerId"]
            .as_u64()
            .unwrap_or_else(|| panic!("duplicate: 缺少 layerId: {dup}"));
        assert_eq!(
            ctx.info()["data"]["layers"].as_array().map(|items| items.len()),
            Some(2),
            "复制后图层数"
        );

        let merged = read_json(pc_layer_merge_down(ctx.session, ctx.doc, copy), "pc_layer_merge_down");
        assert_ok(&merged, "pc_layer_merge_down");
        assert_eq!(merged["data"]["layerId"].as_u64(), Some(base), "合并结果应保留底层 id");
        let info = ctx.info();
        let layers = info["data"]["layers"]
            .as_array()
            .unwrap_or_else(|| panic!("pc_info: layers 缺失: {info}"));
        assert_eq!(layers.len(), 1, "合并后图层数");
        assert_eq!(layers[0]["kind"].as_str(), Some("Pixel"), "合并结果类型");

        let after = ctx.render(region, "merge 后");
        assert!(before == after, "相同内容的向下合并应保持整图像素不变（len={}）", before.len());

        let undo = read_json(pc_undo(ctx.session, ctx.doc), "undo merge");
        assert_ok(&undo, "undo merge");
        assert_eq!(
            ctx.info()["data"]["layers"].as_array().map(|items| items.len()),
            Some(2),
            "撤销合并后图层数"
        );
        let undo2 = read_json(pc_undo(ctx.session, ctx.doc), "undo duplicate");
        assert_ok(&undo2, "undo duplicate");
        assert_eq!(
            ctx.info()["data"]["layers"].as_array().map(|items| items.len()),
            Some(1),
            "撤销复制后图层数"
        );

        let bottom = read_json(pc_layer_merge_down(ctx.session, ctx.doc, base), "pc_layer_merge_down bottom");
        assert_eq!(bottom["ok"].as_bool(), Some(false), "底层合并应失败: {bottom}");
        let error = bottom["error"].as_str().unwrap_or("<缺失>");
        assert!(error.contains("no layer below"), "底层合并错误应包含 no layer below, 实际 {error:?}");
    }

    /// 缩略图：尺寸不超过 side、保持比例、带 8 字节 header、至少有一个非透明像素。
    #[test]
    fn layer_thumbnail_scales_and_reports_size() {
        let ctx = open_sample();
        let base = sample_base_layer(&ctx);
        let mut out: *mut PcBuffer = std::ptr::null_mut();
        let state = read_json(
            pc_layer_thumbnail(ctx.session, ctx.doc, base, 48, &mut out),
            "pc_layer_thumbnail",
        );
        assert_ok(&state, "pc_layer_thumbnail");
        let bytes = take_bytes(out, "pc_layer_thumbnail bytes");
        assert!(bytes.len() > 8, "缩略图字节过短: {}", bytes.len());
        let width = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        let height = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        assert!(width > 0 && height > 0, "缩略图尺寸应为正: {width}x{height}");
        assert!(width <= 48 && height <= 48, "缩略图尺寸不应超过 48: {width}x{height}");
        assert_eq!(bytes.len(), 8 + (width * height * 4) as usize, "缩略图长度应与尺寸匹配");
        assert!(bytes[8..].iter().step_by(4).any(|a| *a > 0), "缩略图不应全透明");
    }

    // ---------------------------------------------------------------- 基础设施

    /// 读取并释放一个 `PcBuffer`。失败信息包含调用上下文。
    fn take_bytes(buf: *mut PcBuffer, ctx: &str) -> Vec<u8> {
        assert!(!buf.is_null(), "{ctx}: 期望非空 PcBuffer, 实际 null");
        // SAFETY: `buf` 由桥接层 `PcBuffer::from_vec` 分配，读取后立即释放且只释放一次。
        let bytes = unsafe {
            let b = &*buf;
            std::slice::from_raw_parts(b.ptr, b.len).to_vec()
        };
        pc_buffer_free(buf);
        bytes
    }

    /// 读取 JSON 响应并释放缓冲区。
    fn read_json(buf: *mut PcBuffer, ctx: &str) -> Value {
        let bytes = take_bytes(buf, ctx);
        match serde_json::from_slice::<Value>(&bytes) {
            Ok(value) => value,
            Err(e) => {
                let text = String::from_utf8_lossy(&bytes).into_owned();
                panic!("{ctx}: 期望合法 JSON, 实际解析失败 ({e}): {text}")
            }
        }
    }

    fn assert_ok(state: &Value, ctx: &str) {
        assert_eq!(state["ok"].as_bool(), Some(true), "{ctx}: 期望 ok=true, 实际 {state}");
    }

    /// 断言失败响应：`ok=false` 且 `error` 含指定片段（`C-EDIT-API@1 §D` 的错误文本）。
    fn assert_err(state: &Value, ctx: &str, needle: &str) {
        assert_eq!(state["ok"].as_bool(), Some(false), "{ctx}: 期望 ok=false, 实际 {state}");
        let error = state["error"].as_str().unwrap_or("<缺失>");
        assert!(
            error.contains(needle),
            "{ctx}: 期望 error 包含 {needle:?}, 实际 {error:?}（完整响应 {state}）"
        );
    }

    fn assert_str(item: &Value, field: &str, expected: &str, ctx: &str) {
        match item.get(field).and_then(Value::as_str) {
            Some(actual) if actual == expected => {}
            Some(actual) => panic!("{ctx}: 字段 {field} 期望 {expected:?}, 实际 {actual:?}"),
            None => panic!("{ctx}: 字段 {field} 期望 {expected:?}, 实际缺失（item={item}）"),
        }
    }

    fn assert_bool(item: &Value, field: &str, expected: bool, ctx: &str) {
        match item.get(field).and_then(Value::as_bool) {
            Some(actual) if actual == expected => {}
            Some(actual) => panic!("{ctx}: 字段 {field} 期望 {expected}, 实际 {actual}"),
            None => panic!("{ctx}: 字段 {field} 期望 {expected}, 实际缺失（item={item}）"),
        }
    }

    fn assert_num(item: &Value, field: &str, expected: f64, ctx: &str) {
        match item.get(field).and_then(Value::as_f64) {
            Some(actual) if (actual - expected).abs() <= EPS => {}
            Some(actual) => panic!("{ctx}: 字段 {field} 期望 {expected}, 实际 {actual}（item={item}）"),
            None => panic!("{ctx}: 字段 {field} 期望 {expected}, 实际缺失（item={item}）"),
        }
    }

    /// 断言 4 元颜色数组（`[b, g, r, a]`，逐项容差 [`EPS`]），不做任何通道交换。
    fn assert_color(item: &Value, field: &str, expected: [f32; 4], ctx: &str) {
        let array = match item.get(field).and_then(Value::as_array) {
            Some(array) => array,
            None => panic!("{ctx}: 字段 {field} 期望 {expected:?}, 实际缺失（item={item}）"),
        };
        assert_eq!(array.len(), 4, "{ctx}: 字段 {field} 期望长度 4, 实际 {array:?}");
        for (channel, want) in expected.iter().enumerate() {
            let got = array[channel].as_f64().unwrap_or(f64::NAN);
            assert!(
                (got - f64::from(*want)).abs() <= EPS,
                "{ctx}: 字段 {field}[{channel}] 期望 {want}, 实际 {got}（{field}={array:?}）"
            );
        }
    }

    // ---------------------------------------------------------------- 像素证据

    /// 一块区域的逐像素差异统计。
    struct PixelDiff {
        /// 区域像素总数（即基线像素数）。
        pixels: usize,
        /// 至少一个 RGBA 通道发生变化的像素数。
        changed: usize,
        /// 首个差异像素：线性索引、基线 RGBA、编辑后 RGBA。
        first: Option<(usize, [u8; 4], [u8; 4])>,
    }

    impl PixelDiff {
        fn ratio(&self) -> f64 {
            if self.pixels == 0 { 0.0 } else { self.changed as f64 / self.pixels as f64 }
        }
    }

    /// 两份裸 RGBA8 是否**逐字节**相等（恢复原状的判据，不做容差）。
    fn pixels_equal(baseline: &[u8], restored: &[u8]) -> bool {
        baseline == restored
    }

    /// 逐像素比较两份裸 RGBA8（任一通道不同即算差异）。
    fn diff_pixels(baseline: &[u8], after: &[u8], ctx: &str) -> PixelDiff {
        assert_eq!(
            baseline.len(),
            after.len(),
            "{ctx}: 期望两张像素长度一致, 实际 {} vs {}",
            baseline.len(),
            after.len()
        );
        assert_eq!(baseline.len() % 4, 0, "{ctx}: 期望像素字节数为 4 的倍数, 实际 {}", baseline.len());
        let pixels = baseline.len() / 4;
        let mut changed = 0usize;
        let mut first: Option<(usize, [u8; 4], [u8; 4])> = None;
        for index in 0..pixels {
            let base = [baseline[index * 4], baseline[index * 4 + 1], baseline[index * 4 + 2], baseline[index * 4 + 3]];
            let cur = [after[index * 4], after[index * 4 + 1], after[index * 4 + 2], after[index * 4 + 3]];
            if base != cur {
                changed += 1;
                if first.is_none() {
                    first = Some((index, base, cur));
                }
            }
        }
        PixelDiff { pixels, changed, first }
    }

    /// 把首个差异像素格式化成可定位的诊断文本（坐标 + 两侧 RGBA）。
    fn describe_first_pixel(first: Option<(usize, [u8; 4], [u8; 4])>, w: i32) -> String {
        match first {
            Some((index, base, cur)) => {
                let x = index as i32 % w.max(1);
                let y = index as i32 / w.max(1);
                format!("首个差异像素 (x={x}, y={y}) 基线 RGBA={base:?} 编辑后 RGBA={cur:?}")
            }
            None => "首个差异像素: 无".to_string(),
        }
    }

    /// 断言编辑真的反映到渲染结果（差异像素数 > 0）。
    fn assert_pixels_changed(ctx: &str, w: i32, baseline: &[u8], rendered: &[u8]) -> PixelDiff {
        let diff = diff_pixels(baseline, rendered, ctx);
        assert!(
            diff.changed > 0,
            "{ctx}: 期望编辑改变渲染像素（区域宽 {w}，基线像素数 {}），实际差异 0 像素；{}",
            diff.pixels,
            describe_first_pixel(diff.first, w)
        );
        diff
    }

    /// 断言撤销后像素与基线**逐字节**相等（不留残影）。
    fn assert_pixels_identical(ctx: &str, w: i32, baseline: &[u8], restored: &[u8]) {
        let diff = diff_pixels(baseline, restored, ctx);
        assert!(
            pixels_equal(baseline, restored),
            "{ctx}: 期望逐字节相同：区域宽 {w}，基线像素数 {}，实际差异 {} 像素（{:.4}%）；{}",
            diff.pixels,
            diff.changed,
            diff.ratio() * 100.0,
            describe_first_pixel(diff.first, w)
        );
    }

    /// 渲染一块 1:1 区域并返回裸 RGBA8，同时断言 `pc_render_pixels` 的长度契约。
    fn render_region(
        session: *mut Session,
        doc: u64,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        ctx: &str,
    ) -> Vec<u8> {
        let mut out: *mut PcBuffer = std::ptr::null_mut();
        let state = read_json(pc_render_pixels(session, doc, x, y, w, h, &mut out), ctx);
        assert_ok(&state, ctx);
        assert!(!out.is_null(), "{ctx}: 期望 out 非空, 实际 null");
        let pixels = take_bytes(out, ctx);
        let expected = w as usize * h as usize * 4;
        assert_eq!(pixels.len(), expected, "{ctx}: 期望 {w}x{h} 的裸 RGBA 共 {expected} 字节, 实际 {}", pixels.len());
        pixels
    }

    fn revision(session: *mut Session, doc: u64) -> u64 {
        let state = read_json(pc_revision(session, doc), "pc_revision");
        assert_ok(&state, "pc_revision");
        state["data"]["revision"]
            .as_u64()
            .unwrap_or_else(|| panic!("pc_revision: 期望 data.revision 为 u64, 实际 {state}"))
    }

    // ---------------------------------------------------------------- 测试上下文

    /// 一个已打开的样图文档；`Drop` 释放 session。
    struct Ctx {
        session: *mut Session,
        doc: u64,
    }

    impl Drop for Ctx {
        fn drop(&mut self) {
            pc_session_free(self.session);
        }
    }

    impl Ctx {
        fn revision(&self) -> u64 {
            revision(self.session, self.doc)
        }

        fn get(&self, layer: u64) -> Value {
            read_json(
                pc_layer_edit_get(self.session, self.doc, layer),
                &format!("pc_layer_edit_get layer={layer}"),
            )
        }

        fn apply(&self, layer: u64, params: &str) -> Value {
            read_json(
                pc_layer_edit_apply(
                    self.session,
                    self.doc,
                    layer,
                    params.as_bytes().as_ptr(),
                    params.len(),
                ),
                &format!("pc_layer_edit_apply layer={layer} {params}"),
            )
        }

        fn apply_ok(&self, layer: u64, params: &str) -> Value {
            let state = self.apply(layer, params);
            assert_ok(&state, params);
            state
        }

        fn undo(&self) -> Value {
            let state = read_json(pc_undo(self.session, self.doc), "pc_undo");
            assert_ok(&state, "pc_undo");
            state
        }

        fn info(&self) -> Value {
            let state = read_json(pc_info(self.session, self.doc), "pc_info");
            assert_ok(&state, "pc_info");
            state
        }

        /// 新建文字图层并返回其 id。
        fn create_text(&self, params: &str) -> u64 {
            let state = read_json(
                pc_text_create(self.session, self.doc, params.as_bytes().as_ptr(), params.len()),
                &format!("pc_text_create {params}"),
            );
            assert_ok(&state, &format!("pc_text_create {params}"));
            state["data"]["layerId"]
                .as_u64()
                .unwrap_or_else(|| panic!("pc_text_create: 期望 data.layerId, 实际 {state}"))
        }

        /// 新建形状图层并返回其 id。
        fn create_shape(&self, params: &str) -> u64 {
            let state = read_json(
                pc_shape_create(self.session, self.doc, params.as_bytes().as_ptr(), params.len()),
                &format!("pc_shape_create {params}"),
            );
            assert_ok(&state, &format!("pc_shape_create {params}"));
            state["data"]["layerId"]
                .as_u64()
                .unwrap_or_else(|| panic!("pc_shape_create: 期望 data.layerId, 实际 {state}"))
        }

        fn render(&self, region: (i32, i32, i32, i32), ctx: &str) -> Vec<u8> {
            let (x, y, w, h) = region;
            render_region(self.session, self.doc, x, y, w, h, ctx)
        }

        /// 直接可变访问文档数据：E4 需要"不可编辑图层"（Background）的 id，
        /// E8 需要构造桥接层不暴露的渐变填充。
        fn with_layer_mut<R>(&self, layer: u64, f: impl FnOnce(&mut Layer) -> R) -> R {
            // SAFETY: 测试独占该 session，且没有 C ABI 调用同时进行。
            let session = unsafe { &mut *self.session };
            let entry = session.docs.get_mut(&self.doc).unwrap_or_else(|| panic!("文档 {} 不存在", self.doc));
            let target =
                find_layer_mut(&mut entry.doc.layers, layer).unwrap_or_else(|| panic!("图层 {layer} 不存在"));
            f(target)
        }
    }

    /// 打开样图，返回独占的测试上下文。
    fn open_sample() -> Ctx {
        let raw = std::fs::read(SAMPLE_PATH).unwrap_or_else(|e| panic!("读取样图失败 {SAMPLE_PATH}: {e}"));
        let session = pc_session_new();
        assert!(!session.is_null(), "pc_session_new: 期望非空 session, 实际 null");
        let name = b"sample.jpg";
        let opened = read_json(
            pc_open(session, name.as_ptr(), name.len(), raw.as_ptr(), raw.len()),
            "pc_open",
        );
        assert_ok(&opened, "pc_open");
        let doc = opened["data"]["id"]
            .as_u64()
            .unwrap_or_else(|| panic!("pc_open: 期望 data.id, 实际 {opened}"));
        Ctx { session, doc }
    }

    /// 样图自带的图层（Background）id：用于验证"不可编辑图层"分支。
    ///
    /// `doc_info` 的 `layers` 是**顶层在前**的顺序，测试里新建的文字/形状层会排在前面，
    /// 因此这里从栈底找第一个非文字/非形状（即样图自带的 Pixel 图层）。
    fn sample_base_layer(ctx: &Ctx) -> u64 {
        let info = ctx.info();
        let layers = info["data"]["layers"]
            .as_array()
            .unwrap_or_else(|| panic!("pc_info: 期望 data.layers 为数组, 实际 {info}"));
        layers
            .iter()
            .rev()
            .find_map(|node| match node["kind"].as_str() {
                Some("Type") | Some("Shape") => None,
                _ => node["id"].as_u64(),
            })
            .unwrap_or_else(|| panic!("pc_info: 样图中没有不可编辑图层, 实际 {info}"))
    }

    // ---------------------------------------------------------------- 用例

    /// E1：文字内容 / 字号 / 字体 / 颜色编辑必须改变渲染像素；撤销后像素逐字节回到基线。
    #[test]
    fn e1_text_edit_reflects_in_rendered_pixels() {
        let ctx = open_sample();
        let layer = ctx.create_text(r#"{"text":"AAAA","size":96,"x":200,"y":400,"color":[1,1,1,1]}"#);
        let rev0 = ctx.revision();

        let before = ctx.get(layer);
        let data = &before["data"];
        assert_str(data, "kind", "Type", "e1 get");
        assert_str(data, "text", "AAAA", "e1 get");
        assert_num(data, "size", 96.0, "e1 get");
        assert_color(data, "color", [1.0, 1.0, 1.0, 1.0], "e1 get");

        let (_, _, w, _) = TEXT_REGION;
        let baseline = ctx.render(TEXT_REGION, "e1 基线");

        let state = ctx.apply_ok(layer, r#"{"text":"BBBB","size":128,"fontFamily":"Inter","color":[0,0.5,1,1]}"#);
        let rev1 = ctx.revision();
        assert_eq!(rev1, rev0 + 1, "e1 apply 字段 revision: 期望 {}, 实际 {rev1}", rev0 + 1);
        let after = &state["data"];
        assert_str(after, "text", "BBBB", "e1 apply 回读");
        assert_num(after, "size", 128.0, "e1 apply 回读");
        assert_str(after, "fontFamily", "Inter", "e1 apply 回读");
        assert_color(after, "color", [0.0, 0.5, 1.0, 1.0], "e1 apply 回读");

        let edited = ctx.render(TEXT_REGION, "e1 编辑后");
        let diff = assert_pixels_changed("e1 编辑后", w, &baseline, &edited);
        eprintln!(
            "e1 区域={}x{} 基线像素数={} 差异像素数={} 占比={:.4}% {}",
            TEXT_REGION.2,
            TEXT_REGION.3,
            diff.pixels,
            diff.changed,
            diff.ratio() * 100.0,
            describe_first_pixel(diff.first, w)
        );

        // 撤销：revision 再 +1，读模型与像素都回到基线。
        let undone = ctx.undo();
        assert_ok(&undone, "e1 undo");
        let rev2 = ctx.revision();
        assert_eq!(rev2, rev1 + 1, "e1 undo 字段 revision: 期望 {}, 实际 {rev2}", rev1 + 1);
        let restored = ctx.get(layer);
        assert_str(&restored["data"], "text", "AAAA", "e1 undo 后 get");
        assert_num(&restored["data"], "size", 96.0, "e1 undo 后 get");
        assert_color(&restored["data"], "color", [1.0, 1.0, 1.0, 1.0], "e1 undo 后 get");
        assert_eq!(restored, before, "e1 undo 后 get: 期望读模型逐字段恢复, 实际 {restored}");
        let restored_px = ctx.render(TEXT_REGION, "e1 undo 后");
        assert_pixels_identical("e1 undo 后", w, &baseline, &restored_px);
    }

    /// E2：形状填充 / 描边 / 线宽编辑必须改变渲染像素；撤销后逐字节回到基线。
    #[test]
    fn e2_shape_fill_stroke_reflect_in_rendered_pixels() {
        let ctx = open_sample();
        let layer = ctx.create_shape(
            r#"{"kind":"rect","x":300,"y":300,"w":200,"h":150,"fill":[0.2,0.4,0.9,1],"strokeWidth":0}"#,
        );
        let rev0 = ctx.revision();

        let before = ctx.get(layer);
        let data = &before["data"];
        assert_str(data, "kind", "Shape", "e2 get");
        assert_str(data, "shapeKind", "rect", "e2 get");
        assert_num(data, "x", 300.0, "e2 get");
        assert_num(data, "y", 300.0, "e2 get");
        assert_num(data, "w", 200.0, "e2 get");
        assert_num(data, "h", 150.0, "e2 get");
        assert_color(data, "fillColor", [0.2, 0.4, 0.9, 1.0], "e2 get");
        assert_bool(data, "fillEditable", true, "e2 get");
        assert_bool(data, "strokeEditable", false, "e2 get");
        assert_num(data, "strokeWidth", 0.0, "e2 get");

        let (_, _, w, _) = SHAPE_REGION;
        let baseline = ctx.render(SHAPE_REGION, "e2 基线");

        let state = ctx.apply_ok(
            layer,
            r#"{"fillColor":[0.9,0.1,0.1,1],"strokeColor":[0,0,0,1],"strokeWidth":12}"#,
        );
        let rev1 = ctx.revision();
        assert_eq!(rev1, rev0 + 1, "e2 apply 字段 revision: 期望 {}, 实际 {rev1}", rev0 + 1);
        let after = &state["data"];
        assert_color(after, "fillColor", [0.9, 0.1, 0.1, 1.0], "e2 apply 回读");
        assert_bool(after, "fillEditable", true, "e2 apply 回读");
        assert_color(after, "strokeColor", [0.0, 0.0, 0.0, 1.0], "e2 apply 回读");
        assert_bool(after, "strokeEditable", true, "e2 apply 回读");
        assert_num(after, "strokeWidth", 12.0, "e2 apply 回读");

        let edited = ctx.render(SHAPE_REGION, "e2 编辑后");
        let diff = assert_pixels_changed("e2 编辑后", w, &baseline, &edited);
        eprintln!(
            "e2 区域={}x{} 基线像素数={} 差异像素数={} 占比={:.4}% {}",
            SHAPE_REGION.2,
            SHAPE_REGION.3,
            diff.pixels,
            diff.changed,
            diff.ratio() * 100.0,
            describe_first_pixel(diff.first, w)
        );

        let undone = ctx.undo();
        assert_ok(&undone, "e2 undo");
        let rev2 = ctx.revision();
        assert_eq!(rev2, rev1 + 1, "e2 undo 字段 revision: 期望 {}, 实际 {rev2}", rev1 + 1);
        let restored = ctx.get(layer);
        assert_color(&restored["data"], "fillColor", [0.2, 0.4, 0.9, 1.0], "e2 undo 后 get");
        assert_num(&restored["data"], "strokeWidth", 0.0, "e2 undo 后 get");
        assert_bool(&restored["data"], "strokeEditable", false, "e2 undo 后 get");
        assert_eq!(restored, before, "e2 undo 后 get: 期望读模型逐字段恢复, 实际 {restored}");
        let restored_px = ctx.render(SHAPE_REGION, "e2 undo 后");
        assert_pixels_identical("e2 undo 后", w, &baseline, &restored_px);
    }

    /// E3：仅位移（`dx`/`dy`）改变形状几何与渲染像素；撤销后逐字节回到基线。
    #[test]
    fn e3_shape_offset_moves_geometry_and_pixels() {
        let ctx = open_sample();
        let layer = ctx.create_shape(r#"{"kind":"rect","x":300,"y":300,"w":200,"h":150,"fill":[0.2,0.4,0.9,1]}"#);
        let rev0 = ctx.revision();

        let before = ctx.get(layer);
        let data = &before["data"];
        let (x0, y0) = (
            data["x"].as_f64().unwrap_or_else(|| panic!("e3 get: 字段 x 缺失（{before}）")),
            data["y"].as_f64().unwrap_or_else(|| panic!("e3 get: 字段 y 缺失（{before}）")),
        );

        let (_, _, w, _) = SHAPE_REGION;
        let baseline = ctx.render(SHAPE_REGION, "e3 基线");

        let state = ctx.apply_ok(layer, r#"{"dx":120,"dy":60}"#);
        let rev1 = ctx.revision();
        assert_eq!(rev1, rev0 + 1, "e3 apply 字段 revision: 期望 {}, 实际 {rev1}", rev0 + 1);
        let after = &state["data"];
        assert_num(after, "x", x0 + 120.0, "e3 apply 回读");
        assert_num(after, "y", y0 + 60.0, "e3 apply 回读");
        assert_num(after, "w", 200.0, "e3 apply 回读（宽度不应变化）");
        assert_num(after, "h", 150.0, "e3 apply 回读（高度不应变化）");

        let edited = ctx.render(SHAPE_REGION, "e3 位移后");
        let diff = assert_pixels_changed("e3 位移后", w, &baseline, &edited);
        eprintln!(
            "e3 区域={}x{} 基线像素数={} 差异像素数={} 占比={:.4}% {}",
            SHAPE_REGION.2,
            SHAPE_REGION.3,
            diff.pixels,
            diff.changed,
            diff.ratio() * 100.0,
            describe_first_pixel(diff.first, w)
        );

        let undone = ctx.undo();
        assert_ok(&undone, "e3 undo");
        let rev2 = ctx.revision();
        assert_eq!(rev2, rev1 + 1, "e3 undo 字段 revision: 期望 {}, 实际 {rev2}", rev1 + 1);
        let restored = ctx.get(layer);
        assert_num(&restored["data"], "x", x0, "e3 undo 后 get");
        assert_num(&restored["data"], "y", y0, "e3 undo 后 get");
        assert_eq!(restored, before, "e3 undo 后 get: 期望读模型逐字段恢复, 实际 {restored}");
        let restored_px = ctx.render(SHAPE_REGION, "e3 undo 后");
        assert_pixels_identical("e3 undo 后", w, &baseline, &restored_px);
    }

    /// E4：非法值与非法目标零副作用——`ok=false` + 错误文本 + revision 不变 + `get` 快照逐字段不变。
    #[test]
    fn e4_invalid_values_leave_no_trace() {
        let ctx = open_sample();
        let text = ctx.create_text(r#"{"text":"AAAA","size":96,"x":200,"y":400,"color":[1,1,1,1]}"#);
        let shape = ctx.create_shape(r#"{"kind":"rect","x":300,"y":300,"w":200,"h":150,"fill":[0.2,0.4,0.9,1]}"#);
        let base = sample_base_layer(&ctx);
        let rev_before_all = ctx.revision();

        // (名称, 目标图层, 请求 JSON, 期望错误文本片段)
        let cases: [(&str, u64, &str, &str); 14] = [
            ("空文字", text, r#"{"text":""}"#, "text must not be empty"),
            ("空文字-全空白", text, r#"{"text":"   "}"#, "text must not be empty"),
            ("字号 0", text, r#"{"size":0}"#, "text size must be finite and between 1 and 2000 pt"),
            ("字号 5000", text, r#"{"size":5000}"#, "text size must be finite and between 1 and 2000 pt"),
            ("未知字体", text, r#"{"fontFamily":"NoSuchFont"}"#, "unknown font family: NoSuchFont"),
            ("文字颜色越界", text, r#"{"color":[1.5,0,0,1]}"#, "text color components must be finite and between 0 and 1"),
            ("线宽 -1", shape, r#"{"strokeWidth":-1}"#, "stroke width must be finite and between 0 and 10000 px"),
            ("线宽 20000", shape, r#"{"strokeWidth":20000}"#, "stroke width must be finite and between 0 and 10000 px"),
            ("位移越界", text, r#"{"dx":200000}"#, "offset must be finite and within ±100000 px"),
            ("无字段", text, r#"{}"#, "no editable field provided"),
            ("形状层收到文字字段", shape, r#"{"text":"x"}"#, "text fields are not valid for a shape layer"),
            ("形状层收到文字颜色", shape, r#"{"color":[1,1,1,1]}"#, "text fields are not valid for a shape layer"),
            ("文字层收到填充字段", text, r#"{"fillColor":[1,1,1,1]}"#, "shape fields are not valid for a text layer"),
            ("文字层收到线宽", text, r#"{"strokeWidth":3}"#, "shape fields are not valid for a text layer"),
        ];
        for (name, layer, params, needle) in cases {
            let rev_before = ctx.revision();
            let snapshot_before = ctx.get(layer);
            let state = ctx.apply(layer, params);
            assert_err(&state, &format!("e4 {name} layer={layer} {params}"), needle);
            let rev_after = ctx.revision();
            assert_eq!(
                rev_after, rev_before,
                "e4 {name} {params} 字段 revision: 期望失败不 checkpoint、保持 {rev_before}, 实际 {rev_after}"
            );
            let snapshot_after = ctx.get(layer);
            assert_eq!(
                snapshot_after, snapshot_before,
                "e4 {name} {params}: 期望失败后 get 快照逐字段不变, 实际 {snapshot_after}"
            );
        }

        // 非法目标：null session / 文档不存在 / 图层不存在 / 不可编辑图层（都要零副作用）。
        let missing_doc = ctx.doc + 999;
        let null_get = read_json(
            pc_layer_edit_get(std::ptr::null_mut(), ctx.doc, text),
            "e4 null session get",
        );
        assert_err(&null_get, "e4 null session get", "null session");
        let null_apply = read_json(
            pc_layer_edit_apply(std::ptr::null_mut(), ctx.doc, text, b"{}".as_ptr(), 2),
            "e4 null session apply",
        );
        assert_err(&null_apply, "e4 null session apply", "null session");
        let doc_get = read_json(pc_layer_edit_get(ctx.session, missing_doc, text), "e4 文档不存在 get");
        assert_err(&doc_get, "e4 文档不存在 get", &format!("document {missing_doc} is not open"));
        let doc_apply = read_json(
            pc_layer_edit_apply(ctx.session, missing_doc, text, b"{}".as_ptr(), 2),
            "e4 文档不存在 apply",
        );
        assert_err(&doc_apply, "e4 文档不存在 apply", &format!("document {missing_doc} is not open"));
        let layer_get = read_json(pc_layer_edit_get(ctx.session, ctx.doc, u64::MAX), "e4 图层不存在 get");
        assert_err(&layer_get, "e4 图层不存在 get", "layer not found");
        let layer_apply = read_json(
            pc_layer_edit_apply(ctx.session, ctx.doc, u64::MAX, b"{}".as_ptr(), 2),
            "e4 图层不存在 apply",
        );
        assert_err(&layer_apply, "e4 图层不存在 apply", "layer not found");
        let base_get = read_json(pc_layer_edit_get(ctx.session, ctx.doc, base), "e4 不可编辑图层 get");
        assert_err(&base_get, "e4 不可编辑图层 get", "not an editable layer");
        let base_apply = read_json(
            pc_layer_edit_apply(ctx.session, ctx.doc, base, b"{}".as_ptr(), 2),
            "e4 不可编辑图层 apply",
        );
        assert_err(&base_apply, "e4 不可编辑图层 apply", "not an editable layer");
        // 非法 JSON 参数：解包失败也要零副作用。
        let bad_json = read_json(
            pc_layer_edit_apply(ctx.session, ctx.doc, text, b"not-json".as_ptr(), 8),
            "e4 非法 JSON",
        );
        assert_err(&bad_json, "e4 非法 JSON", "invalid edit parameters");

        let rev_after_all = ctx.revision();
        assert_eq!(
            rev_after_all, rev_before_all,
            "e4 全部失败用例结束后 revision: 期望保持 {rev_before_all}, 实际 {rev_after_all}"
        );
        eprintln!(
            "e4 非法值子用例={} revision 全程保持={}",
            cases.len(),
            rev_after_all
        );
    }

    /// E5：同一颜色数组经"创建路径"与"编辑路径"写出的渲染像素逐字节相同（颜色不交换）。
    #[test]
    fn e5_edit_color_matches_create_color() {
        // `[b, g, r, a]`：若实现里对通道做了重排，创建路径与编辑路径会渲染出不同颜色。
        const C: [f32; 4] = [0.1, 0.2, 0.9, 1.0];
        const OTHER: [f32; 4] = [0.5, 0.5, 0.5, 1.0];

        // ---- 形状：创建路径 vs 编辑路径
        let created_ctx = open_sample();
        let created = created_ctx.create_shape(
            r#"{"kind":"rect","x":300,"y":300,"w":200,"h":150,"fill":[0.1,0.2,0.9,1]}"#,
        );
        let created_info = created_ctx.get(created);
        assert_color(&created_info["data"], "fillColor", C, "e5 形状创建路径 get");
        let created_px = created_ctx.render(SHAPE_REGION, "e5 形状创建路径渲染");

        let edited_ctx = open_sample();
        let edited = edited_ctx.create_shape(
            r#"{"kind":"rect","x":300,"y":300,"w":200,"h":150,"fill":[0.5,0.5,0.5,1]}"#,
        );
        let applied = edited_ctx.apply_ok(edited, r#"{"fillColor":[0.1,0.2,0.9,1]}"#);
        assert_color(&applied["data"], "fillColor", C, "e5 形状编辑路径 apply 回读");
        let edited_info = edited_ctx.get(edited);
        assert_color(&edited_info["data"], "fillColor", C, "e5 形状编辑路径 get");
        let edited_px = edited_ctx.render(SHAPE_REGION, "e5 形状编辑路径渲染");
        assert_pixels_identical("e5 形状 创建路径 vs 编辑路径", SHAPE_REGION.2, &created_px, &edited_px);
        eprintln!(
            "e5 形状 创建路径 vs 编辑路径 区域={}x{} 逐字节相同={}",
            SHAPE_REGION.2,
            SHAPE_REGION.3,
            pixels_equal(&created_px, &edited_px)
        );

        // ---- 文字：创建路径 vs 编辑路径
        let created_ctx = open_sample();
        let created = created_ctx.create_text(r#"{"text":"AAAA","size":96,"x":200,"y":400,"color":[0.1,0.2,0.9,1]}"#);
        let created_info = created_ctx.get(created);
        assert_color(&created_info["data"], "color", C, "e5 文字创建路径 get");
        let created_px = created_ctx.render(TEXT_REGION, "e5 文字创建路径渲染");

        let edited_ctx = open_sample();
        let edited = edited_ctx.create_text(r#"{"text":"AAAA","size":96,"x":200,"y":400,"color":[0.5,0.5,0.5,1]}"#);
        let applied = edited_ctx.apply_ok(edited, r#"{"color":[0.1,0.2,0.9,1]}"#);
        assert_color(&applied["data"], "color", C, "e5 文字编辑路径 apply 回读");
        let edited_info = edited_ctx.get(edited);
        assert_color(&edited_info["data"], "color", C, "e5 文字编辑路径 get");
        let edited_px = edited_ctx.render(TEXT_REGION, "e5 文字编辑路径渲染");
        assert_pixels_identical("e5 文字 创建路径 vs 编辑路径", TEXT_REGION.2, &created_px, &edited_px);
        eprintln!(
            "e5 文字 创建路径 vs 编辑路径 区域={}x{} 逐字节相同={}",
            TEXT_REGION.2,
            TEXT_REGION.3,
            pixels_equal(&created_px, &edited_px)
        );

        // `OTHER` 只用于构造"待编辑"的初始填充色，这里显式断言它确实与 C 不同。
        assert_ne!(C, OTHER, "e5 前置: 初始色必须与目标色不同");
    }

    /// E6：无变化（值相同 / 零位移）不写文档、不 checkpoint、revision 与 canUndo 不变。
    #[test]
    fn e6_unchanged_values_do_not_checkpoint() {
        let ctx = open_sample();
        let layer = ctx.create_text(r#"{"text":"AAAA","size":96,"x":200,"y":400,"color":[1,1,1,1]}"#);
        let rev0 = ctx.revision();
        let can_undo = ctx.info()["data"]["canUndo"].clone();
        let size = ctx.get(layer)["data"]["size"]
            .as_f64()
            .unwrap_or_else(|| panic!("e6 get: 字段 size 缺失"));

        let requests = [
            ("相同字号", format!(r#"{{"size":{size}}}"#)),
            ("零位移", r#"{"dx":0,"dy":0}"#.to_string()),
            ("相同内容", r#"{"text":"AAAA"}"#.to_string()),
        ];
        for (name, params) in requests {
            let state = ctx.apply(layer, &params);
            assert_ok(&state, &format!("e6 {name} {params}"));
            let rev = ctx.revision();
            assert_eq!(
                rev, rev0,
                "e6 {name} {params} 字段 revision: 期望无变化不 checkpoint、保持 {rev0}, 实际 {rev}"
            );
            let can_undo_after = ctx.info()["data"]["canUndo"].clone();
            assert_eq!(
                can_undo_after, can_undo,
                "e6 {name} {params} 字段 canUndo: 期望保持 {can_undo}, 实际 {can_undo_after}"
            );
            assert_num(&state["data"], "size", size, &format!("e6 {name} 返回 payload"));
        }
        eprintln!("e6 无变化请求数={} revision 保持={rev0}", 3);
    }

    /// E7：`pc_layer_edit_get` 是只读的——连续 10 次调用不改 revision、不改渲染结果。
    #[test]
    fn e7_get_is_read_only() {
        let ctx = open_sample();
        let layer = ctx.create_text(r#"{"text":"AAAA","size":96,"x":200,"y":400,"color":[1,1,1,1]}"#);
        let rev0 = ctx.revision();
        let baseline = ctx.render(TEXT_REGION, "e7 基线");

        for i in 0..10 {
            let state = ctx.get(layer);
            assert_ok(&state, &format!("e7 get 第 {i} 次"));
            assert_str(&state["data"], "kind", "Type", &format!("e7 get 第 {i} 次"));
        }
        let rev1 = ctx.revision();
        assert_eq!(rev1, rev0, "e7 十次 get 后 revision: 期望保持 {rev0}, 实际 {rev1}");
        let after = ctx.render(TEXT_REGION, "e7 十次 get 后");
        assert_pixels_identical("e7 十次 get 后", TEXT_REGION.2, &baseline, &after);
        eprintln!("e7 get 调用次数=10 revision 保持={rev1}");
    }

    /// E8：渐变填充只读保护（R2.6）——`fillEditable=false`，写填充被拒且零副作用。
    #[test]
    fn e8_gradient_fill_is_read_only() {
        let ctx = open_sample();
        let layer = ctx.create_shape(r#"{"kind":"rect","x":300,"y":300,"w":200,"h":150,"fill":[0.2,0.4,0.9,1]}"#);
        // 直接构造渐变填充：应用的新建形状只能产生纯色填充，该状态在 UI 上不可达（R2.6）。
        ctx.with_layer_mut(layer, |target| match &mut target.content {
            LayerContent::Shape(shape) => {
                shape.fill = Some(Fill::Gradient {
                    stops: vec![(0.0, Color::rgba(0.0, 0.0, 0.0, 1.0)), (1.0, Color::rgba(1.0, 1.0, 1.0, 1.0))],
                    angle: 0.0,
                    scale: 1.0,
                    style: GradientStyle::Linear,
                    reverse: false,
                    opacity_stops: Vec::new(),
                    midpoints: Vec::new(),
                    offset: (0.0, 0.0),
                    dither: false,
                    align: true,
                });
            }
            _ => panic!("e8 前置: 期望形状图层"),
        });

        let rev0 = ctx.revision();
        let before = ctx.get(layer);
        assert_str(&before["data"], "kind", "Shape", "e8 get");
        assert_bool(&before["data"], "fillEditable", false, "e8 get");
        assert_color(&before["data"], "fillColor", [0.0, 0.0, 0.0, 0.0], "e8 get");

        let state = ctx.apply(layer, r#"{"fillColor":[1,1,1,1]}"#);
        assert_err(&state, "e8 写渐变填充", "this layer's fill is not a solid color");
        let rev1 = ctx.revision();
        assert_eq!(rev1, rev0, "e8 写渐变填充 字段 revision: 期望保持 {rev0}, 实际 {rev1}");
        let after = ctx.get(layer);
        assert_eq!(after, before, "e8 写渐变填充: 期望 get 快照逐字段不变, 实际 {after}");
        eprintln!("e8 渐变填充 fillEditable=false, 写填充被拒且 revision 保持={rev1}");
    }

    /// E9：**只**提交文字内容（不带 size / color / fontFamily / dx / dy）也必须改变渲染像素。
    ///
    /// E1 一次同时改 `text + size + fontFamily + color`，"像素有变化"无法区分是文字内容生效
    /// 还是颜色/字号生效。T5 设备复验发现"只改内容时画面零变化"，因此这里把变量收敛到
    /// `text` 一个字段，直接判定：
    ///   ① `ok==true`；② `revision == before + 1`（相等即说明该请求被当成"无变化"，走
    ///   `unchanged → Ok(None)` 分支）；③ 回读 `text` 等于新值；④ 渲染差异像素 > 0。
    #[test]
    fn e9_only_text_change_reflects_in_pixels() {
        let ctx = open_sample();
        let layer = ctx.create_text(r#"{"text":"PhotoCraft","size":96,"x":200,"y":400,"color":[1,1,1,1]}"#);
        let rev0 = ctx.revision();
        // 取样区域必须**覆盖变宽后的文本**：旧 cache 包围盒 x 207..687、新 cache 207..935
        // （见下方 probe 输出）。沿用 E1 的 `TEXT_REGION`（x 只到 640）会把新增的 "BBBB"
        // 整段裁在区域之外，从而得到"像素 0 变化"的**假阴性**——这是 E1 掩盖本缺陷的
        // 第二个盲点（第一个是同时改四项，无法区分是哪一项生效）。
        let region = (120, 300, 960, 220);
        let (_, _, w, _) = region;
        let baseline = ctx.render(region, "e9 基线");
        assert_str(&ctx.get(layer)["data"], "text", "PhotoCraft", "e9 基线 get");

        // 诊断探针：把文档里真实的 text / runs 数量 / cache 包围盒打出来，用来区分
        // "副本没写回" 与 "写回了但 cache 没随文本重建"。
        let probe = |tag: &str| {
            let (text, runs, cache) = ctx.with_layer_mut(layer, |l| match &l.content {
                LayerContent::Text(t) => {
                    (t.text.clone(), t.runs.len(), t.cache.as_ref().map(|s| s.content_bounds()))
                }
                _ => panic!("e9 probe: 期望文字图层"),
            });
            eprintln!("e9 probe[{tag}] text={text:?} runs={runs} cache_bounds={cache:?}");
        };
        probe("apply 前");

        // 只改内容：其余字段全部缺省（= 不改），位移也不给。
        let params = r#"{"text":"PhotoCraftBBBB"}"#;
        let state = ctx.apply(layer, params);
        // 无论成败都打印完整响应：若走了 "unchanged → Ok(None)" 分支，payload 会是旧值。
        eprintln!("e9 apply {params} -> {state}");
        assert_ok(&state, &format!("e9 只改 text {params}"));

        let rev1 = ctx.revision();
        eprintln!("e9 revision {rev0} -> {rev1}");
        assert_eq!(
            rev1,
            rev0 + 1,
            "e9 只改 text: revision 期望 {}, 实际 {rev1}（相等 = 被判为「无变化」而跳过 checkpoint）；\
             apply 响应 {state}",
            rev0 + 1
        );

        assert_str(&state["data"], "text", "PhotoCraftBBBB", "e9 apply 回读");
        assert_str(&ctx.get(layer)["data"], "text", "PhotoCraftBBBB", "e9 get 回读");
        probe("apply 后");

        let edited = ctx.render(region, "e9 编辑后");
        let diff = assert_pixels_changed("e9 只改 text 编辑后", w, &baseline, &edited);
        eprintln!(
            "e9 区域={}x{} 基线像素数={} 差异像素数={} 占比={:.4}% {}",
            region.2,
            region.3,
            diff.pixels,
            diff.changed,
            diff.ratio() * 100.0,
            describe_first_pixel(diff.first, w)
        );
        // 定向断言：差异必须出现在旧文本右边界（文档 x=687，区域相对 x=567）附近或之后，
        // 即"新增的 BBBB"。若差异出现在旧文本内部，说明改动影响了原有字形（不应发生）。
        if let Some((index, _, _)) = diff.first {
            let first_x = index as i32 % w.max(1);
            assert!(
                first_x >= 480,
                "e9 首个差异像素相对 x={first_x}（绝对 x={}）落在旧文本 PhotoCraft 内部（相对 \
                 x<480）；只改 text 不应改变原有字形（区域 {region:?}）",
                120 + first_x
            );
        }

        // 一次提交 = 一次 checkpoint：撤销后内容与像素都回到基线。
        let undone = ctx.undo();
        assert_ok(&undone, "e9 undo");
        let rev2 = ctx.revision();
        assert_eq!(rev2, rev1 + 1, "e9 undo revision: 期望 {}, 实际 {rev2}", rev1 + 1);
        assert_str(&ctx.get(layer)["data"], "text", "PhotoCraft", "e9 undo 后 get");
        let restored_px = ctx.render(region, "e9 undo 后");
        assert_pixels_identical("e9 undo 后", w, &baseline, &restored_px);

        // ---- 子场景 2：文字层带非空 `runs`（PSD 富文本形态）时，只改 text 仍必须生效。
        // 预置 runs = char_runs()（与摘要字段同样式，视觉与 cache 都不变），用来否证
        // 「runs 未清空 → 新字符沿用旧排版 → 画面不变」这一假说。
        ctx.with_layer_mut(layer, |l| match &mut l.content {
            LayerContent::Text(t) => {
                let runs = t.char_runs();
                t.runs = runs;
            }
            _ => panic!("e9 子场景 2: 期望文字图层"),
        });
        probe("预置 runs 后");

        let rev3 = ctx.revision();
        let params2 = r#"{"text":"PhotoCraftCCCC"}"#;
        let state2 = ctx.apply(layer, params2);
        eprintln!("e9 子场景 2 apply {params2} -> {state2}");
        assert_ok(&state2, "e9 子场景 2 只改 text");
        assert_eq!(
            ctx.revision(),
            rev3 + 1,
            "e9 子场景 2: 期望 revision {}（带 runs 的层也只一次 checkpoint）",
            rev3 + 1
        );
        assert_str(&ctx.get(layer)["data"], "text", "PhotoCraftCCCC", "e9 子场景 2 get 回读");
        // §E.1：任一文字属性编辑都必须把 runs 归一（清空），回落到摘要字段。
        let runs_after = ctx.with_layer_mut(layer, |l| match &l.content {
            LayerContent::Text(t) => t.runs.len(),
            _ => panic!("e9 子场景 2: 期望文字图层"),
        });
        assert_eq!(runs_after, 0, "e9 子场景 2: 文字属性编辑后 runs 必须清空（§E.1）");
        let edited2 = ctx.render(region, "e9 子场景 2 编辑后");
        let diff2 = assert_pixels_changed("e9 子场景 2 只改 text", w, &baseline, &edited2);
        eprintln!("e9 子场景 2 差异像素数={} 占比={:.4}%", diff2.changed, diff2.ratio() * 100.0);
    }

    /// E16：批量复制必须是单次 checkpoint，一次撤销整批回退。
    #[test]
    fn e16_duplicate_many_single_checkpoint() {
        let ctx = open_sample();
        let base = sample_base_layer(&ctx);
        let first = read_json(pc_layer_duplicate(ctx.session, ctx.doc, base), "e16 dup single");
        assert_ok(&first, "e16 dup single");
        let copy = first["data"]["layerId"]
            .as_u64()
            .unwrap_or_else(|| panic!("e16 dup single: 缺少 layerId: {first}"));
        assert_eq!(
            ctx.info()["data"]["layers"].as_array().map(|items| items.len()),
            Some(2),
            "e16 前置：单次复制后应为 2 层"
        );

        let ids = format!("[{base},{copy}]");
        let state = read_json(
            pc_layer_duplicate_many(ctx.session, ctx.doc, ids.as_bytes().as_ptr(), ids.len()),
            "e16 dup many",
        );
        assert_ok(&state, "e16 dup many");
        let new_ids = state["data"]["layerIds"]
            .as_array()
            .unwrap_or_else(|| panic!("e16 dup many: 缺少 layerIds: {state}"));
        assert_eq!(new_ids.len(), 2, "e16 应返回 2 个新 id");
        assert_eq!(
            ctx.info()["data"]["layers"].as_array().map(|items| items.len()),
            Some(4),
            "e16 批量复制后应为 4 层"
        );

        let undo = read_json(pc_undo(ctx.session, ctx.doc), "e16 undo 1");
        assert_ok(&undo, "e16 undo 1");
        assert_eq!(
            ctx.info()["data"]["layers"].as_array().map(|items| items.len()),
            Some(2),
            "e16 单次 undo 必须整批恢复到 2 层"
        );
        let undo2 = read_json(pc_undo(ctx.session, ctx.doc), "e16 undo 2");
        assert_ok(&undo2, "e16 undo 2");
        assert_eq!(
            ctx.info()["data"]["layers"].as_array().map(|items| items.len()),
            Some(1),
            "e16 再 undo 回 1 层"
        );

        // 空 payload / 缺失 id：失败且不 checkpoint。
        let empty = read_json(pc_layer_duplicate_many(ctx.session, ctx.doc, b"[]".as_ptr(), 2), "e16 empty");
        assert_eq!(empty["ok"].as_bool(), Some(false), "e16 空载荷应失败");
        assert!(
            empty["error"].as_str().unwrap_or("").contains("no layer to duplicate"),
            "e16 空载荷文案: {empty}"
        );
        let rev = ctx.revision();
        let missing = read_json(
            pc_layer_duplicate_many(ctx.session, ctx.doc, b"[999999]".as_ptr(), 8),
            "e16 missing",
        );
        assert_eq!(missing["ok"].as_bool(), Some(false), "e16 缺失 id 应失败");
        assert!(
            missing["error"].as_str().unwrap_or("").contains("layer not found: 999999"),
            "e16 缺失 id 文案: {missing}"
        );
        assert_eq!(ctx.revision(), rev, "e16 失败不应 bump revision");
        eprintln!("e16 duplicate_many single checkpoint ok");
    }

    /// E17：批量建组一次 checkpoint，一次撤销恢复原顶层；解组同样单次生效。
    #[test]
    fn e17_group_single_checkpoint_and_undo_restores() {
        let ctx = open_sample();
        let base = sample_base_layer(&ctx);
        let first = read_json(pc_layer_duplicate(ctx.session, ctx.doc, base), "e17 dup single");
        assert_ok(&first, "e17 dup single");
        let copy = first["data"]["layerId"]
            .as_u64()
            .unwrap_or_else(|| panic!("e17 dup single: 缺少 layerId: {first}"));

        let ids = format!("[{base},{copy}]");
        let grouped = read_json(
            pc_layer_group(ctx.session, ctx.doc, ids.as_bytes().as_ptr(), ids.len(), b"G".as_ptr(), 1),
            "e17 group",
        );
        assert_ok(&grouped, "e17 group");
        let group_id = grouped["data"]["groupId"]
            .as_u64()
            .unwrap_or_else(|| panic!("e17 group: 缺少 groupId: {grouped}"));
        let info = ctx.info();
        let layers = info["data"]["layers"]
            .as_array()
            .unwrap_or_else(|| panic!("e17 group: layers 缺失: {info}"));
        assert_eq!(layers.len(), 1, "e17 成组后顶层应只剩组");
        assert_eq!(layers[0]["isGroup"].as_bool(), Some(true), "e17 顶层应为组");
        assert_eq!(
            layers[0]["children"].as_array().map(|children| children.len()),
            Some(2),
            "e17 组内应有 2 层"
        );

        let undo = read_json(pc_undo(ctx.session, ctx.doc), "e17 undo group");
        assert_ok(&undo, "e17 undo group");
        assert_eq!(
            ctx.info()["data"]["layers"].as_array().map(|items| items.len()),
            Some(2),
            "e17 一次 undo 必须恢复两个顶层图层"
        );

        // 再次成组后解组：同样一次生效，回到两个顶层。
        let grouped2 = read_json(
            pc_layer_group(ctx.session, ctx.doc, ids.as_bytes().as_ptr(), ids.len(), b"G".as_ptr(), 1),
            "e17 group 2",
        );
        assert_ok(&grouped2, "e17 group 2");
        let group_id2 = grouped2["data"]["groupId"]
            .as_u64()
            .unwrap_or_else(|| panic!("e17 group 2: 缺少 groupId: {grouped2}"));
        let ungrouped = read_json(pc_layer_ungroup(ctx.session, ctx.doc, group_id2), "e17 ungroup");
        assert_ok(&ungrouped, "e17 ungroup");
        assert_eq!(
            ctx.info()["data"]["layers"].as_array().map(|items| items.len()),
            Some(2),
            "e17 解组后顶层应恢复 2 层"
        );
        eprintln!("e17 group single checkpoint ok, group={group_id} ungroup={group_id2}");
    }

    /// E18：把已有组与外部图层再次成组时，必须整体迁移 G1 子树（子层不丢）且一次 undo 回退。
    #[test]
    fn e18_nested_group_moves_whole_subtree() {
        let ctx = open_sample();
        let base = sample_base_layer(&ctx);
        let dup = read_json(pc_layer_duplicate(ctx.session, ctx.doc, base), "e18 dup");
        assert_ok(&dup, "e18 dup");
        let copy = dup["data"]["layerId"]
            .as_u64()
            .unwrap_or_else(|| panic!("e18 dup: 缺少 layerId: {dup}"));

        let ids = format!("[{base},{copy}]");
        let g1 = read_json(
            pc_layer_group(ctx.session, ctx.doc, ids.as_bytes().as_ptr(), ids.len(), b"G1".as_ptr(), 2),
            "e18 g1",
        );
        assert_ok(&g1, "e18 g1");
        let g1_id = g1["data"]["groupId"]
            .as_u64()
            .unwrap_or_else(|| panic!("e18 g1: 缺少 groupId: {g1}"));

        let shape_params = r#"{"kind":"rect","x":100,"y":100,"w":200,"h":200,"fill":[1,0,0,1]}"#;
        let shape_id = ctx.create_shape(shape_params);

        let ids2 = format!("[{g1_id},{shape_id}]");
        let g2 = read_json(
            pc_layer_group(ctx.session, ctx.doc, ids2.as_bytes().as_ptr(), ids2.len(), b"G2".as_ptr(), 2),
            "e18 g2",
        );
        assert_ok(&g2, "e18 g2");

        let info = ctx.info();
        let layers = info["data"]["layers"]
            .as_array()
            .unwrap_or_else(|| panic!("e18 g2: layers 缺失: {info}"));
        assert_eq!(layers.len(), 1, "e18 嵌套成组后顶层应只剩 G2");
        assert_eq!(layers[0]["isGroup"].as_bool(), Some(true), "e18 顶层应是组");
        let children = layers[0]["children"]
            .as_array()
            .unwrap_or_else(|| panic!("e18 G2 children 缺失: {info}"));
        assert_eq!(children.len(), 2, "e18 G2 应有两个子项（G1 + shape）");

        let nested = children
            .iter()
            .find(|child| child["id"].as_u64() == Some(g1_id))
            .unwrap_or_else(|| panic!("e18 G2 中未找到 G1（id={g1_id}）: {info}"));
        assert_eq!(nested["isGroup"].as_bool(), Some(true), "e18 G1 迁移后仍应为组");
        assert_eq!(
            nested["children"].as_array().map(|items| items.len()),
            Some(2),
            "e18 G1 的子层必须整体保留"
        );
        assert!(
            children.iter().any(|child| child["id"].as_u64() == Some(shape_id)),
            "e18 G2 应包含 shape 子项"
        );

        let undo = read_json(pc_undo(ctx.session, ctx.doc), "e18 undo");
        assert_ok(&undo, "e18 undo");
        let info = ctx.info();
        let layers = info["data"]["layers"]
            .as_array()
            .unwrap_or_else(|| panic!("e18 undo: layers 缺失: {info}"));
        assert_eq!(layers.len(), 2, "e18 一次 undo 应恢复 G1 + shape 两个顶层");
        let g1_after = layers
            .iter()
            .find(|layer| layer["id"].as_u64() == Some(g1_id))
            .unwrap_or_else(|| panic!("e18 undo 后未找到 G1: {info}"));
        assert_eq!(g1_after["isGroup"].as_bool(), Some(true), "e18 undo 后 G1 恢复为组");
        assert_eq!(
            g1_after["children"].as_array().map(|items| items.len()),
            Some(2),
            "e18 undo 后 G1 子层完整"
        );
        eprintln!("e18 nested group migration ok, g1={g1_id}");
    }

    /// E19：Box 框文本只改 text 也必须重建 cache 并改变像素；一次 undo 回旧文本。
    #[test]
    fn e19_box_text_edit_reflects_in_pixels_and_undoes() {
        let ctx = open_sample();
        let layer = ctx.create_text(r#"{"text":"SEED","size":72,"x":200,"y":300,"color":[1,1,1,1]}"#);
        ctx.with_layer_mut(layer, |l| match &mut l.content {
            LayerContent::Text(t) => {
                t.shape = TextShape::Box { x: 200.0, y: 300.0, width: 800.0, height: 240.0 };
                t.transform = Affine::translate(0.0, 0.0);
                t.sync_summary();
            }
            _ => panic!("e19 前置: 期望文字图层"),
        });
        let seed = ctx.apply(layer, r#"{"text":"SEED"}"#);
        assert_ok(&seed, "e19 seed");
        let region = (150, 250, 900, 400);
        let baseline = ctx.render(region, "e19 Box 基线");
        let rev0 = ctx.revision();

        let state = ctx.apply(layer, r#"{"text":"AAAA BBBB"}"#);
        assert_ok(&state, "e19 Box 只改 text");
        let rev1 = ctx.revision();
        assert_eq!(rev1, rev0 + 1, "e19 Box 编辑 revision: 期望 {}, 实际 {rev1}", rev0 + 1);
        assert_str(&ctx.get(layer)["data"], "text", "AAAA BBBB", "e19 Box get 回读");
        let edited = ctx.render(region, "e19 Box 编辑后");
        let diff = assert_pixels_changed("e19 Box 只改 text", region.2, &baseline, &edited);
        let bounds = ctx.with_layer_mut(layer, |l| match &l.content {
            LayerContent::Text(t) => t.cache.as_ref().map(|s| s.content_bounds()),
            _ => None,
        });
        eprintln!(
            "e19 box diff={} ratio={:.4}% cache_bounds={bounds:?}",
            diff.changed,
            diff.ratio() * 100.0
        );
        assert!(diff.changed > 0, "e19 Box 编辑必须改变像素");

        ctx.undo();
        assert_str(&ctx.get(layer)["data"], "text", "SEED", "e19 undo 后 text");
        let undone = ctx.render(region, "e19 undo 后像素");
        let undo_diff = diff_pixels(&baseline, &undone, "e19 undo 后");
        assert_eq!(
            undo_diff.changed, 0,
            "e19 undo 后应逐字节回到 Box 基线, changed={}",
            undo_diff.changed
        );
    }

    /// E20：Area 路径内段落只改 text 也必须刷新 cache/像素并可撤销。
    #[test]
    fn e20_area_text_edit_reflects_in_pixels_and_undoes() {
        let ctx = open_sample();
        let layer = ctx.create_text(r#"{"text":"SEED","size":72,"x":200,"y":300,"color":[1,1,1,1]}"#);
        ctx.with_layer_mut(layer, |l| match &mut l.content {
            LayerContent::Text(t) => {
                let path = photocraft_doc::vector::Path::new(vec![photocraft_doc::vector::Subpath::polygon(&[
                    (200.0, 300.0),
                    (1000.0, 300.0),
                    (1000.0, 540.0),
                    (200.0, 540.0),
                ])]);
                t.shape = TextShape::Area { path };
                t.transform = Affine::translate(0.0, 0.0);
                t.sync_summary();
            }
            _ => panic!("e20 前置: 期望文字图层"),
        });
        let seed = ctx.apply(layer, r#"{"text":"SEED"}"#);
        assert_ok(&seed, "e20 seed");
        let region = (150, 250, 900, 400);
        let baseline = ctx.render(region, "e20 Area 基线");
        let rev0 = ctx.revision();

        let state = ctx.apply(layer, r#"{"text":"AAAA BBBB CCCC"}"#);
        assert_ok(&state, "e20 Area 只改 text");
        let rev1 = ctx.revision();
        assert_eq!(rev1, rev0 + 1, "e20 Area 编辑 revision: 期望 {}, 实际 {rev1}", rev0 + 1);
        assert_str(&ctx.get(layer)["data"], "text", "AAAA BBBB CCCC", "e20 Area get 回读");
        let edited = ctx.render(region, "e20 Area 编辑后");
        let diff = assert_pixels_changed("e20 Area 只改 text", region.2, &baseline, &edited);
        eprintln!("e20 area diff={} ratio={:.4}%", diff.changed, diff.ratio() * 100.0);

        ctx.undo();
        assert_str(&ctx.get(layer)["data"], "text", "SEED", "e20 undo 后 text");
        let undone = ctx.render(region, "e20 undo 后像素");
        assert_eq!(
            diff_pixels(&baseline, &undone, "e20 undo 后").changed,
            0,
            "e20 undo 后应回到 Area 基线"
        );
    }

    /// E21：路径文字（TextShape::Path）只改 text 也必须刷新 cache/像素并可撤销。
    #[test]
    fn e21_path_text_edit_reflects_in_pixels_and_undoes() {
        let ctx = open_sample();
        let layer = ctx.create_text(r#"{"text":"SEED","size":72,"x":200,"y":900,"color":[1,1,1,1]}"#);
        ctx.with_layer_mut(layer, |l| match &mut l.content {
            LayerContent::Text(t) => {
                let path = photocraft_doc::vector::Subpath::polyline(&[(200.0, 900.0), (1200.0, 900.0)]);
                t.shape = TextShape::Path { path, start: 0.0, flip: false };
                t.transform = Affine::translate(0.0, 0.0);
                t.sync_summary();
            }
            _ => panic!("e21 前置: 期望文字图层"),
        });
        let seed = ctx.apply(layer, r#"{"text":"SEED"}"#);
        assert_ok(&seed, "e21 seed");
        let region = (150, 800, 1100, 220);
        let baseline = ctx.render(region, "e21 Path 基线");
        let rev0 = ctx.revision();

        let state = ctx.apply(layer, r#"{"text":"PATH TEXT"}"#);
        assert_ok(&state, "e21 Path 只改 text");
        let rev1 = ctx.revision();
        assert_eq!(rev1, rev0 + 1, "e21 Path 编辑 revision: 期望 {}, 实际 {rev1}", rev0 + 1);
        assert_str(&ctx.get(layer)["data"], "text", "PATH TEXT", "e21 Path get 回读");
        let edited = ctx.render(region, "e21 Path 编辑后");
        let diff = assert_pixels_changed("e21 Path 只改 text", region.2, &baseline, &edited);
        eprintln!("e21 path diff={} ratio={:.4}%", diff.changed, diff.ratio() * 100.0);

        ctx.undo();
        assert_str(&ctx.get(layer)["data"], "text", "SEED", "e21 undo 后 text");
        let undone = ctx.render(region, "e21 undo 后像素");
        assert_eq!(
            diff_pixels(&baseline, &undone, "e21 undo 后").changed,
            0,
            "e21 undo 后应回到 Path 基线"
        );
    }

    /// E22：新建空白图层 ABI —— 插入位置、自动命名、撤销与失败零副作用。
    #[test]
    fn e22_layer_create_inserts_names_and_undoes() {
        let ctx = open_sample();
        let layer_count = |ctx: &Ctx| -> usize { ctx.info()["data"]["layers"].as_array().map_or(0, Vec::len) };
        let base = layer_count(&ctx);
        assert_eq!(base, 1, "open_sample 应为单层 Background");
        let background = ctx.info()["data"]["layers"][0]["id"].as_u64().unwrap_or(0);

        // 1) above=0：插到最顶层之上，自动命名「图层 1」，单次 revision 递增。
        let rev0 = ctx.revision();
        let created = read_json(pc_layer_create(ctx.session, ctx.doc, 0, b"".as_ptr(), 0), "pc_layer_create above=0");
        assert_ok(&created, "pc_layer_create above=0");
        let first_id = created["data"]["info"]["layers"][0]["id"].as_u64().unwrap_or(0);
        assert_eq!(created["data"]["layerId"].as_u64().unwrap_or(0), first_id, "layerId 应等于最顶层 id");
        assert_eq!(created["data"]["info"]["layers"][0]["name"].as_str().unwrap_or(""), "图层 1", "自动命名");
        assert_eq!(layer_count(&ctx), base + 1, "新建后层数 +1");
        assert_eq!(ctx.revision(), rev0 + 1, "新建只应递增一次 revision");

        // 2) 再建一次：自动编号递增到「图层 2」。
        let second = read_json(pc_layer_create(ctx.session, ctx.doc, 0, b"".as_ptr(), 0), "pc_layer_create 第二次");
        assert_ok(&second, "pc_layer_create 第二次");
        assert_eq!(second["data"]["info"]["layers"][0]["name"].as_str().unwrap_or(""), "图层 2", "自动编号递增");

        // 3) above=Background：插到该层正上方（列表倒数第二行），并使用自定义名。
        let named = read_json(pc_layer_create(ctx.session, ctx.doc, background, b"custom".as_ptr(), 6), "pc_layer_create above=Background");
        assert_ok(&named, "pc_layer_create above=Background");
        let named_id = named["data"]["layerId"].as_u64().unwrap_or(0);
        let rows = named["data"]["info"]["layers"].as_array().cloned().unwrap_or_default();
        let named_name = rows.iter().find(|r| r["id"].as_u64() == Some(named_id)).and_then(|r| r["name"].as_str()).unwrap_or("");
        assert_eq!(named_name, "custom", "自定义名");
        let above_bg_index = rows.iter().position(|r| r["id"].as_u64() == Some(named_id));
        let bg_index = rows.iter().position(|r| r["id"].as_u64() == Some(background));
        assert_eq!(above_bg_index.map(|i| i + 1), bg_index, "新层应紧贴 Background 上方");
        assert_eq!(layer_count(&ctx), base + 3, "第三层已插入");

        // 4) 失败零副作用：above 不存在时文本带 id，层数与 revision 都不变。
        let before = layer_count(&ctx);
        let rev_before = ctx.revision();
        let missing = read_json(pc_layer_create(ctx.session, ctx.doc, 999_999, b"".as_ptr(), 0), "pc_layer_create 缺失 above");
        assert_eq!(missing["ok"].as_bool(), Some(false), "缺失 above 应失败");
        assert_eq!(missing["error"].as_str().unwrap_or(""), "layer not found: 999999", "错误文本");
        assert_eq!(layer_count(&ctx), before, "失败不得改动层数");
        assert_eq!(ctx.revision(), rev_before, "失败不得递增 revision");

        // 5) 一次 undo 只撤销最后一层。
        ctx.undo();
        assert_eq!(layer_count(&ctx), base + 2, "undo 一次应回到两层");
    }

    /// E23：新建文档 ABI —— 尺寸、白色 Background、默认名与尺寸校验。
    #[test]
    fn e23_document_new_creates_white_background() {
        let ctx = open_sample();
        let created = read_json(pc_document_new(ctx.session, 64, 48, b"".as_ptr(), 0), "pc_document_new");
        assert_ok(&created, "pc_document_new");
        let id = created["data"]["id"].as_u64().unwrap_or(0);
        assert_ne!(id, ctx.doc, "新文档 id 应不同于已有文档");
        let info = &created["data"]["info"];
        assert_eq!(info["width"].as_u64(), Some(64), "宽度");
        assert_eq!(info["height"].as_u64(), Some(48), "高度");
        assert_eq!(info["name"].as_str(), Some("Untitled"), "空名字回落到 Untitled");
        let layers = info["layers"].as_array().cloned().unwrap_or_default();
        assert_eq!(layers.len(), 1, "应只有一个 Background 层");
        assert_eq!(layers[0]["name"].as_str(), Some("Background"), "层名");

        // 白底：8×8 取样全部为不透明白色。
        let px = render_region(ctx.session, id, 0, 0, 8, 8, "e23 白底");
        assert_eq!(&px[..4], &[255, 255, 255, 255], "Background 应为不透明白色");

        // 校验：0 尺寸与超限尺寸都失败，错误文本固定。
        let zero = read_json(pc_document_new(ctx.session, 0, 10, b"".as_ptr(), 0), "pc_document_new 0");
        assert_eq!(zero["ok"].as_bool(), Some(false), "0 尺寸应失败");
        assert_eq!(zero["error"].as_str(), Some("document size must be positive"), "0 尺寸错误文本");
        let big = read_json(pc_document_new(ctx.session, 20000, 10, b"".as_ptr(), 0), "pc_document_new 超限");
        assert_eq!(big["ok"].as_bool(), Some(false), "单边超限应失败");
        assert_eq!(big["error"].as_str(), Some("document side exceeds 16384"), "超限错误文本");

// 自定义名。
        let named = read_json(pc_document_new(ctx.session, 16, 16, b"custom".as_ptr(), 6), "pc_document_new 命名");
        assert_ok(&named, "pc_document_new 命名");
        assert_eq!(named["data"]["info"]["name"].as_str(), Some("custom"), "自定义名");
    }

    /// E24：画布翻转/旋转 ABI —— 像素位移、宽高交换、撤销与不支持图层类型的拒绝。
    #[test]
    fn e24_document_turn_moves_pixels_and_undoes() {
        let ctx = open_sample();
        let created = read_json(pc_document_new(ctx.session, 10, 4, b"".as_ptr(), 0), "e24 建图");
        assert_ok(&created, "e24 建图");
        let id = created["data"]["id"].as_u64().unwrap_or(0);
        {
            // SAFETY: 测试独占该 session，且没有 C ABI 调用同时进行。
            let session = unsafe { &mut *ctx.session };
            let entry = session.docs.get_mut(&id).unwrap_or_else(|| panic!("e24 文档存在"));
            let layer = &mut entry.doc.layers[0];
            layer.surface_mut().unwrap_or_else(|| panic!("e24 Background surface")).fill_rect(Rect::new(0, 0, 2, 1), &[1.0, 0.0, 0.0, 1.0]);
        }
        let red_at = |x: i32, y: i32| -> bool {
            let px = render_region(ctx.session, id, x, y, 1, 1, "e24 取样");
            px.len() >= 4 && px[0] > 240 && px[1] < 20 && px[2] < 20
        };
        assert!(red_at(0, 0), "基线：左上角应为红");

        // 水平翻转：x 0..2 的红点移到 8..10。
        let flipped = read_json(pc_document_turn(ctx.session, id, 0), "pc_document_turn 翻转");
        assert_ok(&flipped, "pc_document_turn 翻转");
        assert_eq!(flipped["data"]["info"]["width"].as_u64(), Some(10), "翻转不改宽");
        assert!(red_at(9, 0), "水平翻转后右侧应为红");
        assert!(!red_at(0, 0), "水平翻转后左侧应为白");

        // 撤销回基线。
        let undone = read_json(pc_undo(ctx.session, id), "e24 undo");
        assert_ok(&undone, "e24 undo");
        assert!(red_at(0, 0), "撤销后左上恢复为红");

        // 顺时针 90°：10×4 → 4×10，红点 (1,0) → (3,1)。
        let rotated = read_json(pc_document_turn(ctx.session, id, 3), "pc_document_turn 90cw");
        assert_ok(&rotated, "pc_document_turn 90cw");
        assert_eq!(rotated["data"]["info"]["width"].as_u64(), Some(4), "90° 后宽=4");
        assert_eq!(rotated["data"]["info"]["height"].as_u64(), Some(10), "90° 后高=10");
        assert!(red_at(3, 1), "90°cw 后红点应落在 (3,1)");

        // 非法编码。
        let bad = read_json(pc_document_turn(ctx.session, id, 9), "pc_document_turn 非法");
        assert_eq!(bad["ok"].as_bool(), Some(false), "非法编码应失败");
        assert_eq!(bad["error"].as_str(), Some("invalid turn: expected 0..=4"), "错误文本");

        // 文字层现在参与几何变换：可旋转、尺寸交换、可撤销。
        let text_doc = read_json(pc_document_new(ctx.session, 10, 4, b"".as_ptr(), 0), "e24 文字文档");
        assert_ok(&text_doc, "e24 文字文档");
        let tid = text_doc["data"]["id"].as_u64().unwrap_or(0);
        let text_params = r#"{"text":"A","size":24,"x":2,"y":1,"color":[1,1,1,1]}"#;
        let made = read_json(pc_text_create(ctx.session, tid, text_params.as_bytes().as_ptr(), text_params.len()), "e24 文字层");
        assert_ok(&made, "e24 文字层");
        let text_turn = read_json(pc_document_turn(ctx.session, tid, 3), "pc_document_turn 文字层");
        assert_ok(&text_turn, "文字层应可旋转");
        assert_eq!(text_turn["data"]["info"]["width"].as_u64(), Some(4), "文字文档 90° 后宽=4");
        assert_eq!(text_turn["data"]["info"]["height"].as_u64(), Some(10), "文字文档 90° 后高=10");
        let text_back = read_json(pc_undo(ctx.session, tid), "e24 文字 undo");
        assert_ok(&text_back, "e24 文字 undo");
        assert_eq!(text_back["data"]["width"].as_u64(), Some(10), "文字 undo 恢复宽=10");

        // 形状层同样参与几何变换（缓存像素与 path 一起转）。
        let shape_doc = read_json(pc_document_new(ctx.session, 10, 4, b"".as_ptr(), 0), "e24 形状文档");
        assert_ok(&shape_doc, "e24 形状文档");
        let sid = shape_doc["data"]["id"].as_u64().unwrap_or(0);
        let shape_params = r#"{"kind":"rect","x":1,"y":1,"w":2,"h":1,"fill":[1,0,0,1]}"#;
        let shape_made = read_json(pc_shape_create(ctx.session, sid, shape_params.as_bytes().as_ptr(), shape_params.len()), "e24 形状层");
        assert_ok(&shape_made, "e24 形状层");
        let shape_turn = read_json(pc_document_turn(ctx.session, sid, 3), "pc_document_turn 形状层");
        assert_ok(&shape_turn, "形状层应可旋转");
        assert_eq!(shape_turn["data"]["info"]["width"].as_u64(), Some(4), "形状文档 90° 后宽=4");
        assert_eq!(shape_turn["data"]["info"]["height"].as_u64(), Some(10), "形状文档 90° 后高=10");
    }

    /// E25：历史记录 ABI —— 操作标签栈与 undo/redo 同步移动。
    #[test]
    fn e25_history_labels_track_undo_and_redo() {
        let ctx = open_sample();
        let labels = |value: &Value, key: &str| -> Vec<String> {
            value["data"][key]
                .as_array()
                .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
                .unwrap_or_default()
        };

        let empty = read_json(pc_history(ctx.session, ctx.doc), "pc_history 初始");
        assert_ok(&empty, "pc_history 初始");
        assert_eq!(labels(&empty, "undo").len(), 0, "初始 undo 为空");

        // 三个可辨识操作：新建图层 → 复制图层 → 旋转/翻转。
        let created = read_json(pc_layer_create(ctx.session, ctx.doc, 0, b"".as_ptr(), 0), "e25 新建");
        assert_ok(&created, "e25 新建");
        let layer = created["data"]["layerId"].as_u64().unwrap_or(0);
        let dup = read_json(pc_layer_duplicate(ctx.session, ctx.doc, layer), "e25 复制");
        assert_ok(&dup, "e25 复制");
        let turned = read_json(pc_document_turn(ctx.session, ctx.doc, 2), "e25 旋转");
        assert_ok(&turned, "e25 旋转");

        let h = read_json(pc_history(ctx.session, ctx.doc), "pc_history 三步后");
        assert_ok(&h, "pc_history 三步后");
        assert_eq!(labels(&h, "undo"), vec!["旋转/翻转", "复制图层", "新建图层"], "undo 标签最近在前");
        assert_eq!(labels(&h, "redo").len(), 0, "redo 为空");

        // 撤销一次：最近标签从 undo 移到 redo。
        let undone = read_json(pc_undo(ctx.session, ctx.doc), "e25 undo");
        assert_ok(&undone, "e25 undo");
        let h2 = read_json(pc_history(ctx.session, ctx.doc), "pc_history undo 后");
        assert_ok(&h2, "pc_history undo 后");
        assert_eq!(labels(&h2, "undo"), vec!["复制图层", "新建图层"], "undo 少一步");
        assert_eq!(labels(&h2, "redo"), vec!["旋转/翻转"], "redo 接住标签");

        // 重做：标签回到 undo 顶部。
        let redone = read_json(pc_redo(ctx.session, ctx.doc), "e25 redo");
        assert_ok(&redone, "e25 redo");
        let h3 = read_json(pc_history(ctx.session, ctx.doc), "pc_history redo 后");
        assert_ok(&h3, "pc_history redo 后");
        assert_eq!(labels(&h3, "undo"), vec!["旋转/翻转", "复制图层", "新建图层"], "redo 后标签回位");
        assert_eq!(labels(&h3, "redo").len(), 0, "redo 清空");
    }

    /// E26：批量历史跳转 —— 一次调用只递增一次 revision，标签栈同步，超深退化为撤到底。
    #[test]
    fn e26_history_jump_steps_batches_revision() {
        let ctx = open_sample();
        let labels = |value: &Value, key: &str| -> Vec<String> {
            value["data"][key]
                .as_array()
                .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
                .unwrap_or_default()
        };

        let created = read_json(pc_layer_create(ctx.session, ctx.doc, 0, b"".as_ptr(), 0), "e26 新建");
        assert_ok(&created, "e26 新建");
        let layer = created["data"]["layerId"].as_u64().unwrap_or(0);
        let dup = read_json(pc_layer_duplicate(ctx.session, ctx.doc, layer), "e26 复制");
        assert_ok(&dup, "e26 复制");
        let turned = read_json(pc_document_turn(ctx.session, ctx.doc, 3), "e26 旋转");
        assert_ok(&turned, "e26 旋转");
        assert_eq!(turned["data"]["info"]["width"].as_u64(), Some(1500), "旋转后宽=1500");

        // 一次跳转 2 步：只递增一次 revision，尺寸回到旋转前。
        let rev_before = ctx.revision();
        let jumped = read_json(pc_undo_steps(ctx.session, ctx.doc, 2), "pc_undo_steps 2");
        assert_ok(&jumped, "pc_undo_steps 2");
        assert_eq!(jumped["data"]["steps"].as_u64(), Some(2), "实际跳转 2 步");
        assert_eq!(jumped["data"]["info"]["width"].as_u64(), Some(2400), "跳转后宽回 2400");
        assert_eq!(ctx.revision(), rev_before + 1, "批量跳转只递增一次 revision");
        let h = read_json(pc_history(ctx.session, ctx.doc), "e26 跳转后 history");
        assert_eq!(labels(&h, "undo"), vec!["新建图层"], "undo 剩一步");
        assert_eq!(labels(&h, "redo"), vec!["复制图层", "旋转/翻转"], "redo 下一步在前");

        // 重做一步：标签回位。
        let redone = read_json(pc_redo_steps(ctx.session, ctx.doc, 1), "pc_redo_steps 1");
        assert_ok(&redone, "pc_redo_steps 1");
        assert_eq!(redone["data"]["steps"].as_u64(), Some(1), "实际重做 1 步");
        let h2 = read_json(pc_history(ctx.session, ctx.doc), "e26 重做后 history");
        assert_eq!(labels(&h2, "undo"), vec!["复制图层", "新建图层"], "undo 两步");

        // 边界：steps=0 拒绝；超深退化为撤到底。
        let zero = read_json(pc_undo_steps(ctx.session, ctx.doc, 0), "pc_undo_steps 0");
        assert_eq!(zero["ok"].as_bool(), Some(false), "0 步应失败");
        assert_eq!(zero["error"].as_str(), Some("steps must be positive"), "0 步错误文本");
        let all = read_json(pc_undo_steps(ctx.session, ctx.doc, 99), "pc_undo_steps 99");
        assert_ok(&all, "pc_undo_steps 99");
        assert_eq!(all["data"]["steps"].as_u64(), Some(2), "超深只撤到栈底");
        assert_eq!(all["data"]["info"]["width"].as_u64(), Some(2400), "撤到底回到原始尺寸");
    }
}

/// 蒙版 C ABI 像素级回归测试（`C-MASK-TEST@1`，计划 T3）。
///
/// 全部用例走真实导出的 C ABI（`pc_open` / `pc_render_pixels` / `pc_select` /
/// `pc_layer_visible` / `pc_layer_mask_*` / `pc_revision` / `pc_undo` / `pc_info`），
/// 以「隐藏整层」与「无蒙版」两条基线做像素差分，避免"差分恒为 0 但两端都错"的假 PASS。
/// 独立 mod：自带全部 helper，不引用 `effects_api_tests` / `edit_api_tests` 的私有项。
#[cfg(test)]
mod mask_api_tests {
    //! T3：`C-MASK-TEST@1` E1–E10。
    #![allow(clippy::panic, clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

    use super::*;

    /// 仓库自带样图（2400x1500），与既有测试同一路径。
    const SAMPLE_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../entry/src/main/resources/rawfile/sample.jpg");
    /// `C-MASK-TEST@1 §B`：主取样区域 `(x, y, w, h)`，与矩形选区同域。
    const MASK_REGION: (i32, i32, i32, i32) = (100, 100, 300, 300);
    /// `C-MASK-TEST@1 §B`：边界取样区域 `(x, y, w, h)`，包含选区外缓冲，用于 R2 的边界断言。
    const EDGE_REGION: (i32, i32, i32, i32) = (60, 60, 380, 380);
    /// 矩形选区 `(x0, y0, x1, y1)`：与 §1.3 一致，`100<=x<300` 且 `100<=y<300`。
    const SELECT_RECT: (i32, i32, i32, i32) = (100, 100, 300, 300);
    /// E9 构造「可挂蒙版但不可烘焙」的形状层（与既有编辑用例同语义的样本）。
    const SHAPE_PARAMS: &str = r#"{"kind":"rect","x":300,"y":300,"w":200,"h":150,"fill":[0.2,0.4,0.9,1]}"#;
    /// 浮点字段（density / feather 由 f32 回读）比较容差。
    const EPS: f64 = 1e-6;

    // ---------------------------------------------------------------- 基础设施

    /// 读取并释放一个 `PcBuffer`。失败信息包含调用上下文。
    fn take_bytes(buf: *mut PcBuffer, ctx: &str) -> Vec<u8> {
        assert!(!buf.is_null(), "{ctx}: 期望非空 PcBuffer, 实际 null");
        // SAFETY: `buf` 由桥接层 `PcBuffer::from_vec` 分配，读取后立即释放且只释放一次。
        let bytes = unsafe {
            let b = &*buf;
            std::slice::from_raw_parts(b.ptr, b.len).to_vec()
        };
        pc_buffer_free(buf);
        bytes
    }

    /// 读取 JSON 响应并释放缓冲区。
    fn read_json(buf: *mut PcBuffer, ctx: &str) -> Value {
        let bytes = take_bytes(buf, ctx);
        match serde_json::from_slice::<Value>(&bytes) {
            Ok(value) => value,
            Err(e) => {
                let text = String::from_utf8_lossy(&bytes).into_owned();
                panic!("{ctx}: 期望合法 JSON, 实际解析失败 ({e}): {text}")
            }
        }
    }

    fn assert_ok(state: &Value, ctx: &str) {
        assert_eq!(state["ok"].as_bool(), Some(true), "{ctx}: 期望 ok=true, 实际 {state}");
    }

    /// 断言失败响应：`ok=false` 且 `error` 含指定片段（`C-MASK-API@1 §D` 的错误文本）。
    fn assert_err(state: &Value, ctx: &str, needle: &str) {
        assert_eq!(state["ok"].as_bool(), Some(false), "{ctx}: 期望 ok=false, 实际 {state}");
        let error = state["error"].as_str().unwrap_or("<缺失>");
        assert!(
            error.contains(needle),
            "{ctx}: 期望 error 包含 {needle:?}, 实际 {error:?}（完整响应 {state}）"
        );
    }

    fn assert_bool(item: &Value, field: &str, expected: bool, ctx: &str) {
        match item.get(field).and_then(Value::as_bool) {
            Some(actual) if actual == expected => {}
            Some(actual) => panic!("{ctx}: 字段 {field} 期望 {expected}, 实际 {actual}（item={item}）"),
            None => panic!("{ctx}: 字段 {field} 期望 {expected}, 实际缺失（item={item}）"),
        }
    }

    fn assert_num(item: &Value, field: &str, expected: f64, ctx: &str) {
        match item.get(field).and_then(Value::as_f64) {
            Some(actual) if (actual - expected).abs() <= EPS => {}
            Some(actual) => panic!("{ctx}: 字段 {field} 期望 {expected}, 实际 {actual}（item={item}）"),
            None => panic!("{ctx}: 字段 {field} 期望 {expected}, 实际缺失（item={item}）"),
        }
    }

    /// 读取必填数值字段（输出行 / 打印用）。
    fn number(item: &Value, field: &str, ctx: &str) -> f64 {
        item.get(field)
            .and_then(Value::as_f64)
            .unwrap_or_else(|| panic!("{ctx}: 字段 {field} 期望数值, 实际缺失（item={item}）"))
    }

    // ---------------------------------------------------------------- 差分

    /// 一块区域的逐像素差异统计（`C-MASK-TEST@1 §B`）。
    struct Diff {
        /// 参与比较的像素数（过滤后）。
        pixels: usize,
        /// 任一 RGBA 字节差 > 1 的像素数。
        changed: usize,
        /// 所有比较字节绝对差的算术平均（0..255 标度）。
        mean: f64,
        /// `changed / pixels × 100`。
        pct: f64,
        /// 首个差异像素：线性索引与两侧 RGBA。
        first: Option<(usize, [u8; 4], [u8; 4])>,
    }

    /// 契约 §B 的 `diff_rgba(a, b)`：全区域差分（坐标由 [`diff_rgba_region`] 提供）。
    fn diff_rgba(a: &[u8], b: &[u8]) -> Diff {
        diff_rgba_region(a, b, (0, 0, 1, 0), |_, _| true)
    }

    /// 在 `region = (x, y, w, h)` 上差分；`keep` 过滤像素，参数是**文档绝对坐标**
    /// （由区域原点 + 区域内相对坐标换算，与选区的坐标系一致）。
    ///
    /// `mean` = 所有比较字节绝对差之和 / 比较字节数；`changed` = 任一字节差 > 1 的像素数；
    /// `pct = changed / 比较像素数 × 100`。
    fn diff_rgba_region(
        a: &[u8],
        b: &[u8],
        region: (i32, i32, i32, i32),
        keep: impl Fn(i32, i32) -> bool,
    ) -> Diff {
        assert_eq!(a.len(), b.len(), "差分: 期望两张裸 RGBA 长度一致, 实际 {} vs {}", a.len(), b.len());
        assert_eq!(a.len() % 4, 0, "差分: 期望像素字节数为 4 的倍数, 实际 {}", a.len());
        let w = region.2.max(1);
        let total = a.len() / 4;
        let mut pixels = 0usize;
        let mut changed = 0usize;
        let mut byte_sum = 0u64;
        let mut first: Option<(usize, [u8; 4], [u8; 4])> = None;
        for index in 0..total {
            let x = region.0 + (index as i32) % w;
            let y = region.1 + (index as i32) / w;
            if !keep(x, y) {
                continue;
            }
            pixels += 1;
            let base = [a[index * 4], a[index * 4 + 1], a[index * 4 + 2], a[index * 4 + 3]];
            let cur = [b[index * 4], b[index * 4 + 1], b[index * 4 + 2], b[index * 4 + 3]];
            let mut hit = false;
            for channel in 0..4 {
                let delta = base[channel].abs_diff(cur[channel]);
                byte_sum += u64::from(delta);
                if delta > 1 {
                    hit = true;
                }
            }
            if hit {
                changed += 1;
                if first.is_none() {
                    first = Some((index, base, cur));
                }
            }
        }
        let compared_bytes = pixels * 4;
        Diff {
            pixels,
            changed,
            mean: if compared_bytes == 0 { 0.0 } else { byte_sum as f64 / compared_bytes as f64 },
            pct: if pixels == 0 { 0.0 } else { changed as f64 / pixels as f64 * 100.0 },
            first,
        }
    }

    /// 把首个差异像素格式化成绝对坐标 + 两侧 RGBA（`C-MASK-TEST@1 §D` 的失败信息要求）。
    fn describe_first(first: Option<(usize, [u8; 4], [u8; 4])>, region: (i32, i32, i32, i32)) -> String {
        match first {
            Some((index, base, cur)) => {
                let w = region.2.max(1);
                let x = region.0 + (index as i32) % w;
                let y = region.1 + (index as i32) / w;
                format!("首个差异像素 (x={x}, y={y}) 基线 RGBA={base:?} 实际 RGBA={cur:?}")
            }
            None => "首个差异像素: 无".to_string(),
        }
    }

    /// 断言区域差分不超过 `limit`（`0.0` 即逐字节一致）；失败信息含区域/像素数/差异数/首个差异像素。
    fn assert_mean_at_most(
        ctx: &str,
        region: (i32, i32, i32, i32),
        baseline: &[u8],
        rendered: &[u8],
        keep: impl Fn(i32, i32) -> bool,
        limit: f64,
    ) -> Diff {
        let diff = diff_rgba_region(baseline, rendered, region, keep);
        assert!(
            diff.mean <= limit + EPS,
            "{ctx}: 区域={region:?} 基线像素数={} 差异像素数={} 占比={:.4}% mean={:.6} 期望 mean<={limit:.6}；{}",
            diff.pixels,
            diff.changed,
            diff.pct,
            diff.mean,
            describe_first(diff.first, region)
        );
        diff
    }

    /// 断言两幅画面逐字节一致（`mean == 0.0`）。
    fn assert_same(ctx: &str, region: (i32, i32, i32, i32), baseline: &[u8], rendered: &[u8]) -> Diff {
        assert_mean_at_most(ctx, region, baseline, rendered, |_, _| true, 0.0)
    }

    /// 断言两幅画面确实不同（双基线的反向证据，避免"两端都错"的假 PASS）。
    fn assert_differs(ctx: &str, region: (i32, i32, i32, i32), baseline: &[u8], rendered: &[u8]) -> Diff {
        let diff = diff_rgba_region(baseline, rendered, region, |_, _| true);
        assert!(
            diff.mean > 0.0 || diff.changed > 0,
            "{ctx}: 期望两幅画面不同（区域={region:?}），实际 mean=0.0000 changed=0；{}",
            describe_first(diff.first, region)
        );
        diff
    }

    // ---------------------------------------------------------------- 测试上下文

    /// 一个已打开的样图文档；`Drop` 释放 session。
    struct MaskCtx {
        session: *mut Session,
        doc: u64,
    }

    impl Drop for MaskCtx {
        fn drop(&mut self) {
            pc_session_free(self.session);
        }
    }

    impl MaskCtx {
        /// `pc_revision` 的 `data.revision`。
        fn rev(&self) -> u64 {
            let state = read_json(pc_revision(self.session, self.doc), "pc_revision");
            assert_ok(&state, "pc_revision");
            state["data"]["revision"]
                .as_u64()
                .unwrap_or_else(|| panic!("pc_revision: 期望 data.revision 为 u64, 实际 {state}"))
        }

        /// `pc_render_pixels` → 裸 RGBA8（`w * h * 4`），并断言长度契约。
        fn render(&self, region: (i32, i32, i32, i32), ctx: &str) -> Vec<u8> {
            let (x, y, w, h) = region;
            let mut out: *mut PcBuffer = std::ptr::null_mut();
            let state = read_json(pc_render_pixels(self.session, self.doc, x, y, w, h, &mut out), ctx);
            assert_ok(&state, ctx);
            assert!(!out.is_null(), "{ctx}: 期望 out 非空, 实际 null");
            let pixels = take_bytes(out, ctx);
            let expected = w as usize * h as usize * 4;
            assert_eq!(pixels.len(), expected, "{ctx}: 期望 {w}x{h} 的裸 RGBA 共 {expected} 字节, 实际 {}", pixels.len());
            pixels
        }

        /// `pc_layer_mask_get` 原始响应。
        fn mask_json(&self, layer: u64) -> Value {
            read_json(pc_layer_mask_get(self.session, self.doc, layer), &format!("pc_layer_mask_get layer={layer}"))
        }

        /// `pc_layer_mask_get` 的成功响应 `data`（断言 `ok`）。
        fn mask_data(&self, layer: u64, ctx: &str) -> Value {
            let state = self.mask_json(layer);
            assert_ok(&state, &format!("{ctx} pc_layer_mask_get layer={layer}"));
            state["data"].clone()
        }

        fn add_raw(&self, layer: u64, fill: &str) -> Value {
            read_json(
                pc_layer_mask_add(self.session, self.doc, layer, fill.as_bytes().as_ptr(), fill.len()),
                &format!("pc_layer_mask_add layer={layer} fill={fill}"),
            )
        }

        fn add(&self, layer: u64, fill: &str) -> Value {
            let state = self.add_raw(layer, fill);
            assert_ok(&state, &format!("pc_layer_mask_add layer={layer} fill={fill}"));
            state
        }

        fn set_raw(&self, layer: u64, params: &str) -> Value {
            read_json(
                pc_layer_mask_set(self.session, self.doc, layer, params.as_bytes().as_ptr(), params.len()),
                &format!("pc_layer_mask_set layer={layer} {params}"),
            )
        }

        fn set(&self, layer: u64, params: &str) -> Value {
            let state = self.set_raw(layer, params);
            assert_ok(&state, &format!("pc_layer_mask_set layer={layer} {params}"));
            state
        }

        fn fill_raw(&self, layer: u64, mode: &str) -> Value {
            read_json(
                pc_layer_mask_fill_from_selection(
                    self.session,
                    self.doc,
                    layer,
                    mode.as_bytes().as_ptr(),
                    mode.len(),
                ),
                &format!("pc_layer_mask_fill_from_selection layer={layer} mode={mode}"),
            )
        }

        fn fill(&self, layer: u64, mode: &str) -> Value {
            let state = self.fill_raw(layer, mode);
            assert_ok(&state, &format!("pc_layer_mask_fill_from_selection layer={layer} mode={mode}"));
            state
        }

        fn apply_raw(&self, layer: u64) -> Value {
            read_json(pc_layer_mask_apply(self.session, self.doc, layer), &format!("pc_layer_mask_apply layer={layer}"))
        }

        fn apply(&self, layer: u64) -> Value {
            let state = self.apply_raw(layer);
            assert_ok(&state, &format!("pc_layer_mask_apply layer={layer}"));
            state
        }

        fn remove_raw(&self, layer: u64) -> Value {
            read_json(pc_layer_mask_remove(self.session, self.doc, layer), &format!("pc_layer_mask_remove layer={layer}"))
        }

        fn remove(&self, layer: u64) -> Value {
            let state = self.remove_raw(layer);
            assert_ok(&state, &format!("pc_layer_mask_remove layer={layer}"));
            state
        }

        /// 切换整层可见性（用于构造「隐藏层基线」）。
        fn set_visible(&self, layer: u64, visible: bool) {
            let ctx = format!("pc_layer_visible layer={layer} visible={visible}");
            let state = read_json(pc_layer_visible(self.session, self.doc, layer, visible), &ctx);
            assert_ok(&state, &ctx);
        }

        fn select(&self, params: &str, ctx: &str) {
            let state = read_json(pc_select(self.session, self.doc, params.as_bytes().as_ptr(), params.len()), ctx);
            assert_ok(&state, ctx);
        }

        /// 建立矩形选区：参数是 `(x0, y0, x1, y1)`（文档坐标）。
        fn select_rect(&self, rect: (i32, i32, i32, i32)) {
            let (x0, y0, x1, y1) = rect;
            let params = format!(r#"{{"kind":"rect","x0":{x0},"y0":{y0},"x1":{x1},"y1":{y1}}}"#);
            self.select(&params, &format!("pc_select rect {rect:?}"));
        }

        fn select_none(&self) {
            self.select(r#"{"kind":"none"}"#, "pc_select none");
        }

        /// `pc_selection_info` 的 `data`（`{"active","x0","y0","x1","y1"}`）。
        fn selection_data(&self, ctx: &str) -> Value {
            let state = read_json(pc_selection_info(self.session, self.doc), ctx);
            assert_ok(&state, ctx);
            state["data"].clone()
        }

        /// `pc_info` 的 `data`。
        fn info(&self, ctx: &str) -> Value {
            let state = read_json(pc_info(self.session, self.doc), ctx);
            assert_ok(&state, ctx);
            state["data"].clone()
        }

        fn can_undo(&self) -> bool {
            self.info("pc_info canUndo")["canUndo"]
                .as_bool()
                .unwrap_or_else(|| panic!("pc_info: 字段 canUndo 缺失"))
        }

        fn can_redo(&self) -> bool {
            self.info("pc_info canRedo")["canRedo"]
                .as_bool()
                .unwrap_or_else(|| panic!("pc_info: 字段 canRedo 缺失"))
        }

        fn undo(&self, ctx: &str) -> Value {
            let state = read_json(pc_undo(self.session, self.doc), ctx);
            assert_ok(&state, ctx);
            state
        }

        /// 新建形状层并返回 id（E9 构造「可挂蒙版但不可 apply」目标）。
        fn create_shape(&self, params: &str) -> u64 {
            let state = read_json(
                pc_shape_create(self.session, self.doc, params.as_bytes().as_ptr(), params.len()),
                &format!("pc_shape_create {params}"),
            );
            assert_ok(&state, &format!("pc_shape_create {params}"));
            state["data"]["layerId"]
                .as_u64()
                .unwrap_or_else(|| panic!("pc_shape_create: 期望 data.layerId, 实际 {state}"))
        }

        /// 新建调整层并返回 id（E9 构造「不可挂蒙版」目标，语义等同组/填充层）。
        fn create_adjustment(&self, kind: &str) -> u64 {
            let params = "{}";
            let state = read_json(
                pc_adjustment_add(
                    self.session,
                    self.doc,
                    kind.as_bytes().as_ptr(),
                    kind.len(),
                    params.as_bytes().as_ptr(),
                    params.len(),
                ),
                &format!("pc_adjustment_add {kind}"),
            );
            assert_ok(&state, &format!("pc_adjustment_add {kind}"));
            state["data"]["layerId"]
                .as_u64()
                .unwrap_or_else(|| panic!("pc_adjustment_add: 期望 data.layerId, 实际 {state}"))
        }

        /// 新建文字图层并返回 id（E13 构造「可挂蒙版但不可烘焙」的文字层）。
        fn create_text(&self, params: &str) -> u64 {
            let state = read_json(
                pc_text_create(self.session, self.doc, params.as_bytes().as_ptr(), params.len()),
                &format!("pc_text_create {params}"),
            );
            assert_ok(&state, &format!("pc_text_create {params}"));
            state["data"]["layerId"]
                .as_u64()
                .unwrap_or_else(|| panic!("pc_text_create: 期望 data.layerId, 实际 {state}"))
        }
    }

    /// 打开样图，返回独占的测试上下文。
    fn open_sample() -> MaskCtx {
        let raw = std::fs::read(SAMPLE_PATH).unwrap_or_else(|e| panic!("读取样图失败 {SAMPLE_PATH}: {e}"));
        let session = pc_session_new();
        assert!(!session.is_null(), "pc_session_new: 期望非空 session, 实际 null");
        let name = b"sample.jpg";
        let opened = read_json(pc_open(session, name.as_ptr(), name.len(), raw.as_ptr(), raw.len()), "pc_open");
        assert_ok(&opened, "pc_open");
        let doc = opened["data"]["id"]
            .as_u64()
            .unwrap_or_else(|| panic!("pc_open: 期望 data.id, 实际 {opened}"));
        MaskCtx { session, doc }
    }

    /// 样图自带的像素图层 id（`pc_info.data.layers` 顶层在前，样图只有一层 Background）。
    fn sample_base_layer(ctx: &MaskCtx) -> u64 {
        let info = ctx.info("pc_info sample_base_layer");
        let layers = info["layers"]
            .as_array()
            .unwrap_or_else(|| panic!("pc_info: 期望 data.layers 为数组, 实际 {info}"));
        layers
            .iter()
            .rev()
            .find_map(|node| match node["kind"].as_str() {
                Some("Pixel") => node["id"].as_u64(),
                _ => None,
            })
            .unwrap_or_else(|| panic!("pc_info: 样图中没有像素图层, 实际 {info}"))
    }

    // ---------------------------------------------------------------- 双基线

    /// 双基线：同一文档内先取「无蒙版」基线，再隐藏整层取「隐藏层」基线，随后恢复可见并校验
    /// 画面逐字节复原；最后断言两条基线**确实不同**（否则"差分恒为 0"可能两端都错）。
    ///
    /// 返回 `(无蒙版基线, 隐藏层基线)`。注意：两次 `pc_layer_visible` 各产生 1 个 checkpoint。
    fn dual_baseline(ctx: &MaskCtx, layer: u64, region: (i32, i32, i32, i32), tag: &str) -> (Vec<u8>, Vec<u8>) {
        let no_mask = ctx.render(region, &format!("{tag} 无蒙版基线"));
        ctx.set_visible(layer, false);
        let hidden = ctx.render(region, &format!("{tag} 隐藏层基线"));
        ctx.set_visible(layer, true);
        let restored = ctx.render(region, &format!("{tag} 恢复可见"));
        assert_same(&format!("{tag} 隐藏再显示后"), region, &no_mask, &restored);
        assert_differs(&format!("{tag} 前置: 隐藏层基线必须不同于无蒙版基线"), region, &no_mask, &hidden);
        // 两条基线的整体差距（契约 §B 的 `diff_rgba` 口径），作为可解析的反向证据输出。
        let gap = diff_rgba(&no_mask, &hidden);
        eprintln!(
            "{tag} baseline no_mask_px={} hidden_px={} gap_mean={:.4} gap_pct={:.4}%",
            no_mask.len() / 4,
            hidden.len() / 4,
            gap.mean,
            gap.pct
        );
        (no_mask, hidden)
    }

    /// 选区内外判定：与 `SELECT_RECT` 一致（硬边界，无羽化）。
    fn in_selection(x: i32, y: i32) -> bool {
        x >= SELECT_RECT.0 && x < SELECT_RECT.2 && y >= SELECT_RECT.1 && y < SELECT_RECT.3
    }

    /// 断言失败调用零副作用：`revision` 与调用前一致。
    fn assert_rev_unchanged(ctx: &MaskCtx, label: &str, before: u64) {
        let after = ctx.rev();
        assert_eq!(after, before, "{label}: 期望失败零副作用、revision 保持 {before}, 实际 {after}");
    }

    // ---------------------------------------------------------------- 用例

    /// E1：`add("reveal")` 后读模型全字段正确、revision +1，画面与操作前**逐字节一致**。
    #[test]
    fn e1_add_reveal_keeps_rendered_pixels() {
        let ctx = open_sample();
        let layer = sample_base_layer(&ctx);
        let rev0 = ctx.rev();
        let before = ctx.render(MASK_REGION, "e1 操作前");

        let state = ctx.add(layer, "reveal");
        let data = &state["data"];
        assert_bool(data, "present", true, "e1 add 回读");
        assert_bool(data, "enabled", true, "e1 add 回读");
        assert_bool(data, "linked", true, "e1 add 回读");
        assert_num(data, "density", 1.0, "e1 add 回读");
        assert_num(data, "feather", 0.0, "e1 add 回读");
        let rev1 = ctx.rev();
        assert_eq!(rev1, rev0 + 1, "e1 add 字段 revision: 期望 {}, 实际 {rev1}", rev0 + 1);

        let after = ctx.render(MASK_REGION, "e1 操作后");
        let diff = assert_same("e1 显示全部后渲染", MASK_REGION, &before, &after);

        let read = ctx.mask_data(layer, "e1 add 后 get");
        assert_eq!(&read, data, "e1 add 后 get 应与写响应一致, 实际 {read}");

        eprintln!(
            "e1 rev={rev0}->{rev1} mean={:.4} pct={:.4}% present={} density={:.3} feather={:.1}",
            diff.mean,
            diff.pct,
            data["present"].as_bool().unwrap_or(false),
            number(data, "density", "e1 输出"),
            number(data, "feather", "e1 输出")
        );
    }

    /// E2：`add("hide")` 的画面等于「隐藏整层」基线；停用等于「无蒙版」基线；重新启用回到隐藏基线。
    #[test]
    fn e2_add_hide_matches_hidden_layer() {
        let ctx = open_sample();
        let layer = sample_base_layer(&ctx);
        let (no_mask, hidden) = dual_baseline(&ctx, layer, EDGE_REGION, "e2");

        let rev0 = ctx.rev();
        let state = ctx.add(layer, "hide");
        let data = &state["data"];
        assert_bool(data, "present", true, "e2 add 回读");
        assert_bool(data, "enabled", true, "e2 add 回读");
        assert_bool(data, "linked", true, "e2 add 回读");
        assert_num(data, "density", 1.0, "e2 add 回读");
        assert_num(data, "feather", 0.0, "e2 add 回读");
        let rev1 = ctx.rev();
        assert_eq!(rev1, rev0 + 1, "e2 add 字段 revision: 期望 {}, 实际 {rev1}", rev0 + 1);

        let masked = ctx.render(EDGE_REGION, "e2 隐藏全部后");
        assert_differs("e2 隐藏全部必须改变画面（反向证据）", EDGE_REGION, &no_mask, &masked);
        assert_same("e2 隐藏全部 vs 隐藏层基线", EDGE_REGION, &hidden, &masked);

        // 停用：只切 enabled，画面回到「无蒙版」基线。
        let rev2 = ctx.rev();
        let disabled = ctx.set(layer, r#"{"enabled":false}"#);
        assert_bool(&disabled["data"], "present", true, "e2 停用回读");
        assert_bool(&disabled["data"], "enabled", false, "e2 停用回读");
        assert_num(&disabled["data"], "density", 1.0, "e2 停用回读");
        assert_num(&disabled["data"], "feather", 0.0, "e2 停用回读");
        let rev3 = ctx.rev();
        assert_eq!(rev3, rev2 + 1, "e2 停用字段 revision: 期望 {}, 实际 {rev3}", rev2 + 1);
        let off = ctx.render(EDGE_REGION, "e2 停用后");
        assert_same("e2 停用蒙版 vs 无蒙版基线", EDGE_REGION, &no_mask, &off);

        // 重新启用：回到隐藏基线。
        let rev4 = ctx.rev();
        let enabled = ctx.set(layer, r#"{"enabled":true}"#);
        assert_bool(&enabled["data"], "enabled", true, "e2 重新启用回读");
        let rev5 = ctx.rev();
        assert_eq!(rev5, rev4 + 1, "e2 重新启用字段 revision: 期望 {}, 实际 {rev5}", rev4 + 1);
        let on = ctx.render(EDGE_REGION, "e2 重新启用后");
        let diff = assert_same("e2 重新启用 vs 隐藏层基线", EDGE_REGION, &hidden, &on);

        eprintln!(
            "e2 rev={rev0}->{rev5} mean={:.4} pct={:.4}% present={} density={:.3} feather={:.1}",
            diff.mean,
            diff.pct,
            enabled["data"]["present"].as_bool().unwrap_or(false),
            number(&enabled["data"], "density", "e2 输出"),
            number(&enabled["data"], "feather", "e2 输出")
        );
    }

    /// E3：选区（`100..300`）×（`100..300`）内隐藏——选区内等于隐藏层基线、选区外等于无蒙版基线，
    /// 且操作后**保留选区**。
    #[test]
    fn e3_fill_from_selection_hide() {
        let ctx = open_sample();
        let layer = sample_base_layer(&ctx);
        let (no_mask, hidden) = dual_baseline(&ctx, layer, EDGE_REGION, "e3");
        ctx.select_rect(SELECT_RECT);

        let rev0 = ctx.rev();
        let state = ctx.fill(layer, "hide");
        let data = &state["data"];
        assert_bool(data, "present", true, "e3 fill 回读");
        assert_bool(data, "enabled", true, "e3 fill 回读");
        assert_num(data, "density", 1.0, "e3 fill 回读");
        assert_num(data, "feather", 0.0, "e3 fill 回读");
        let rev1 = ctx.rev();
        assert_eq!(rev1, rev0 + 1, "e3 fill 字段 revision: 期望 {}, 实际 {rev1}", rev0 + 1);

        let edge = ctx.render(EDGE_REGION, "e3 选区内隐藏后");
        let inside = assert_mean_at_most("e3 选区内 vs 隐藏层基线", EDGE_REGION, &hidden, &edge, in_selection, 0.0);
        let outside = assert_mean_at_most("e3 选区外 vs 无蒙版基线", EDGE_REGION, &no_mask, &edge, |x, y| {
            !in_selection(x, y)
        }, 0.0);
        assert_differs("e3 选区内隐藏必须改变画面（反向证据）", EDGE_REGION, &no_mask, &edge);

        // R2.2 / §E.4：选区必须保留。
        let selection = ctx.selection_data("pc_selection_info（e3 操作后）");
        assert_bool(&selection, "active", true, "e3 fill 后 selection");
        assert!(
            number(&selection, "x0", "e3 selection") <= f64::from(SELECT_RECT.0)
                && number(&selection, "y0", "e3 selection") <= f64::from(SELECT_RECT.1)
                && number(&selection, "x1", "e3 selection") >= f64::from(SELECT_RECT.2)
                && number(&selection, "y1", "e3 selection") >= f64::from(SELECT_RECT.3),
            "e3 fill 后 selection 应覆盖 {SELECT_RECT:?}, 实际 {selection}"
        );

        eprintln!(
            "e3 rev={rev0}->{rev1} mean={:.4} pct={:.4}% present={} density={:.3} feather={:.1} \
             outside_mean={:.4} inside_px={} outside_px={}",
            inside.mean,
            inside.pct,
            data["present"].as_bool().unwrap_or(false),
            number(data, "density", "e3 输出"),
            number(data, "feather", "e3 输出"),
            outside.mean,
            inside.pixels,
            outside.pixels
        );
    }

    /// E4：同一选区「选区内显示」——选区内等于无蒙版基线、选区外等于隐藏层基线。
    #[test]
    fn e4_fill_from_selection_reveal() {
        let ctx = open_sample();
        let layer = sample_base_layer(&ctx);
        let (no_mask, hidden) = dual_baseline(&ctx, layer, EDGE_REGION, "e4");
        ctx.select_rect(SELECT_RECT);

        let rev0 = ctx.rev();
        let state = ctx.fill(layer, "reveal");
        let data = &state["data"];
        assert_bool(data, "present", true, "e4 fill 回读");
        assert_bool(data, "enabled", true, "e4 fill 回读");
        assert_num(data, "density", 1.0, "e4 fill 回读");
        assert_num(data, "feather", 0.0, "e4 fill 回读");
        let rev1 = ctx.rev();
        assert_eq!(rev1, rev0 + 1, "e4 fill 字段 revision: 期望 {}, 实际 {rev1}", rev0 + 1);

        let edge = ctx.render(EDGE_REGION, "e4 选区内显示后");
        let inside = assert_mean_at_most("e4 选区内 vs 无蒙版基线", EDGE_REGION, &no_mask, &edge, in_selection, 0.0);
        let outside = assert_mean_at_most("e4 选区外 vs 隐藏层基线", EDGE_REGION, &hidden, &edge, |x, y| {
            !in_selection(x, y)
        }, 0.0);
        assert_differs("e4 选区内显示必须改变画面（反向证据）", EDGE_REGION, &no_mask, &edge);

        let selection = ctx.selection_data("pc_selection_info（e4 操作后）");
        assert_bool(&selection, "active", true, "e4 fill 后 selection");

        eprintln!(
            "e4 rev={rev0}->{rev1} mean={:.4} pct={:.4}% present={} density={:.3} feather={:.1} \
             outside_mean={:.4} inside_px={} outside_px={}",
            inside.mean,
            inside.pct,
            data["present"].as_bool().unwrap_or(false),
            number(data, "density", "e4 输出"),
            number(data, "feather", "e4 输出"),
            outside.mean,
            inside.pixels,
            outside.pixels
        );
    }

    /// E5：`hide` + `density=0` 的画面等于「无蒙版」基线（蒙版仍 present），恢复 density=1 后回到隐藏基线。
    #[test]
    fn e5_density_zero_equals_no_mask() {
        let ctx = open_sample();
        let layer = sample_base_layer(&ctx);
        let (no_mask, hidden) = dual_baseline(&ctx, layer, MASK_REGION, "e5");
        assert_differs("e5 前置: 隐藏全部基线必须不同于无蒙版基线", MASK_REGION, &no_mask, &hidden);

        let rev0 = ctx.rev();
        ctx.add(layer, "hide");
        let rev1 = ctx.rev();
        assert_eq!(rev1, rev0 + 1, "e5 add 字段 revision: 期望 {}, 实际 {rev1}", rev0 + 1);

        let rev2 = ctx.rev();
        let zero = ctx.set(layer, r#"{"density":0}"#);
        assert_bool(&zero["data"], "present", true, "e5 density=0 回读");
        assert_bool(&zero["data"], "enabled", true, "e5 density=0 回读");
        assert_num(&zero["data"], "density", 0.0, "e5 density=0 回读");
        assert_num(&zero["data"], "feather", 0.0, "e5 density=0 回读");
        let rev3 = ctx.rev();
        assert_eq!(rev3, rev2 + 1, "e5 density=0 字段 revision: 期望 {}, 实际 {rev3}", rev2 + 1);
        let px0 = ctx.render(MASK_REGION, "e5 density=0 后");
        let zero_diff = assert_same("e5 density=0 vs 无蒙版基线", MASK_REGION, &no_mask, &px0);

        let rev4 = ctx.rev();
        let full = ctx.set(layer, r#"{"density":1}"#);
        assert_num(&full["data"], "density", 1.0, "e5 density=1 回读");
        let rev5 = ctx.rev();
        assert_eq!(rev5, rev4 + 1, "e5 density=1 字段 revision: 期望 {}, 实际 {rev5}", rev4 + 1);
        let px1 = ctx.render(MASK_REGION, "e5 density=1 后");
        assert_same("e5 density=1 vs 隐藏层基线", MASK_REGION, &hidden, &px1);

        eprintln!(
            "e5 rev={rev0}->{rev5} mean={:.4} pct={:.4}% present={} density={:.3} feather={:.1} \
             density0_mean={:.4} density1_rev={rev5}",
            zero_diff.mean,
            zero_diff.pct,
            full["data"]["present"].as_bool().unwrap_or(false),
            number(&full["data"], "density", "e5 输出"),
            number(&full["data"], "feather", "e5 输出"),
            zero_diff.mean
        );
    }

    /// E6：`apply` 把（含羽化的）蒙版烘焙进 alpha 且画面等价（≤1 级量化）、`present=false`、可撤销；
    /// 停用后再 `apply` 等价于删除蒙版（R5.5）。
    #[test]
    fn e6_apply_bakes_alpha_and_drops_mask() {
        let ctx = open_sample();
        let layer = sample_base_layer(&ctx);
        let no_mask = ctx.render(EDGE_REGION, "e6 无蒙版基线");
        ctx.select_rect(SELECT_RECT);

        ctx.fill(layer, "hide");
        ctx.set(layer, r#"{"feather":8.0}"#);
        let before_apply = ctx.render(EDGE_REGION, "e6 apply 前");
        assert_differs("e6 前置: 羽化蒙版必须改变画面", EDGE_REGION, &no_mask, &before_apply);

        let rev0 = ctx.rev();
        let applied = ctx.apply(layer);
        assert_bool(&applied["data"], "present", false, "e6 apply 回读");
        let rev1 = ctx.rev();
        assert_eq!(rev1, rev0 + 1, "e6 apply 字段 revision: 期望 {}, 实际 {rev1}", rev0 + 1);
        let after_apply = ctx.render(EDGE_REGION, "e6 apply 后");
        let baked = assert_mean_at_most("e6 apply 前后画面等价（允许 1 级量化）", EDGE_REGION, &before_apply, &after_apply, |_, _| true, 1.0);
        let read = ctx.mask_data(layer, "e6 apply 后 get");
        assert_bool(&read, "present", false, "e6 apply 后 get");

        // ③ 撤销：蒙版回来，画面恢复（≤1 级量化）。
        let rev2 = ctx.rev();
        ctx.undo("e6 undo apply");
        let rev3 = ctx.rev();
        assert_eq!(rev3, rev2 + 1, "e6 undo 字段 revision: pc_undo 语义为 +1，期望 {}, 实际 {rev3}", rev2 + 1);
        let restored_state = ctx.mask_data(layer, "e6 undo 后 get");
        assert_bool(&restored_state, "present", true, "e6 undo 后 get");
        let restored = ctx.render(EDGE_REGION, "e6 undo 后");
        assert_mean_at_most("e6 undo 后画面恢复", EDGE_REGION, &before_apply, &restored, |_, _| true, 1.0);

        // ④ 停用后 apply 等价于删除：先清掉撤销回来的蒙版（否则 add 会报 already has a mask）。
        ctx.remove(layer);
        let removed = ctx.render(EDGE_REGION, "e6 撤销后删除蒙版");
        assert_same("e6 删除蒙版后画面 vs 无蒙版基线", EDGE_REGION, &no_mask, &removed);

        ctx.add(layer, "hide");
        ctx.set(layer, r#"{"enabled":false}"#);
        let disabled = ctx.render(EDGE_REGION, "e6 停用后 apply 前");
        assert_same("e6 停用蒙版画面 vs 无蒙版基线", EDGE_REGION, &no_mask, &disabled);
        let rev4 = ctx.rev();
        let applied2 = ctx.apply(layer);
        assert_bool(&applied2["data"], "present", false, "e6 停用后 apply 回读");
        let rev5 = ctx.rev();
        assert_eq!(rev5, rev4 + 1, "e6 停用后 apply 字段 revision: 期望 {}, 实际 {rev5}", rev4 + 1);
        let after_apply2 = ctx.render(EDGE_REGION, "e6 停用后 apply 后");
        let disabled_diff = assert_same("e6 停用后 apply vs 无蒙版基线", EDGE_REGION, &no_mask, &after_apply2);

        eprintln!(
            "e6 rev={rev0}->{rev5} mean={:.4} pct={:.4}% present={} density={:.3} feather={:.1} \
             baked_mean={:.4} baked_pct={:.4}% disabled_apply_mean={:.4}",
            disabled_diff.mean,
            disabled_diff.pct,
            applied2["data"]["present"].as_bool().unwrap_or(false),
            number(&applied2["data"], "density", "e6 输出"),
            number(&applied2["data"], "feather", "e6 输出"),
            baked.mean,
            baked.pct,
            disabled_diff.mean
        );
    }

    /// E7：`remove` 后画面完整可见（等于无蒙版基线），`undo` 后蒙版与画面一起恢复。
    #[test]
    fn e7_remove_and_undo() {
        let ctx = open_sample();
        let layer = sample_base_layer(&ctx);
        let (no_mask, hidden) = dual_baseline(&ctx, layer, MASK_REGION, "e7");

        let rev0 = ctx.rev();
        ctx.add(layer, "hide");
        let rev1 = ctx.rev();
        assert_eq!(rev1, rev0 + 1, "e7 add 字段 revision: 期望 {}, 实际 {rev1}", rev0 + 1);
        let masked = ctx.render(MASK_REGION, "e7 隐藏全部后");
        assert_same("e7 隐藏全部 vs 隐藏层基线", MASK_REGION, &hidden, &masked);

        let rev2 = ctx.rev();
        let removed_state = ctx.remove(layer);
        assert_bool(&removed_state["data"], "present", false, "e7 remove 回读");
        let rev3 = ctx.rev();
        assert_eq!(rev3, rev2 + 1, "e7 remove 字段 revision: 期望 {}, 实际 {rev3}", rev2 + 1);
        let read = ctx.mask_data(layer, "e7 remove 后 get");
        assert_bool(&read, "present", false, "e7 remove 后 get");
        let gone = ctx.render(MASK_REGION, "e7 删除后");
        let removed = assert_same("e7 删除后 vs 无蒙版基线", MASK_REGION, &no_mask, &gone);

        let rev4 = ctx.rev();
        ctx.undo("e7 undo remove");
        let rev5 = ctx.rev();
        assert_eq!(rev5, rev4 + 1, "e7 undo 字段 revision: pc_undo 语义为 +1，期望 {}, 实际 {rev5}", rev4 + 1);
        let restored = ctx.mask_data(layer, "e7 undo 后 get");
        assert_bool(&restored, "present", true, "e7 undo 后 get");
        assert_bool(&restored, "enabled", true, "e7 undo 后 get");
        let back = ctx.render(MASK_REGION, "e7 undo 后");
        assert_same("e7 undo 后 vs 隐藏层基线", MASK_REGION, &hidden, &back);

        eprintln!(
            "e7 rev={rev0}->{rev5} mean={:.4} pct={:.4}% present={} density={:.3} feather={:.1}",
            removed.mean,
            removed.pct,
            restored["present"].as_bool().unwrap_or(false),
            number(&restored, "density", "e7 输出"),
            number(&restored, "feather", "e7 输出")
        );
    }

    /// E8：逐步撤销恢复每个操作前的状态（`add` → `set(enabled=false)` → `remove`），
    /// 每个写操作恰好 1 个 undo 步（第四次 `undo` 必须报 `nothing to undo`）。
    #[test]
    fn e8_undo_stepwise_restores_state() {
        let ctx = open_sample();
        let layer = sample_base_layer(&ctx);
        let no_mask = ctx.render(MASK_REGION, "e8 无蒙版基线");
        let rev0 = ctx.rev();
        assert!(!ctx.can_undo(), "e8 前置: 新文档不应有 undo 步");

        ctx.add(layer, "hide");
        let r1 = ctx.rev();
        assert_eq!(r1, rev0 + 1, "e8 add 字段 revision: 期望 {}, 实际 {r1}", rev0 + 1);

        let disabled = ctx.set(layer, r#"{"enabled":false}"#);
        assert_bool(&disabled["data"], "enabled", false, "e8 set 回读");
        let r2 = ctx.rev();
        assert_eq!(r2, r1 + 1, "e8 set 字段 revision: 期望 {}, 实际 {r2}", r1 + 1);
        // R3.2：停用后的画面等于「无蒙版」基线。
        let off = ctx.render(MASK_REGION, "e8 停用后");
        assert_same("e8 停用后 vs 无蒙版基线", MASK_REGION, &no_mask, &off);

        let gone_state = ctx.remove(layer);
        assert_bool(&gone_state["data"], "present", false, "e8 remove 回读");
        let r3 = ctx.rev();
        assert_eq!(r3, r2 + 1, "e8 remove 字段 revision: 期望 {}, 实际 {r3}", r2 + 1);
        let gone = ctx.render(MASK_REGION, "e8 删除后");
        assert_same("e8 删除后 vs 无蒙版基线", MASK_REGION, &no_mask, &gone);

        // undo 1：回到「有蒙版且停用」。
        let u1 = ctx.rev();
        ctx.undo("e8 undo 1");
        let u1b = ctx.rev();
        assert_eq!(u1b, u1 + 1, "e8 第 1 次 undo 字段 revision: pc_undo 语义为 +1，期望 {}, 实际 {u1b}", u1 + 1);
        let first = ctx.mask_data(layer, "e8 undo 1 后");
        assert_bool(&first, "present", true, "e8 undo 1 后");
        assert_bool(&first, "enabled", false, "e8 undo 1 后");
        let px1 = ctx.render(MASK_REGION, "e8 undo 1 后");
        assert_same("e8 undo 1 后 vs 无蒙版基线（蒙版停用）", MASK_REGION, &no_mask, &px1);

        // undo 2：回到「有蒙版且启用」。
        let u2 = ctx.rev();
        ctx.undo("e8 undo 2");
        let u2b = ctx.rev();
        assert_eq!(u2b, u2 + 1, "e8 第 2 次 undo 字段 revision: pc_undo 语义为 +1，期望 {}, 实际 {u2b}", u2 + 1);
        let second = ctx.mask_data(layer, "e8 undo 2 后");
        assert_bool(&second, "present", true, "e8 undo 2 后");
        assert_bool(&second, "enabled", true, "e8 undo 2 后");
        let px2 = ctx.render(MASK_REGION, "e8 undo 2 后");
        let revived = assert_differs("e8 undo 2 后画面必须重新被蒙版遮住", MASK_REGION, &no_mask, &px2);

        // undo 3：回到「无蒙版」。
        let u3 = ctx.rev();
        ctx.undo("e8 undo 3");
        let u3b = ctx.rev();
        assert_eq!(u3b, u3 + 1, "e8 第 3 次 undo 字段 revision: pc_undo 语义为 +1，期望 {}, 实际 {u3b}", u3 + 1);
        let third = ctx.mask_data(layer, "e8 undo 3 后");
        assert_bool(&third, "present", false, "e8 undo 3 后");
        let px3 = ctx.render(MASK_REGION, "e8 undo 3 后");
        let restored = assert_same("e8 undo 3 后 vs 无蒙版基线", MASK_REGION, &no_mask, &px3);

        // 三个写操作 = 三个 undo 步：撤销栈必须恰好耗尽。
        assert!(!ctx.can_undo(), "e8 三次 undo 后 canUndo 应为 false（恰好 3 个 undo 步）");
        assert!(ctx.can_redo(), "e8 三次 undo 后 canRedo 应为 true");
        let extra = read_json(pc_undo(ctx.session, ctx.doc), "e8 第 4 次 undo");
        assert_err(&extra, "e8 第 4 次 undo", "nothing to undo");

        eprintln!("e8 undo 2 后重新被遮挡 mean={:.4} pct={:.4}%", revived.mean, revived.pct);
        eprintln!(
            "e8 rev={rev0}->{u3b} mean={:.4} pct={:.4}% present={} density={:.3} feather={:.1}",
            restored.mean,
            restored.pct,
            third["present"].as_bool().unwrap_or(false),
            number(&third, "density", "e8 输出"),
            number(&third, "feather", "e8 输出")
        );
    }

    /// E9：12 组非法调用全部 `ok=false`、错误文本逐字匹配（`C-MASK-API@1 §D`）、`revision` 不变。
    ///
    /// 说明：样本图没有组图层，第 12 组用 `pc_adjustment_add` 构造调整层（与「组/填充层」同属
    /// `mask_capable == false` 的不可挂蒙版目标），语义等价、未伪造结果。
    #[test]
    fn e9_invalid_params_leave_document_untouched() {
        let ctx = open_sample();
        let base = sample_base_layer(&ctx);
        let mut groups = 0usize;

        // 1) null session（get + add）
        let null = std::ptr::null_mut();
        let state = read_json(pc_layer_mask_get(null, ctx.doc, base), "e9 null session get");
        assert_err(&state, "e9 null session get", "null session");
        let state = read_json(pc_layer_mask_add(null, ctx.doc, base, b"reveal".as_ptr(), 6), "e9 null session add");
        assert_err(&state, "e9 null session add", "null session");
        groups += 1;

        // 2) document {id} is not open
        let missing = ctx.doc + 999;
        let rev = ctx.rev();
        let state = read_json(pc_layer_mask_get(ctx.session, missing, base), "e9 文档不存在 get");
        assert_err(&state, "e9 文档不存在 get", &format!("document {missing} is not open"));
        assert_rev_unchanged(&ctx, "e9 文档不存在 get", rev);
        groups += 1;

        // 3) layer not found
        let rev = ctx.rev();
        let state = read_json(pc_layer_mask_get(ctx.session, ctx.doc, u64::MAX), "e9 图层不存在 get");
        assert_err(&state, "e9 图层不存在 get", "layer not found");
        assert_rev_unchanged(&ctx, "e9 图层不存在 get", rev);
        groups += 1;

        // 4) fill must be reveal or hide
        let rev = ctx.rev();
        let state = ctx.add_raw(base, "foo");
        assert_err(&state, "e9 非法 fill", "fill must be reveal or hide");
        assert_rev_unchanged(&ctx, "e9 非法 fill", rev);
        groups += 1;

        // 5) no selection（先显式清空选区）
        ctx.select_none();
        let rev = ctx.rev();
        let state = ctx.fill_raw(base, "hide");
        assert_err(&state, "e9 无选区 fill", "no selection");
        assert_rev_unchanged(&ctx, "e9 无选区 fill", rev);
        groups += 1;

        // 6) mode must be reveal or hide（先建选区，确保失败原因不是 no selection）
        ctx.select_rect(SELECT_RECT);
        let rev = ctx.rev();
        let state = ctx.fill_raw(base, "foo");
        assert_err(&state, "e9 非法 mode", "mode must be reveal or hide");
        assert_rev_unchanged(&ctx, "e9 非法 mode", rev);
        groups += 1;

        // 7) layer already has a mask
        ctx.add(base, "reveal");
        let rev = ctx.rev();
        let state = ctx.add_raw(base, "hide");
        assert_err(&state, "e9 重复添加", "layer already has a mask");
        assert_rev_unchanged(&ctx, "e9 重复添加", rev);
        groups += 1;

        // 8) density 越界
        let rev = ctx.rev();
        let state = ctx.set_raw(base, r#"{"density":1.5}"#);
        assert_err(&state, "e9 density 1.5", "density must be a finite number within 0..1");
        assert_rev_unchanged(&ctx, "e9 density 1.5", rev);
        groups += 1;

        // 9) feather 越界
        let rev = ctx.rev();
        let state = ctx.set_raw(base, r#"{"feather":-1.0}"#);
        assert_err(&state, "e9 feather -1", "feather must be a finite number within 0..1000");
        assert_rev_unchanged(&ctx, "e9 feather -1", rev);
        groups += 1;

        // 10) layer has no mask：先成功删除，再让 set / apply / remove 全部失败
        ctx.remove(base);
        let rev = ctx.rev();
        let state = ctx.set_raw(base, "{}");
        assert_err(&state, "e9 无蒙版 set", "layer has no mask");
        assert_rev_unchanged(&ctx, "e9 无蒙版 set", rev);
        let state = ctx.apply_raw(base);
        assert_err(&state, "e9 无蒙版 apply", "layer has no mask");
        assert_rev_unchanged(&ctx, "e9 无蒙版 apply", rev);
        let state = ctx.remove_raw(base);
        assert_err(&state, "e9 无蒙版 remove", "layer has no mask");
        assert_rev_unchanged(&ctx, "e9 无蒙版 remove", rev);
        groups += 1;

        // 11) apply layer mask supports pixel layers only（形状层可挂蒙版、不可烘焙）
        let shape = ctx.create_shape(SHAPE_PARAMS);
        ctx.add(shape, "reveal");
        let rev = ctx.rev();
        let state = ctx.apply_raw(shape);
        assert_err(&state, "e9 形状层 apply", "apply layer mask supports pixel layers only");
        assert_rev_unchanged(&ctx, "e9 形状层 apply", rev);
        groups += 1;

        // 12) layer cannot take a pixel mask（调整层；样本图无组图层，语义等价）
        let adjustment = ctx.create_adjustment("brightnessContrast");
        let rev = ctx.rev();
        let state = ctx.add_raw(adjustment, "reveal");
        assert_err(&state, "e9 调整层 add", "layer cannot take a pixel mask");
        assert_rev_unchanged(&ctx, "e9 调整层 add", rev);
        groups += 1;

        eprintln!("e9 非法子用例={groups}（其中 get 调用 2 次），每次失败后 revision 均保持不变");
        eprintln!("e9 rev={rev}->{rev} mean=0.0000 pct=0.0000% present=false density=1.000 feather=0.0");
    }

    /// E10：连续 10 次 `pc_layer_mask_get` 不改 revision、不进 undo 栈、不改画面。
    #[test]
    fn e10_mask_get_is_read_only() {
        let ctx = open_sample();
        let layer = sample_base_layer(&ctx);
        let rev0 = ctx.rev();
        assert_eq!(rev0, 0, "e10 前置: 新文档 revision 应为 0, 实际 {rev0}");
        assert!(!ctx.can_undo(), "e10 前置: 新文档不应有 undo 步");
        let before = ctx.render(MASK_REGION, "e10 ten-get 前");

        for i in 0..10 {
            let state = ctx.mask_json(layer);
            let ctx_label = format!("e10 get 第 {i} 次");
            assert_ok(&state, &ctx_label);
            assert_bool(&state["data"], "present", false, &ctx_label);
            assert_bool(&state["data"], "enabled", true, &ctx_label);
            assert_bool(&state["data"], "linked", true, &ctx_label);
            assert_num(&state["data"], "density", 1.0, &ctx_label);
            assert_num(&state["data"], "feather", 0.0, &ctx_label);
        }
        let rev1 = ctx.rev();
        assert_eq!(rev1, rev0, "e10 十次 get 后 revision: 期望保持 {rev0}, 实际 {rev1}");
        assert!(!ctx.can_undo(), "e10 十次 get 后 canUndo 应为 false（get 不得建立 undo 步）");
        let after = ctx.render(MASK_REGION, "e10 ten-get 后");
        let diff = assert_same("e10 十次 get 后画面", MASK_REGION, &before, &after);

        // 反向证据：一次真实写操作必然进栈；撤销后回到空栈 ⇒ get 从未贡献 undo 步。
        ctx.add(layer, "reveal");
        assert!(ctx.can_undo(), "e10 add 后 canUndo 应为 true");
        ctx.undo("e10 undo add");
        assert!(!ctx.can_undo(), "e10 undo 后 canUndo 应回到 false（恰好 1 步）");
        assert!(ctx.can_redo(), "e10 undo 后 canRedo 应为 true");
        let present = ctx.mask_data(layer, "e10 收尾 get");
        assert_bool(&present, "present", false, "e10 收尾 get");

        eprintln!(
            "e10 rev={rev0}->{rev1} mean={:.4} pct={:.4}% present={} density={:.3} feather={:.1}",
            diff.mean,
            diff.pct,
            present["present"].as_bool().unwrap_or(false),
            number(&present, "density", "e10 输出"),
            number(&present, "feather", "e10 输出")
        );
    }

    // ---------------------------------------------------------------- E11（终验盲区补齐）

    /// E11 羽化过渡带边距：`feather=12` → σ=12，三次 box 卷积半径 11 ⇒ 支撑约 36 px，
    /// 内缩 48 px 可保证内核区的覆盖值是精确的 1.0 / 0.0。
    const FEATHER_MARGIN: i32 = 48;

    /// E11：选区内核（距选区边界 ≥ [`FEATHER_MARGIN`]），避开羽化过渡带。
    fn in_selection_core(x: i32, y: i32) -> bool {
        x >= SELECT_RECT.0 + FEATHER_MARGIN
            && x < SELECT_RECT.2 - FEATHER_MARGIN
            && y >= SELECT_RECT.1 + FEATHER_MARGIN
            && y < SELECT_RECT.3 - FEATHER_MARGIN
    }

    /// E11：选区外核（距选区边界 ≥ [`FEATHER_MARGIN`]），避开羽化过渡带。
    fn outside_selection_core(x: i32, y: i32) -> bool {
        !(x >= SELECT_RECT.0 - FEATHER_MARGIN
            && x < SELECT_RECT.2 + FEATHER_MARGIN
            && y >= SELECT_RECT.1 - FEATHER_MARGIN
            && y < SELECT_RECT.3 + FEATHER_MARGIN)
    }

    /// E11：**已有蒙版**时 `fill_from_selection` 只替换 `surface`，保留
    /// `enabled / linked / density / feather`（`C-MASK-API@1 §E.4`、`lib.rs:1975-1984`）。
    ///
    /// e1–e10 只覆盖「无蒙版 → fill」；本用例专门覆盖该盲区分支，这是 D1 漏检的同类路径。
    ///
    /// 像素期望说明（避免错误期望）：
    /// - `density=0.4` 时「隐藏」的有效覆盖值是 `1 - 0.4 = 0.6`（40% 透明），**不是完全不可见**；
    ///   因此不与「density=1 的隐藏层基线」比 0，而是与 fill **之前**的渲染
    ///   （hide + density 0.4 + feather 12，整幅覆盖值 0.0 ⇒ effective 0.6）比较。
    /// - `feather=12` 在选区边界产生过渡带（σ=12、三次 box 半径 11 ⇒ 支撑约 36 px），
    ///   所以两条像素断言都只取距边界 ≥ `FEATHER_MARGIN`(48 px) 的内核区。
    #[test]
    fn e11_fill_over_existing_mask_preserves_properties() {
        let ctx = open_sample();
        let layer = sample_base_layer(&ctx);
        let no_mask = ctx.render(EDGE_REGION, "e11 无蒙版基线");
        ctx.select_rect(SELECT_RECT);

        // 3：add("hide")，确认初值。
        let rev0 = ctx.rev();
        let added = ctx.add(layer, "hide");
        let data = &added["data"];
        assert_bool(data, "present", true, "e11 add 回读");
        assert_bool(data, "enabled", true, "e11 add 回读");
        assert_bool(data, "linked", true, "e11 add 回读");
        assert_num(data, "density", 1.0, "e11 add 回读");
        assert_num(data, "feather", 0.0, "e11 add 回读");
        let rev1 = ctx.rev();
        assert_eq!(rev1, rev0 + 1, "e11 add 字段 revision: 期望 {}, 实际 {rev1}", rev0 + 1);

        // 4：density=0.4 / feather=12。
        let rev2 = ctx.rev();
        let tuned = ctx.set(layer, r#"{"density":0.4,"feather":12}"#);
        let tuned_data = &tuned["data"];
        assert_bool(tuned_data, "present", true, "e11 set 回读");
        assert_bool(tuned_data, "enabled", true, "e11 set 回读");
        assert_bool(tuned_data, "linked", true, "e11 set 回读");
        assert_num(tuned_data, "density", 0.4, "e11 set 回读");
        assert_num(tuned_data, "feather", 12.0, "e11 set 回读");
        let rev3 = ctx.rev();
        assert_eq!(rev3, rev2 + 1, "e11 set 字段 revision: 期望 {}, 实际 {rev3}", rev2 + 1);

        // 「同 density/feather 的隐藏基线」：此时整幅蒙版覆盖值 0.0 ⇒ effective 0.6。
        let hidden_at_density = ctx.render(EDGE_REGION, "e11 同密度隐藏基线");
        assert_differs(
            "e11 前置: 0.4 密度的隐藏基线必须不同于无蒙版基线",
            EDGE_REGION,
            &no_mask,
            &hidden_at_density,
        );

        // 5（关键步）：已有蒙版 → fill("reveal")，只换 surface，保留其余属性。
        let rev4 = ctx.rev();
        let filled = ctx.fill(layer, "reveal");
        let filled_data = &filled["data"];
        assert_bool(filled_data, "present", true, "e11 fill 回读");
        assert_bool(filled_data, "enabled", true, "e11 fill 回读（不得被重置）");
        assert_bool(filled_data, "linked", true, "e11 fill 回读（不得被重置）");
        assert_num(filled_data, "density", 0.4, "e11 fill 回读（不得被重置为 1.0）");
        assert_num(filled_data, "feather", 12.0, "e11 fill 回读（不得被重置为 0.0）");
        let rev5 = ctx.rev();
        assert_eq!(rev5, rev4 + 1, "e11 fill 字段 revision（§E.4 总是写）: 期望 {}, 实际 {rev5}", rev4 + 1);

        // 读模型同样不得重置，且与写响应一致。
        let read = ctx.mask_data(layer, "e11 fill 后 get");
        assert_bool(&read, "present", true, "e11 fill 后 get");
        assert_bool(&read, "enabled", true, "e11 fill 后 get");
        assert_bool(&read, "linked", true, "e11 fill 后 get");
        assert_num(&read, "density", 0.4, "e11 fill 后 get（不得被重置为 1.0）");
        assert_num(&read, "feather", 12.0, "e11 fill 后 get（不得被重置为 0.0）");
        assert_eq!(&read, filled_data, "e11 fill 后 get 应与写响应一致, 实际 {read}");

        // fill 保留选区（R2.2 / §E.4）。
        let selection = ctx.selection_data("pc_selection_info（e11 fill 后）");
        assert_bool(&selection, "active", true, "e11 fill 后 selection");

        // 像素：选区内核 = 无蒙版基线；选区外核 = 同密度隐藏基线。
        let edge = ctx.render(EDGE_REGION, "e11 fill reveal 后");
        let inside = assert_mean_at_most("e11 选区内核 vs 无蒙版基线", EDGE_REGION, &no_mask, &edge, in_selection_core, 0.0);
        let outside = assert_mean_at_most(
            "e11 选区外核 vs 同密度隐藏基线",
            EDGE_REGION,
            &hidden_at_density,
            &edge,
            outside_selection_core,
            0.0,
        );

        // 反向证据：fill 必须让选区内核从 0.6 半透明变为完全可见，否则两条"相同"断言无意义。
        let activated = diff_rgba_region(&hidden_at_density, &edge, EDGE_REGION, in_selection_core);
        assert!(
            activated.mean > 0.0,
            "e11 反向证据: fill 后选区内核必须变为完全可见（0.6 → 1.0），实际 mean=0.0000（区域={EDGE_REGION:?}）；{}",
            describe_first(activated.first, EDGE_REGION)
        );

        eprintln!("e11 内核像素数 inside_px={} outside_px={}", inside.pixels, outside.pixels);
        eprintln!(
            "e11 rev={rev4}->{rev5} density={:.3} feather={:.1} present={} inside_mean={:.4} outside_mean={:.4}",
            number(filled_data, "density", "e11 输出"),
            number(filled_data, "feather", "e11 输出"),
            filled_data["present"].as_bool().unwrap_or(false),
            inside.mean,
            outside.mean
        );

    }

    /// 统计水平剖面上「半透明」像素数：`2 < alpha < 250` 视为过渡带。
    fn alpha_band(pixels: &[u8], region: (i32, i32, i32, i32), y: i32, x_start: i32, x_end: i32) -> usize {
        let width = region.2.max(1) as usize;
        let mut count = 0usize;
        for x in x_start..x_end {
            let index = ((y - region.1) as usize * width + (x - region.0) as usize) * 4 + 3;
            let alpha = pixels[index];
            if alpha > 2 && alpha < 250 {
                count += 1;
            }
        }
        count
    }

    /// E12：羽化必须在硬边处产生多像素过渡带（E11 只断言核心区，未直接采样过渡带）。
    #[test]
    fn e12_feather_produces_transition_band() {
        let ctx = open_sample();
        let layer = sample_base_layer(&ctx);
        ctx.select_rect(SELECT_RECT);

        let rev0 = ctx.rev();
        let _ = ctx.fill(layer, "hide");
        let rev1 = ctx.rev();
        assert_eq!(rev1, rev0 + 1, "e12 fill hide 字段 revision: 期望 {}, 实际 {rev1}", rev0 + 1);

        let hard = ctx.render(EDGE_REGION, "e12 硬边基线");
        let hard_band = alpha_band(&hard, EDGE_REGION, 200, 60, 380);
        assert!(hard_band <= 4, "e12 硬边过渡带不应超过 4px, 实际 {hard_band}");

        let rev2 = ctx.rev();
        let feathered = ctx.set(layer, r#"{"feather":12}"#);
        assert_num(&feathered["data"], "feather", 12.0, "e12 set feather");
        let rev3 = ctx.rev();
        assert_eq!(rev3, rev2 + 1, "e12 set feather 字段 revision: 期望 {}, 实际 {rev3}", rev2 + 1);

        let soft = ctx.render(EDGE_REGION, "e12 羽化后");
        let soft_band = alpha_band(&soft, EDGE_REGION, 200, 60, 380);
        assert!(soft_band >= 6, "e12 羽化后过渡带应至少 6px, 实际 {soft_band}");
        assert!(
            soft_band > hard_band + 4,
            "e12 羽化过渡带应明显宽于硬边: hard={hard_band} soft={soft_band}"
        );

        // 反向证据：羽化必须改变画面（否则两条「相同」断言无意义）。
        let diff = diff_rgba_region(&hard, &soft, EDGE_REGION, |_, _| true);
        assert!(
            diff.mean > 0.0 || diff.changed > 0,
            "e12 羽化必须改变画面, 实际 mean=0 changed=0（区域={EDGE_REGION:?}）"
        );

        let alpha_at = |pixels: &[u8], x: i32, y: i32| -> u8 {
            let width = EDGE_REGION.2 as usize;
            pixels[((y - EDGE_REGION.1) as usize * width + (x - EDGE_REGION.0) as usize) * 4 + 3]
        };
        assert!(alpha_at(&soft, 200, 200) <= 2, "e12 羽化后选区中心仍应隐藏, alpha={}", alpha_at(&soft, 200, 200));
        assert!(alpha_at(&soft, 60, 200) >= 250, "e12 羽化后选区外远端仍应可见, alpha={}", alpha_at(&soft, 60, 200));

        // 撤销：feather 回 0，画面逐字节回到硬边基线。
        let rev4 = ctx.rev();
        ctx.undo("e12 undo feather");
        let rev5 = ctx.rev();
        assert_eq!(rev5, rev4 + 1, "e12 undo 字段 revision: pc_undo 语义为 +1, 期望 {}, 实际 {rev5}", rev4 + 1);
        let restored = ctx.mask_data(layer, "e12 undo 后");
        assert_num(&restored, "feather", 0.0, "e12 undo 后 feather");
        let undone = ctx.render(EDGE_REGION, "e12 undo 后像素");
        assert_same("e12 undo 后应逐字节回到硬边基线", EDGE_REGION, &hard, &undone);

        eprintln!(
            "e12 hard_band={hard_band} soft_band={soft_band} mean={:.4} changed={} rev={rev0}->{rev3}",
            diff.mean, diff.changed
        );
    }

    /// E13：文字层可挂蒙版（`mask_capable`），但不可烘焙（`apply` 仅像素层）。
    #[test]
    fn e13_text_layer_mask_capable_but_not_bakeable() {
        let ctx = open_sample();
        let text = ctx.create_text(r#"{"text":"Hi","size":48,"x":200,"y":400,"color":[1,1,1,1]}"#);
        let rev0 = ctx.rev();
        let added = ctx.add(text, "hide");
        assert_bool(&added["data"], "present", true, "e13 文字层 add");
        assert_bool(&added["data"], "enabled", true, "e13 文字层 add");
        let rev1 = ctx.rev();
        assert_eq!(rev1, rev0 + 1, "e13 add 字段 revision: 期望 {}, 实际 {rev1}", rev0 + 1);

        let tuned = ctx.set(text, r#"{"density":0.5,"feather":4}"#);
        assert_num(&tuned["data"], "density", 0.5, "e13 文字层 set density");
        assert_num(&tuned["data"], "feather", 4.0, "e13 文字层 set feather");

        let rev = ctx.rev();
        let state = ctx.apply_raw(text);
        assert_err(&state, "e13 文字层 apply", "apply layer mask supports pixel layers only");
        assert_rev_unchanged(&ctx, "e13 文字层 apply", rev);

        let removed = ctx.remove(text);
        assert_bool(&removed["data"], "present", false, "e13 文字层 remove");
        eprintln!("e13 text add/set/remove ok, apply rejected; rev={rev0}->{rev1}");
    }

    /// E14：组与调整层不可挂像素蒙版（`mask_capable=false`）。
    #[test]
    fn e14_group_and_adjustment_reject_pixel_mask() {
        let ctx = open_sample();
        let base = sample_base_layer(&ctx);
        let adjustment = ctx.create_adjustment("brightnessContrast");
        let rev = ctx.rev();
        let state = ctx.add_raw(adjustment, "reveal");
        assert_err(&state, "e14 调整层 add", "layer cannot take a pixel mask");
        assert_rev_unchanged(&ctx, "e14 调整层 add", rev);

        let ids = format!("[{base}]");
        let grouped = read_json(
            pc_layer_group(ctx.session, ctx.doc, ids.as_bytes().as_ptr(), ids.len(), b"G".as_ptr(), 1),
            "e14 group",
        );
        assert_ok(&grouped, "e14 group");
        let group_id = grouped["data"]["groupId"]
            .as_u64()
            .unwrap_or_else(|| panic!("e14 group: 期望 data.groupId, 实际 {grouped}"));
        let rev = ctx.rev();
        let state = ctx.add_raw(group_id, "hide");
        assert_err(&state, "e14 组 add", "layer cannot take a pixel mask");
        assert_rev_unchanged(&ctx, "e14 组 add", rev);
        eprintln!("e14 adjustment/group reject pixel mask, group id={group_id}");
    }

    /// E15：羽化上限 1000 仍可用（不 panic、过渡带存在、撤销可回 0）。
    #[test]
    fn e15_feather_upper_bound_renders_and_undoes() {
        let ctx = open_sample();
        let layer = sample_base_layer(&ctx);
        ctx.select_rect(SELECT_RECT);
        let _ = ctx.fill(layer, "hide");

        let rev0 = ctx.rev();
        let sane = ctx.set(layer, r#"{"feather":25}"#);
        assert_num(&sane["data"], "feather", 25.0, "e15 feather=25");
        let rev1 = ctx.rev();
        assert_eq!(rev1, rev0 + 1, "e15 set feather=25 字段 revision: 期望 {}, 实际 {rev1}", rev0 + 1);
        let sane_px = ctx.render(EDGE_REGION, "e15 feather=25");
        let sane_band = alpha_band(&sane_px, EDGE_REGION, 200, 60, 380);
        assert!(sane_band > 0, "e15 feather=25 应存在半透明过渡带");

        let state = ctx.set(layer, r#"{"feather":1000}"#);
        assert_num(&state["data"], "feather", 1000.0, "e15 feather=1000");
        let rev2 = ctx.rev();
        assert_eq!(rev2, rev1 + 1, "e15 set feather=1000 字段 revision: 期望 {}, 实际 {rev2}", rev1 + 1);

        let started = std::time::Instant::now();
        let px = ctx.render(EDGE_REGION, "e15 feather=1000");
        let elapsed = started.elapsed();
        assert_eq!(
            px.len(),
            EDGE_REGION.2 as usize * EDGE_REGION.3 as usize * 4,
            "e15 feather=1000 渲染长度"
        );
        let band = alpha_band(&px, EDGE_REGION, 200, 60, 380);

        ctx.undo("e15 undo 1");
        let restored25 = ctx.mask_data(layer, "e15 undo1 后");
        assert_num(&restored25, "feather", 25.0, "e15 undo1 后 feather");
        ctx.undo("e15 undo 2");
        let restored0 = ctx.mask_data(layer, "e15 undo2 后");
        assert_num(&restored0, "feather", 0.0, "e15 undo2 后 feather");
        eprintln!(
            "e15 band25={sane_band} band1000={band} ms={:.1} rev={rev0}->{rev2}",
            elapsed.as_secs_f64() * 1000.0
        );
    }
}
/// 裁剪与画布尺寸的桥接级像素回归（计划 T4，契约 `C-CANVAS-TEST@1`）。
///
/// 全部断言经由公开的 `pc_*` C ABI；像素证据来自 `pc_render_pixels`（裸 RGBA8，无头部）。
/// 本模块自带全部 helper，不引用 `effects_api_tests` / `edit_api_tests` / `mask_api_tests`
/// 的私有项，也不修改任何生产函数。
#[cfg(test)]
mod canvas_api_tests {
    #![allow(clippy::panic, clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

    use super::*;

    /// 仓库自带 JPEG 样图（2400×1500），与既有测试同一路径。
    const SAMPLE_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../entry/src/main/resources/rawfile/sample.jpg");

    /// 取样 / 渲染区域 `(x, y, w, h)`：`w`/`h` 是区域尺寸，不是右上界。
    type Region = (i32, i32, i32, i32);

    /// 契约 e1 指定的裁剪矩形 `(x0, y0, x1, y1)`（`x1`/`y1` 为独占上界）。
    const CROP_RECT: Region = (100, 80, 420, 380);

    /// 契约 e1 的裁剪结果尺寸（= `x1 - x0` × `y1 - y0`）。
    const CROP_SIZE: (u32, u32) = (320, 300);

    /// 区域 A：`CROP_RECT` 的 `(x, y, w, h)` 形式，供 `pc_render_pixels` 取样。
    const SOURCE_REGION: Region = (CROP_RECT.0, CROP_RECT.1, CROP_SIZE.0 as i32, CROP_SIZE.1 as i32);

    /// e5 的取样点：取图像内部一块（远离边界，±2 平移不会越界）。
    const SAMPLE_POINT: Region = (1000, 800, 8, 8);

    // ---------------------------------------------------------------- 基础设施

    /// 读取并释放一个 `PcBuffer`。
    fn take_bytes(buf: *mut PcBuffer, ctx: &str) -> Vec<u8> {
        assert!(!buf.is_null(), "{ctx}: 期望非空 PcBuffer, 实际 null");
        // SAFETY: `buf` 由桥接层 `PcBuffer::from_vec` 分配；此处只读取一次并在随后释放一次。
        let bytes = unsafe {
            let b = &*buf;
            std::slice::from_raw_parts(b.ptr, b.len).to_vec()
        };
        pc_buffer_free(buf);
        bytes
    }

    /// 读取 JSON 响应并释放缓冲区。
    fn read_json(buf: *mut PcBuffer, ctx: &str) -> Value {
        let bytes = take_bytes(buf, ctx);
        match serde_json::from_slice::<Value>(&bytes) {
            Ok(value) => value,
            Err(e) => panic!("{ctx}: 期望合法 JSON, 实际解析失败 ({e}): {}", String::from_utf8_lossy(&bytes)),
        }
    }

    fn assert_ok(state: &Value, ctx: &str) {
        assert_eq!(state["ok"].as_bool(), Some(true), "{ctx}: 期望 ok=true, 实际 {state}");
    }

    /// 断言失败响应：`ok=false` 且 `error` 含契约给定的固定文案片段。
    fn assert_fail(state: &Value, ctx: &str, needle: &str) {
        assert_eq!(state["ok"].as_bool(), Some(false), "{ctx}: 期望 ok=false, 实际 {state}");
        let error = state["error"].as_str().unwrap_or("<缺失>");
        assert!(error.contains(needle), "{ctx}: 期望 error 包含 {needle:?}, 实际 {error:?}（完整响应 {state}）");
    }

    /// 读取必填的 `u32` 字段。
    fn u32_field(item: &Value, field: &str, ctx: &str) -> u32 {
        let raw = item[field]
            .as_u64()
            .unwrap_or_else(|| panic!("{ctx}: 字段 {field} 期望非负整数, 实际缺失（item={item}）"));
        u32::try_from(raw).unwrap_or_else(|_| panic!("{ctx}: 字段 {field}={raw} 超出 u32（item={item}）"))
    }

    /// 读取必填的 `i32` 字段。
    fn i32_field(item: &Value, field: &str, ctx: &str) -> i32 {
        let raw = item[field]
            .as_i64()
            .unwrap_or_else(|| panic!("{ctx}: 字段 {field} 期望整数, 实际缺失（item={item}）"));
        i32::try_from(raw).unwrap_or_else(|_| panic!("{ctx}: 字段 {field}={raw} 超出 i32（item={item}）"))
    }

    /// 读取样图字节（供多个文档复用，避免重复读盘）。
    fn sample_bytes(ctx: &str) -> Vec<u8> {
        std::fs::read(SAMPLE_PATH).unwrap_or_else(|e| panic!("{ctx}: 读取样图失败 {SAMPLE_PATH}: {e}"))
    }

    /// 逐字节比较两块裸 RGBA8；失败时报告长度、首个差异像素的绝对坐标与两侧 RGBA。
    fn assert_bytes_equal(expect: &[u8], actual: &[u8], region: Region, ctx: &str) {
        assert_eq!(
            actual.len(),
            expect.len(),
            "{ctx}: 字节长度不同：期望 {}，实际 {}（区域 {region:?}）",
            expect.len(),
            actual.len()
        );
        if let Some(index) = expect.iter().zip(actual.iter()).position(|(a, b)| a != b) {
            let pixel = index / 4;
            let (x, y) = (region.0 + (pixel as i32) % region.2, region.1 + (pixel as i32) / region.2);
            panic!(
                "{ctx}: 逐字节比较失败：区域 {region:?} 首个差异像素 (x={x}, y={y})，期望 RGBA={:?}，实际 RGBA={:?}",
                &expect[pixel * 4..pixel * 4 + 4],
                &actual[pixel * 4..pixel * 4 + 4]
            );
        }
    }

    /// 断言区域内每个像素的 alpha 都是 0（扩展区必须完全透明）。
    fn assert_alpha_zero(pixels: &[u8], region: Region, ctx: &str) {
        for (i, px) in pixels.chunks_exact(4).enumerate() {
            assert_eq!(px[3], 0, "{ctx}: 区域 {region:?} 第 {i} 个像素 alpha 期望 0，实际 {}（RGBA={px:?}）", px[3]);
        }
    }

    /// 从左侧起统计连续"整列 alpha 全为 0"的列数（验证锚点的扩展方向）。
    fn leading_transparent_cols(pixels: &[u8], region: Region) -> i32 {
        let (w, h) = (region.2, region.3);
        for col in 0..w {
            let all_zero = (0..h).all(|row| pixels[((row * w + col) * 4 + 3) as usize] == 0);
            if !all_zero {
                return col;
            }
        }
        w
    }

    /// 从上侧起统计连续"整行 alpha 全为 0"的行数。
    fn leading_transparent_rows(pixels: &[u8], region: Region) -> i32 {
        let (w, h) = (region.2, region.3);
        for row in 0..h {
            let all_zero = (0..w).all(|col| pixels[((row * w + col) * 4 + 3) as usize] == 0);
            if !all_zero {
                return row;
            }
        }
        h
    }

    /// 单轴 `CanvasAnchor::origin` 的测试侧参考实现（`align`：0=左/上，1=中，2=右/下）。
    fn expected_axis_origin(align: i32, old: u32, new: u32) -> i32 {
        let delta = old as i32 - new as i32;
        match align {
            0 => 0,
            1 => delta.div_euclid(2),
            _ => delta,
        }
    }

    /// 递归查找第一个非组图层 id（自带实现，不引用其他 mod 的 helper）。
    fn first_layer_id(layers: &Value) -> Option<u64> {
        let list = layers.as_array()?;
        for node in list {
            if node["isGroup"].as_bool() == Some(false)
                && let Some(id) = node["id"].as_u64()
            {
                return Some(id);
            }
        }
        for node in list {
            if let Some(id) = first_layer_id(&node["children"]) {
                return Some(id);
            }
        }
        None
    }

    // ---------------------------------------------------------------- 上下文

    /// 一个 session + 一个文档的测试上下文；`Drop` 负责释放 session。
    struct Ctx {
        session: *mut Session,
        doc: u64,
    }

    impl Drop for Ctx {
        fn drop(&mut self) {
            pc_session_free(self.session);
        }
    }

    impl Ctx {
        /// 打开仓库自带样图。
        fn open(ctx: &str) -> Ctx {
            let raw = sample_bytes(ctx);
            Self::open_bytes(&raw, ctx)
        }

        /// 用已读入内存的样图字节打开新文档（多个用例可复用同一份字节）。
        fn open_bytes(raw: &[u8], ctx: &str) -> Ctx {
            let session = pc_session_new();
            assert!(!session.is_null(), "{ctx}: 期望非空 session, 实际 null");
            let name = b"sample.jpg";
            let state_ctx = format!("{ctx} pc_open");
            let opened = read_json(pc_open(session, name.as_ptr(), name.len(), raw.as_ptr(), raw.len()), &state_ctx);
            assert_ok(&opened, &state_ctx);
            let doc = opened["data"]["id"]
                .as_u64()
                .unwrap_or_else(|| panic!("{ctx}: pc_open 未返回 data.id（{opened}）"));
            Ctx { session, doc }
        }

        fn rev(&self) -> u64 {
            let state = read_json(pc_revision(self.session, self.doc), "pc_revision");
            assert_ok(&state, "pc_revision");
            state["data"]["revision"]
                .as_u64()
                .unwrap_or_else(|| panic!("pc_revision: 缺少 data.revision（{state}）"))
        }

        fn info(&self) -> Value {
            let state = read_json(pc_info(self.session, self.doc), "pc_info");
            assert_ok(&state, "pc_info");
            state["data"].clone()
        }

        fn size(&self) -> (u32, u32) {
            let info = self.info();
            (u32_field(&info, "width", "pc_info"), u32_field(&info, "height", "pc_info"))
        }

        fn can_undo(&self) -> bool {
            let info = self.info();
            info["canUndo"].as_bool().unwrap_or_else(|| panic!("pc_info: 缺少 canUndo（{info}）"))
        }

        fn render(&self, region: Region, ctx: &str) -> Vec<u8> {
            let (x, y, w, h) = region;
            let state_ctx = format!("{ctx} pc_render_pixels{region:?}");
            let mut out: *mut PcBuffer = std::ptr::null_mut();
            let state = read_json(pc_render_pixels(self.session, self.doc, x, y, w, h, &mut out), &state_ctx);
            assert_ok(&state, &state_ctx);
            assert!(!out.is_null(), "{ctx}: pc_render_pixels 期望 out 非空, 实际 null");
            let pixels = take_bytes(out, ctx);
            let expected = (w as usize) * (h as usize) * 4;
            assert_eq!(
                pixels.len(),
                expected,
                "{ctx}: 区域 {region:?} 期望 {expected} 字节裸 RGBA（{} 像素 × 4），实际 {}",
                (w as usize) * (h as usize),
                pixels.len()
            );
            pixels
        }

        fn crop(&self, rect: Region, ctx: &str) -> Value {
            let (x0, y0, x1, y1) = rect;
            read_json(
                pc_document_crop(self.session, self.doc, x0, y0, x1, y1),
                &format!("{ctx} pc_document_crop{rect:?}"),
            )
        }

        fn resize(&self, width: i32, height: i32, anchor: i32, ctx: &str) -> Value {
            read_json(
                pc_document_resize_canvas(self.session, self.doc, width, height, anchor),
                &format!("{ctx} pc_document_resize_canvas({width}, {height}, anchor={anchor})"),
            )
        }

        fn undo(&self, ctx: &str) -> Value {
            read_json(pc_undo(self.session, self.doc), &format!("{ctx} pc_undo"))
        }

        fn redo(&self, ctx: &str) -> Value {
            read_json(pc_redo(self.session, self.doc), &format!("{ctx} pc_redo"))
        }

        fn select_rect(&self, rect: Region, ctx: &str) -> Value {
            let (x0, y0, x1, y1) = rect;
            let params = format!(r#"{{"kind":"rect","x0":{x0},"y0":{y0},"x1":{x1},"y1":{y1}}}"#);
            let bytes = params.as_bytes();
            read_json(pc_select(self.session, self.doc, bytes.as_ptr(), bytes.len()), &format!("{ctx} pc_select{params}"))
        }

        fn selection_info(&self, ctx: &str) -> Value {
            let state_ctx = format!("{ctx} pc_selection_info");
            let state = read_json(pc_selection_info(self.session, self.doc), &state_ctx);
            assert_ok(&state, &state_ctx);
            state["data"].clone()
        }

        fn first_layer(&self, ctx: &str) -> u64 {
            let info = self.info();
            first_layer_id(&info["layers"]).unwrap_or_else(|| panic!("{ctx}: 样图中没有非组图层（layers={}）", info["layers"]))
        }

        fn mask_add(&self, layer: u64, fill: &str, ctx: &str) -> Value {
            let bytes = fill.as_bytes();
            read_json(
                pc_layer_mask_add(self.session, self.doc, layer, bytes.as_ptr(), bytes.len()),
                &format!("{ctx} pc_layer_mask_add({fill})"),
            )
        }

        fn mask_fill_from_selection(&self, layer: u64, mode: &str, ctx: &str) -> Value {
            let bytes = mode.as_bytes();
            read_json(
                pc_layer_mask_fill_from_selection(self.session, self.doc, layer, bytes.as_ptr(), bytes.len()),
                &format!("{ctx} pc_layer_mask_fill_from_selection({mode})"),
            )
        }

        fn mask_get(&self, layer: u64, ctx: &str) -> Value {
            let state_ctx = format!("{ctx} pc_layer_mask_get");
            let state = read_json(pc_layer_mask_get(self.session, self.doc, layer), &state_ctx);
            assert_ok(&state, &state_ctx);
            state["data"].clone()
        }
    }

    // ---------------------------------------------------------------- 用例 e1–e8

    /// e1 / R3.1 / R3.2：裁剪后新尺寸内容与裁剪前同一区域逐字节相等（证明不是简单清零）。
    #[test]
    fn e1_crop_shrinks_canvas_and_matches_source_region() {
        let c = Ctx::open("e1");
        let (src_w, src_h) = c.size();
        assert!(
            src_w > CROP_RECT.2 as u32 && src_h > CROP_RECT.3 as u32,
            "e1: 样图 {src_w}x{src_h} 必须大于裁剪矩形 {CROP_RECT:?}"
        );
        let rev0 = c.rev();
        let expect = c.render(SOURCE_REGION, "e1 裁剪前区域 A");
        let state = c.crop(CROP_RECT, "e1");
        assert_ok(&state, "e1 crop");
        assert_eq!(
            (u32_field(&state["data"], "width", "e1"), u32_field(&state["data"], "height", "e1")),
            CROP_SIZE,
            "e1: crop 响应尺寸应为 {CROP_SIZE:?}（响应 {state}）"
        );
        let rev1 = c.rev();
        assert_eq!(rev1, rev0 + 1, "e1: 裁剪成功应恰好一次 checkpoint（revision {rev0}->{rev1}）");
        let actual = c.render((0, 0, CROP_SIZE.0 as i32, CROP_SIZE.1 as i32), "e1 裁剪后");
        assert_bytes_equal(
            &expect,
            &actual,
            (0, 0, CROP_SIZE.0 as i32, CROP_SIZE.1 as i32),
            "e1: 裁剪后内容必须与裁剪前区域 A（裁剪前的同一取样区域）逐字节相等",
        );
        assert_eq!(c.size(), CROP_SIZE, "e1: pc_info 尺寸应为 {CROP_SIZE:?}");
        eprintln!(
            "c1 crop {}x{} px_equal=true rev={rev0}->{rev1} src={src_w}x{src_h}",
            CROP_SIZE.0, CROP_SIZE.1
        );
    }

    /// e2 / R4.1–R4.3：撤销恢复原尺寸与原像素，重做回到裁剪结果。
    #[test]
    fn e2_crop_undo_redo_restores_exactly() {
        let c = Ctx::open("e2");
        let (src_w, src_h) = c.size();
        let expect = c.render(SOURCE_REGION, "e2 裁剪前区域 A");
        let rev0 = c.rev();

        let cropped_state = c.crop(CROP_RECT, "e2 crop");
        assert_ok(&cropped_state, "e2 crop");
        let cropped = c.render((0, 0, CROP_SIZE.0 as i32, CROP_SIZE.1 as i32), "e2 裁剪后");
        let rev1 = c.rev();
        assert_eq!(c.size(), CROP_SIZE, "e2: 裁剪后尺寸应为 {CROP_SIZE:?}");

        let undone = c.undo("e2");
        assert_ok(&undone, "e2 undo");
        assert_eq!(c.size(), (src_w, src_h), "e2: undo 后应恢复原尺寸 {src_w}x{src_h}");
        let rev2 = c.rev();
        assert!(rev2 > rev1, "e2: undo 应递增 revision（{rev1}->{rev2}）");
        let restored = c.render(SOURCE_REGION, "e2 undo 后区域 A");
        assert_bytes_equal(&expect, &restored, SOURCE_REGION, "e2: undo 后区域 A 必须逐字节复原");

        let redone = c.redo("e2");
        assert_ok(&redone, "e2 redo");
        assert_eq!(c.size(), CROP_SIZE, "e2: redo 后应回到裁剪尺寸");
        let rev3 = c.rev();
        assert!(rev3 > rev2, "e2: redo 应递增 revision（{rev2}->{rev3}）");
        let again = c.render((0, 0, CROP_SIZE.0 as i32, CROP_SIZE.1 as i32), "e2 redo 后");
        assert_bytes_equal(
            &cropped,
            &again,
            (0, 0, CROP_SIZE.0 as i32, CROP_SIZE.1 as i32),
            "e2: redo 后内容应与裁剪结果逐字节一致",
        );
        eprintln!("c2 undo_redo size={src_w}x{src_h} px_equal=true rev={rev0}->{rev1}->{rev2}->{rev3}");
    }

    /// e3 / R2.1–R2.4：中心锚点扩边后新增区域 alpha 全为 0，原内容整体平移 (+32, +16)。
    #[test]
    fn e3_resize_pads_new_area_with_transparent_pixels() {
        let c = Ctx::open("e3");
        let (src_w, src_h) = c.size();
        let (pad_w, pad_h) = (64u32, 32u32);
        let (half_w, half_h) = ((pad_w / 2) as i32, (pad_h / 2) as i32);
        let expect = c.render((0, 0, 64, 64), "e3 原内容");
        let rev0 = c.rev();

        let state = c.resize((src_w + pad_w) as i32, (src_h + pad_h) as i32, 4, "e3 中心锚点");
        assert_ok(&state, "e3 resize");
        let new_w = src_w + pad_w;
        let new_h = src_h + pad_h;
        assert_eq!(
            (u32_field(&state["data"], "width", "e3"), u32_field(&state["data"], "height", "e3")),
            (new_w, new_h),
            "e3: resize 响应尺寸应为 {new_w}x{new_h}（响应 {state}）"
        );
        let rev1 = c.rev();
        assert_eq!(rev1, rev0 + 1, "e3: resize 成功应恰好一次 checkpoint（revision {rev0}->{rev1}）");

        let moved = c.render((half_w, half_h, 64, 64), "e3 偏移后的原内容");
        assert_bytes_equal(
            &expect,
            &moved,
            (half_w, half_h, 64, 64),
            "e3: 中心锚点下原内容应整体平移 (+32, +16)",
        );

        let left = (0, 0, half_w, new_h as i32);
        let right = (new_w as i32 - half_w, 0, half_w, new_h as i32);
        let top = (half_w, 0, src_w as i32, half_h);
        let bottom = (half_w, src_h as i32 + half_h, src_w as i32, half_h);
        assert_alpha_zero(&c.render(left, "e3 左侧扩展带"), left, "e3: 左侧新增区域必须全透明");
        assert_alpha_zero(&c.render(right, "e3 右侧扩展带"), right, "e3: 右侧新增区域必须全透明");
        assert_alpha_zero(&c.render(top, "e3 上方扩展带"), top, "e3: 上方新增区域必须全透明");
        assert_alpha_zero(&c.render(bottom, "e3 下方扩展带"), bottom, "e3: 下方新增区域必须全透明");

        eprintln!(
            "c3 resize {src_w}x{src_h}->{new_w}x{new_h} pad_alpha_zero=true offset={half_w},{half_h} rev={rev0}->{rev1}"
        );
    }

    /// e4 / R2.2：缩小画布时右锚点（index=5）保留右侧，左上锚点（index=0）`origin == (0,0)`。
    #[test]
    fn e4_resize_shrinks_and_honours_anchor() {
        let raw = sample_bytes("e4");
        let (src_w, src_h) = Ctx::open_bytes(&raw, "e4 尺寸").size();
        let (new_w, new_h) = (src_w - 200, src_h - 100);
        assert!(new_w > 64 && new_h > 64, "e4: 样图 {src_w}x{src_h} 必须足够大");

        // 右锚点（row=1, col=2）：横向保留右侧，纵向居中。
        let right = Ctx::open_bytes(&raw, "e4 右锚点");
        let origin = (src_w as i32 - new_w as i32, (src_h as i32 - new_h as i32).div_euclid(2));
        let expect = right.render((origin.0, origin.1, 64, 64), "e4 右锚点期望来源");
        let rev0 = right.rev();
        let state = right.resize(new_w as i32, new_h as i32, 5, "e4 右锚点");
        assert_ok(&state, "e4 右锚点 resize");
        assert_eq!(right.size(), (new_w, new_h), "e4: 右锚点 resize 后尺寸应为 {new_w}x{new_h}");
        let rev1 = right.rev();
        assert_eq!(rev1, rev0 + 1, "e4: 右锚点 resize 应恰好一次 checkpoint（{rev0}->{rev1}）");
        let got = right.render((0, 0, 64, 64), "e4 右锚点结果");
        assert_bytes_equal(
            &expect,
            &got,
            (0, 0, 64, 64),
            "e4: 右锚点（index=5）下新 (0,0) 必须来自原 origin 位置",
        );

        // 左上锚点（index=0）：origin == (0,0)，新 (0,0) 仍是原 (0,0)。
        let top_left = Ctx::open_bytes(&raw, "e4 左上锚点");
        let expect_tl = top_left.render((0, 0, 64, 64), "e4 左上锚点期望");
        let tl_rev0 = top_left.rev();
        let tl_state = top_left.resize(new_w as i32, new_h as i32, 0, "e4 左上锚点");
        assert_ok(&tl_state, "e4 左上锚点 resize");
        assert_eq!(top_left.size(), (new_w, new_h), "e4: 左上锚点 resize 后尺寸应为 {new_w}x{new_h}");
        let got_tl = top_left.render((0, 0, 64, 64), "e4 左上锚点结果");
        assert_bytes_equal(
            &expect_tl,
            &got_tl,
            (0, 0, 64, 64),
            "e4: 左上锚点（index=0）origin=(0,0)，新 (0,0) 必须仍是原 (0,0)",
        );

        eprintln!(
            "c4 right_anchor {src_w}x{src_h}->{new_w}x{new_h} origin=({},{}) px_equal=true rev={rev0}->{rev1}",
            origin.0, origin.1
        );
        eprintln!(
            "c4 left_anchor {src_w}x{src_h}->{new_w}x{new_h} origin=(0,0) px_equal=true rev={tl_rev0}->{}",
            top_left.rev()
        );
    }

    /// e5 / R2.3：9 个锚点逐一验证"原取样点内容"的新坐标等于 `origin()` 计算值，且扩展方向正确。
    #[test]
    fn e5_resize_anchor_grid_is_consistent() {
        let raw = sample_bytes("e5");
        let base = Ctx::open_bytes(&raw, "e5 基线");
        let (src_w, src_h) = base.size();
        let expect = base.render(SAMPLE_POINT, "e5 基线取样");
        let mut lines = Vec::new();

        for index in 0..9 {
            let ctx = format!("e5 anchor={index}");
            let c = Ctx::open_bytes(&raw, &ctx);
            let rev0 = c.rev();
            let state = c.resize(src_w as i32 + 2, src_h as i32 + 2, index, &ctx);
            assert_ok(&state, &format!("{ctx} resize"));
            assert_eq!(c.size(), (src_w + 2, src_h + 2), "{ctx}: 尺寸应为 {}x{}", src_w + 2, src_h + 2);
            let rev1 = c.rev();
            assert_eq!(rev1, rev0 + 1, "{ctx}: 应恰好一次 checkpoint（{rev0}->{rev1}）");

            let (col, row) = (index % 3, index / 3);
            let nx0 = expected_axis_origin(col, src_w, src_w + 2);
            let ny0 = expected_axis_origin(row, src_h, src_h + 2);
            let at = (SAMPLE_POINT.0 - nx0, SAMPLE_POINT.1 - ny0, SAMPLE_POINT.2, SAMPLE_POINT.3);
            let got = c.render(at, &format!("{ctx} 取样点"));
            assert_bytes_equal(
                &expect,
                &got,
                at,
                &format!("{ctx}: 原取样点内容应位于 ({}, {})", SAMPLE_POINT.0 - nx0, SAMPLE_POINT.1 - ny0),
            );

            let x_strip = (0, SAMPLE_POINT.1, 4, 4);
            let cols = leading_transparent_cols(&c.render(x_strip, &format!("{ctx} 左前缀")), x_strip);
            assert_eq!(cols, -nx0, "{ctx}: 左侧透明列数应等于 -nx0={}", -nx0);

            let y_strip = (SAMPLE_POINT.0, 0, 4, 4);
            let rows = leading_transparent_rows(&c.render(y_strip, &format!("{ctx} 上前缀")), y_strip);
            assert_eq!(rows, -ny0, "{ctx}: 顶部透明行数应等于 -ny0={}", -ny0);

            lines.push(format!(
                "c5 anchor={index} origin=({nx0},{ny0}) offset_ok=true prefix=({cols},{rows}) rev={rev0}->{rev1}"
            ));
        }

        for line in &lines {
            eprintln!("{line}");
        }
    }

    /// e6 / R6.1–R6.3 / R6.5：非法参数 `ok=false` + 固定错误片段，且 `revision`/尺寸/`canUndo` 全部不变。
    #[test]
    fn e6_invalid_arguments_leave_revision_and_size_unchanged() {
        let c = Ctx::open("e6");
        let (src_w, src_h) = c.size();
        // 先建立一个合法撤销点，使 canUndo=true —— 这样"canUndo 不变"才是可观察的断言。
        let seed = c.select_rect(CROP_RECT, "e6 建立撤销点");
        assert_ok(&seed, "e6 建立撤销点");
        assert!(c.can_undo(), "e6: 前置条件：建立撤销点后 canUndo 必须为 true");
        let rev_before = c.rev();
        assert!(rev_before > 0, "e6: 前置条件：建立撤销点后 revision 必须 > 0");
        let mut checked = 0usize;

        for (label, rect, needle) in [
            ("crop 空矩形 (0,0,0,0)", (0, 0, 0, 0), "crop rectangle is outside the document"),
            ("crop 完全在画布外", (5000, 5000, 6000, 6000), "crop rectangle is outside the document"),
            ("crop 负坐标在画布外", (-200, -200, -100, -100), "crop rectangle is outside the document"),
            ("crop 零高度", (0, 0, src_w as i32, 0), "crop rectangle is outside the document"),
        ] {
            let state = c.crop(rect, &format!("e6 {label}"));
            assert_fail(&state, &format!("e6 {label}"), needle);
            assert_eq!(c.rev(), rev_before, "e6 {label}: revision 必须不变");
            assert_eq!(c.size(), (src_w, src_h), "e6 {label}: pc_info 尺寸必须不变");
            assert!(c.can_undo(), "e6 {label}: canUndo 必须不变");
            checked += 1;
        }

        for (label, width, height, anchor, needle) in [
            ("resize width=0", 0, src_h as i32, 4, "invalid canvas size"),
            ("resize height=-1", src_w as i32, -1, 4, "invalid canvas size"),
            ("resize width=16385", 16385, src_h as i32, 4, "invalid canvas size"),
            ("resize 16384x16384 超总像素", 16384, 16384, 4, "canvas is too large"),
            ("resize anchor=9", src_w as i32, src_h as i32, 9, "invalid anchor"),
            ("resize anchor=-1", src_w as i32, src_h as i32, -1, "invalid anchor"),
        ] {
            let state = c.resize(width, height, anchor, &format!("e6 {label}"));
            assert_fail(&state, &format!("e6 {label}"), needle);
            assert_eq!(c.rev(), rev_before, "e6 {label}: revision 必须不变");
            assert_eq!(c.size(), (src_w, src_h), "e6 {label}: pc_info 尺寸必须不变");
            assert!(c.can_undo(), "e6 {label}: canUndo 必须不变");
            checked += 1;
        }

        assert_eq!(checked, 10, "e6: 应覆盖 10 个非法参数组合");
        eprintln!(
            "c6 invalid_cases={checked} rev={rev_before}->{} size={src_w}x{src_h} unchanged=true",
            c.rev()
        );
    }

    /// e7 / R6.3：无变化的 crop / resize 返回 `ok=true` 但不 checkpoint（revision 与 canUndo 不变）。
    #[test]
    fn e7_noop_crop_or_resize_does_not_checkpoint() {
        let c = Ctx::open("e7");
        let (src_w, src_h) = c.size();
        let seed = c.select_rect(CROP_RECT, "e7 建立撤销点");
        assert_ok(&seed, "e7 建立撤销点");
        let rev0 = c.rev();
        assert!(rev0 > 0 && c.can_undo(), "e7: 前置条件：应有可撤销步骤");

        let noop_crop = c.crop((0, 0, src_w as i32, src_h as i32), "e7 no-op crop");
        assert_ok(&noop_crop, "e7: 与当前 bounds 相同的 crop 应返回 ok=true");
        assert_eq!(
            (u32_field(&noop_crop["data"], "width", "e7"), u32_field(&noop_crop["data"], "height", "e7")),
            (src_w, src_h),
            "e7: no-op crop 应返回当前尺寸（响应 {noop_crop}）"
        );
        assert_eq!(c.rev(), rev0, "e7: no-op crop 不得 checkpoint（revision 必须保持 {rev0}）");
        assert!(c.can_undo(), "e7: no-op crop 不得改变 canUndo");

        let noop_resize = c.resize(src_w as i32, src_h as i32, 4, "e7 no-op resize");
        assert_ok(&noop_resize, "e7: 与当前尺寸相同的 resize 应返回 ok=true");
        assert_eq!(
            (u32_field(&noop_resize["data"], "width", "e7"), u32_field(&noop_resize["data"], "height", "e7")),
            (src_w, src_h),
            "e7: no-op resize 应返回当前尺寸（响应 {noop_resize}）"
        );
        assert_eq!(c.rev(), rev0, "e7: no-op resize 不得 checkpoint（revision 必须保持 {rev0}）");
        assert!(c.can_undo(), "e7: no-op resize 不得改变 canUndo");

        eprintln!("c7 noop_crop=true noop_resize=true rev={rev0}->{} canUndo=true size={src_w}x{src_h}", c.rev());
    }

    /// e8 / R3.3 / R1.5：裁剪同时平移选区与图层蒙版，蒙版层像素逐字节不变（未错位）。
    #[test]
    fn e8_crop_moves_selection_and_layer_mask() {
        let c = Ctx::open("e8");
        let layer = c.first_layer("e8");

        let selected = c.select_rect(CROP_RECT, "e8 选区");
        assert_ok(&selected, "e8 选区");
        let before = c.selection_info("e8 选区后");
        assert_eq!(before["active"].as_bool(), Some(true), "e8: 选区应为 active（{before}）");
        assert_eq!(
            (
                i32_field(&before, "x0", "e8 选区"),
                i32_field(&before, "y0", "e8 选区"),
                i32_field(&before, "x1", "e8 选区"),
                i32_field(&before, "y1", "e8 选区"),
            ),
            CROP_RECT,
            "e8: 选区 bounds 应为 {CROP_RECT:?}（{before}）"
        );

        // 蒙版：先加全白蒙版，再用选区覆盖成"仅选区内可见"的形状 —— 若蒙版不随画布平移，
        // 裁剪后原选区之外的蒙版值（0）会落到新画布上，像素必然与原区域不同。
        assert_ok(&c.mask_add(layer, "reveal", "e8 mask add"), "e8 mask add");
        assert_ok(
            &c.mask_fill_from_selection(layer, "reveal", "e8 mask fill"),
            "e8 mask_fill_from_selection",
        );
        let mask_before = c.mask_get(layer, "e8 mask get");
        assert_eq!(mask_before["present"].as_bool(), Some(true), "e8: 蒙版必须存在（{mask_before}）");

        let expect = c.render(SOURCE_REGION, "e8 裁剪前蒙版层");
        let rev0 = c.rev();
        let cropped = c.crop(CROP_RECT, "e8 crop");
        assert_ok(&cropped, "e8 crop");
        assert_eq!(c.size(), CROP_SIZE, "e8: 裁剪后尺寸应为 {CROP_SIZE:?}");
        let rev1 = c.rev();
        assert_eq!(rev1, rev0 + 1, "e8: 裁剪应恰好一次 checkpoint（{rev0}->{rev1}）");

        let after = c.selection_info("e8 裁剪后选区");
        assert_eq!(after["active"].as_bool(), Some(true), "e8: 裁剪后选区应仍存在（{after}）");
        assert_eq!(
            (
                i32_field(&after, "x0", "e8 裁剪后选区"),
                i32_field(&after, "y0", "e8 裁剪后选区"),
                i32_field(&after, "x1", "e8 裁剪后选区"),
                i32_field(&after, "y1", "e8 裁剪后选区"),
            ),
            (0, 0, CROP_SIZE.0 as i32, CROP_SIZE.1 as i32),
            "e8: 选区应随画布平移为 (0,0)-({},{})，实际 {after}",
            CROP_SIZE.0,
            CROP_SIZE.1
        );

        let mask_after = c.mask_get(layer, "e8 裁剪后 mask get");
        assert_eq!(mask_after["present"].as_bool(), Some(true), "e8: 裁剪后蒙版必须仍存在（{mask_after}）");

        let got = c.render((0, 0, CROP_SIZE.0 as i32, CROP_SIZE.1 as i32), "e8 裁剪后蒙版层");
        assert_bytes_equal(
            &expect,
            &got,
            (0, 0, CROP_SIZE.0 as i32, CROP_SIZE.1 as i32),
            "e8: 蒙版层裁剪后必须与裁剪前同区域逐字节相等（蒙版随画布平移未错位）",
        );

        eprintln!(
            "c8 selection=(0,0,{},{}) mask_present=true px_equal=true rev={rev0}->{rev1}",
            CROP_SIZE.0, CROP_SIZE.1
        );
    }

    /// e9 / R4.1–R4.3：`resize_canvas` 的撤销必须恢复原尺寸与原像素。
    ///
    /// 与 e2 的 crop 撤销同构（共用 `checkpoint_snapshot` 修复路径），因此这里同时覆盖
    /// **放大（中心锚点 4）** 与 **缩小（左上锚点 0）** 两个方向，并校验 `pc_undo` 使
    /// `revision` **+1**（不是 −1）。
    #[test]
    fn e9_resize_undo_restores_size_and_pixels() {
        let c = Ctx::open("e9");
        let (src_w, src_h) = c.size();
        assert_eq!((src_w, src_h), (2400, 1500), "e9: 样图尺寸应为 2400x1500");
        let sample: Region = (64, 64, 64, 64);
        let baseline = c.render(sample, "e9 基线 A");
        let rev0 = c.rev();

        // ---- 放大：中心锚点 4 ----
        let grown = c.resize(2500, 1600, 4, "e9 放大");
        assert_ok(&grown, "e9 放大 resize");
        assert_eq!(
            (u32_field(&grown["data"], "width", "e9"), u32_field(&grown["data"], "height", "e9")),
            (2500, 1600),
            "e9: 放大后响应尺寸应为 2500x1600（响应 {grown}）"
        );
        assert_eq!(c.size(), (2500, 1600), "e9: 放大后 pc_info 尺寸应为 2500x1600");
        let rev_grown = c.rev();
        assert_eq!(rev_grown, rev0 + 1, "e9: 放大 resize 应恰好一次 checkpoint（{rev0}->{rev_grown}）");

        let undone = c.undo("e9 放大 undo");
        assert_ok(&undone, "e9 放大 undo");
        assert_eq!(c.size(), (src_w, src_h), "e9: 放大 undo 后应恢复 {src_w}x{src_h}");
        let rev_undo_grown = c.rev();
        assert_eq!(
            rev_undo_grown,
            rev_grown + 1,
            "e9: pc_undo 应使 revision +1（{rev_grown}->{rev_undo_grown}），而不是 −1"
        );
        let restored = c.render(sample, "e9 放大 undo 后");
        assert_bytes_equal(&baseline, &restored, sample, "e9: 放大 undo 后取样区域必须与基线 A 逐字节相等");

        // ---- 缩小：左上锚点 0（覆盖反向尺寸变化）----
        let shrunk = c.resize(2200, 1400, 0, "e9 缩小");
        assert_ok(&shrunk, "e9 缩小 resize");
        assert_eq!(
            (u32_field(&shrunk["data"], "width", "e9"), u32_field(&shrunk["data"], "height", "e9")),
            (2200, 1400),
            "e9: 缩小后响应尺寸应为 2200x1400（响应 {shrunk}）"
        );
        assert_eq!(c.size(), (2200, 1400), "e9: 缩小后 pc_info 尺寸应为 2200x1400");
        let rev_shrunk = c.rev();
        assert_eq!(
            rev_shrunk,
            rev_undo_grown + 1,
            "e9: 缩小 resize 应恰好一次 checkpoint（{rev_undo_grown}->{rev_shrunk}）"
        );

        let undone = c.undo("e9 缩小 undo");
        assert_ok(&undone, "e9 缩小 undo");
        assert_eq!(c.size(), (src_w, src_h), "e9: 缩小 undo 后应恢复 {src_w}x{src_h}");
        let rev_undo_shrunk = c.rev();
        assert_eq!(
            rev_undo_shrunk,
            rev_shrunk + 1,
            "e9: 缩小 pc_undo 应使 revision +1（{rev_shrunk}->{rev_undo_shrunk}）"
        );
        let restored = c.render(sample, "e9 缩小 undo 后");
        assert_bytes_equal(&baseline, &restored, sample, "e9: 缩小 undo 后取样区域必须与基线 A 逐字节相等");

        eprintln!("e9 resize_undo size_restored=true px_equal=true rev={rev0}->{rev_grown}->{rev_undo_grown}");
        eprintln!("e9 shrink_undo size_restored=true px_equal=true rev={rev_undo_grown}->{rev_shrunk}->{rev_undo_shrunk}");
    }

    // ---------------------------------------------------------------- 导出尺寸（e10）

    /// 调 `pc_export` 导出 PNG，返回编码后的文件字节。
    ///
    /// `pc_export` 的实际签名是 `(session, id, format_ptr, format_len, quality, out)`：
    /// `format` 是**字符串**（`"png"`/`"jpeg"/...`，不是数字），`out` 槽内是完整编码文件
    /// （无 8 字节尺寸头，PNG 自带 IHDR）。
    fn export_png_bytes(c: &Ctx, ctx: &str) -> Vec<u8> {
        let format = b"png";
        let state_ctx = format!("{ctx} pc_export(png)");
        let mut out: *mut PcBuffer = std::ptr::null_mut();
        let state = read_json(
            pc_export(c.session, c.doc, format.as_ptr(), format.len(), 20, &mut out),
            &state_ctx,
        );
        assert_ok(&state, &state_ctx);
        assert!(!out.is_null(), "{ctx}: pc_export 期望 out 非空, 实际 null");
        let bytes = take_bytes(out, ctx);
        assert!(!bytes.is_empty(), "{ctx}: 导出字节为空");
        bytes
    }

    /// 解析 PNG IHDR 的宽高（PNG 头布局固定，无需引入解码依赖）。
    fn png_size(bytes: &[u8], ctx: &str) -> (u32, u32) {
        const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        assert!(bytes.len() >= 24, "{ctx}: PNG 字节过短（{} < 24）", bytes.len());
        assert_eq!(&bytes[0..8], SIGNATURE.as_slice(), "{ctx}: PNG 签名不符");
        assert_eq!(
            &bytes[12..16],
            b"IHDR".as_slice(),
            "{ctx}: 首个 chunk 应为 IHDR，实际 {:?}",
            &bytes[12..16]
        );
        let width = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
        let height = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
        (width, height)
    }

    /// 导出 → 回读 IHDR 尺寸 → 与期望比较。
    fn assert_export_size(c: &Ctx, expected: (u32, u32), ctx: &str) {
        let bytes = export_png_bytes(c, ctx);
        let actual = png_size(&bytes, ctx);
        assert_eq!(
            actual, expected,
            "{ctx}: 导出图像尺寸期望 {expected:?}, 实际 {actual:?}（PNG 共 {} 字节）",
            bytes.len()
        );
    }

    /// e10 / 导出按当前画布尺寸：`pc_export` 走 `photocraft_io::export(&entry.doc, ...)`，
    /// 因此裁剪、撤销、画布尺寸调整后，PNG 产物回读的宽高必须等于当前 `pc_info` 尺寸。
    #[test]
    fn e10_export_uses_current_canvas_size() {
        let c = Ctx::open("e10");
        let (src_w, src_h) = c.size();
        assert_eq!((src_w, src_h), (2400, 1500), "e10: 样图尺寸应为 2400x1500");

        // 1) 原图：导出尺寸 = 当前画布尺寸
        assert_export_size(&c, (src_w, src_h), "e10 原图导出");

        // 2) 裁剪到 320x300 → 导出必须跟着变小
        assert_ok(&c.crop(CROP_RECT, "e10 crop"), "e10 crop");
        assert_eq!(c.size(), CROP_SIZE, "e10: 裁剪后尺寸应为 {CROP_SIZE:?}");
        assert_export_size(&c, CROP_SIZE, "e10 裁剪后导出");

        // 3) undo 恢复原尺寸 → 导出必须跟着恢复（证明导出读的是当前文档，不是旧缓存）
        assert_ok(&c.undo("e10 undo"), "e10 undo");
        assert_eq!(c.size(), (src_w, src_h), "e10: undo 后尺寸应恢复 {src_w}x{src_h}");
        assert_export_size(&c, (src_w, src_h), "e10 undo 后导出");

        // 4) 画布尺寸调整到 2600x1700 → 导出必须跟着变大
        assert_ok(&c.resize(2600, 1700, 4, "e10 resize"), "e10 resize");
        assert_eq!(c.size(), (2600, 1700), "e10: resize 后尺寸应为 2600x1700");
        assert_export_size(&c, (2600, 1700), "e10 resize 后导出");

        eprintln!("e10 export 2400x1500 -> 320x300 -> 2400x1500 -> 2600x1700 ok=true");
    }
}
/// 图层自由变换（`C-XFORM-API@1`）像素级回归（计划 X3）。
///
/// 全部用例走真实导出的 C ABI（`pc_document_new` / `pc_layer_transform` / `pc_document_turn` /
/// `pc_render_pixels` / `pc_layer_mask_*` / `pc_select` / `pc_filter_apply` / `pc_undo` /
/// `pc_revision` / `pc_info`）。模块自带全部 helper，不引用其他 `#[cfg(test)] mod` 的私有项。
#[cfg(test)]
mod transform_api_tests {
    //! X3：缩放 / 旋转 90° 与 `pc_document_turn` 等价性 / 蒙版同步 / 撤销恢复 /
    //! 非法参数零副作用 / 全默认值不 checkpoint。
    #![allow(clippy::panic, clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

    use super::*;

    /// 像素区域 `(x, y, w, h)`：`w`/`h` 是尺寸，不是右上界。
    type Region = (i32, i32, i32, i32);
    /// 包围盒 `(x0, y0, x1, y1)`：`x1`/`y1` 为独占上界。
    type Bbox = (i32, i32, i32, i32);

    const DOC_W: u32 = 200;
    const DOC_H: u32 = 100;

    // ---------------------------------------------------------------- 基础 helper

    /// 读取并释放一个 `PcBuffer`。
    fn take_bytes(buf: *mut PcBuffer, ctx: &str) -> Vec<u8> {
        assert!(!buf.is_null(), "{ctx}: 期望非空 PcBuffer, 实际 null");
        // SAFETY: `buf` 由桥接层 `PcBuffer::from_vec` 分配；此处只读取一次并在随后释放一次。
        let bytes = unsafe {
            let b = &*buf;
            std::slice::from_raw_parts(b.ptr, b.len).to_vec()
        };
        pc_buffer_free(buf);
        bytes
    }

    /// 读取 JSON 响应并释放缓冲区。
    fn read_json(buf: *mut PcBuffer, ctx: &str) -> Value {
        let bytes = take_bytes(buf, ctx);
        match serde_json::from_slice::<Value>(&bytes) {
            Ok(value) => value,
            Err(e) => panic!("{ctx}: 期望合法 JSON, 实际解析失败 ({e}): {}", String::from_utf8_lossy(&bytes)),
        }
    }

    fn assert_ok(state: &Value, ctx: &str) {
        assert_eq!(state["ok"].as_bool(), Some(true), "{ctx}: 期望 ok=true, 实际 {state}");
    }

    /// 断言失败响应：`ok=false` 且 `error` 含契约给定的固定文案片段。
    fn assert_fail(state: &Value, ctx: &str, needle: &str) {
        assert_eq!(state["ok"].as_bool(), Some(false), "{ctx}: 期望 ok=false, 实际 {state}");
        let error = state["error"].as_str().unwrap_or("<缺失>");
        assert!(error.contains(needle), "{ctx}: 期望 error 包含 {needle:?}, 实际 {error:?}（完整响应 {state}）");
    }

    fn u32_field(item: &Value, field: &str, ctx: &str) -> u32 {
        let raw = item[field]
            .as_u64()
            .unwrap_or_else(|| panic!("{ctx}: 字段 {field} 期望非负整数, 实际缺失（item={item}）"));
        u32::try_from(raw).unwrap_or_else(|_| panic!("{ctx}: 字段 {field}={raw} 超出 u32（item={item}）"))
    }

    /// 读取 `data.bounds`（`[x0, y0, x1, y1]`）。
    fn bounds_field(state: &Value, ctx: &str) -> Bbox {
        let list = state["data"]["bounds"]
            .as_array()
            .unwrap_or_else(|| panic!("{ctx}: 期望 data.bounds 数组, 实际 {state}"));
        assert_eq!(list.len(), 4, "{ctx}: data.bounds 应为 4 项, 实际 {list:?}");
        let axis = |i: usize| {
            let raw = list[i].as_i64().unwrap_or_else(|| panic!("{ctx}: bounds[{i}] 期望整数（{state}）"));
            i32::try_from(raw).unwrap_or_else(|_| panic!("{ctx}: bounds[{i}]={raw} 超出 i32"))
        };
        (axis(0), axis(1), axis(2), axis(3))
    }

    /// 扫描像素满足 `pred` 的范围；无命中返回 `None`。
    fn bbox_by(pixels: &[u8], w: i32, h: i32, pred: impl Fn(&[u8]) -> bool) -> Option<Bbox> {
        let mut found: Option<Bbox> = None;
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                if !pred(&pixels[i..i + 4]) {
                    continue;
                }
                found = Some(match found {
                    None => (x, y, x + 1, y + 1),
                    Some(b) => (b.0.min(x), b.1.min(y), b.2.max(x + 1), b.3.max(y + 1)),
                });
            }
        }
        found
    }

    /// 可见内容包围盒：alpha > 0 且不是纯白（白底图层上的黑色/灰色内容）。
    ///
    /// 只看“非白”会误把透明像素（RGB=0）算成内容，因此必须同时要求 alpha > 0。
    fn bbox_content(pixels: &[u8], w: i32, h: i32) -> Option<Bbox> {
        bbox_by(pixels, w, h, |px| px[3] > 0 && !(px[0] > 250 && px[1] > 250 && px[2] > 250))
    }

    /// 不透明度包围盒：alpha > 0（蒙版用例里内容本身是白的）。
    fn bbox_alpha(pixels: &[u8], w: i32, h: i32) -> Option<Bbox> {
        bbox_by(pixels, w, h, |px| px[3] > 0)
    }

    fn mean_abs_diff(a: &[u8], b: &[u8]) -> f64 {
        assert_eq!(a.len(), b.len(), "mean_abs_diff: 长度不同 {} vs {}", a.len(), b.len());
        if a.is_empty() {
            return 0.0;
        }
        a.iter().zip(b.iter()).map(|(x, y)| f64::from(x.abs_diff(*y))).sum::<f64>() / a.len() as f64
    }

    /// 像素等价判据：优先逐字节相等；不相等时允许平均绝对差 ≤ 1（90° 旋转的双线性权重理论上是
    /// 0/1，唯一差异来源是源包围盒边缘与默认透明像素的混合，故容差 1 只覆盖边界外的量化误差）。
    /// 返回实测平均绝对差，调用方打印出来，避免无声放宽。
    fn assert_pixels_equivalent(expect: &[u8], actual: &[u8], ctx: &str) -> f64 {
        assert_eq!(expect.len(), actual.len(), "{ctx}: 字节长度不同：期望 {}，实际 {}", expect.len(), actual.len());
        if expect == actual {
            return 0.0;
        }
        let diff = mean_abs_diff(expect, actual);
        assert!(diff <= 1.0, "{ctx}: 逐字节比较失败且平均绝对差 {diff:.4} 超过 1");
        diff
    }

    fn assert_bytes_equal(expect: &[u8], actual: &[u8], ctx: &str) {
        assert_eq!(expect.len(), actual.len(), "{ctx}: 字节长度不同：期望 {}，实际 {}", expect.len(), actual.len());
        if let Some(index) = expect.iter().zip(actual.iter()).position(|(a, b)| a != b) {
            let pixel = index / 4;
            panic!(
                "{ctx}: 逐字节比较失败：第 {pixel} 个像素（字节 {index}）期望 {:?}，实际 {:?}",
                &expect[pixel * 4..pixel * 4 + 4],
                &actual[pixel * 4..pixel * 4 + 4]
            );
        }
    }

    /// 递归查找第一个非组图层 id（自带实现，不引用其他 mod 的 helper）。
    fn first_layer_id(layers: &Value) -> Option<u64> {
        let list = layers.as_array()?;
        for node in list {
            if node["isGroup"].as_bool() == Some(false)
                && let Some(id) = node["id"].as_u64()
            {
                return Some(id);
            }
        }
        for node in list {
            if let Some(id) = first_layer_id(&node["children"]) {
                return Some(id);
            }
        }
        None
    }

    // ---------------------------------------------------------------- 上下文

    /// 一个 session + 一个文档的测试上下文；`Drop` 负责释放 session。
    struct Ctx {
        session: *mut Session,
        doc: u64,
    }

    impl Drop for Ctx {
        fn drop(&mut self) {
            pc_session_free(self.session);
        }
    }

    impl Ctx {
        /// 新建 `w × h` 文档：`pc_document_new` 会自动压入一层不透明的白色像素层。
        fn white(w: u32, h: u32, ctx: &str) -> Ctx {
            let session = pc_session_new();
            assert!(!session.is_null(), "{ctx}: 期望非空 session, 实际 null");
            let state_ctx = format!("{ctx} pc_document_new({w}x{h})");
            let state = read_json(pc_document_new(session, w, h, std::ptr::null::<u8>(), 0), &state_ctx);
            assert_ok(&state, &state_ctx);
            let doc = state["data"]["id"]
                .as_u64()
                .unwrap_or_else(|| panic!("{ctx}: pc_document_new 未返回 data.id（{state}）"));
            Ctx { session, doc }
        }

        fn info(&self) -> Value {
            let state = read_json(pc_info(self.session, self.doc), "pc_info");
            assert_ok(&state, "pc_info");
            state["data"].clone()
        }

        fn rev(&self) -> u64 {
            let state = read_json(pc_revision(self.session, self.doc), "pc_revision");
            assert_ok(&state, "pc_revision");
            state["data"]["revision"]
                .as_u64()
                .unwrap_or_else(|| panic!("pc_revision: 缺少 data.revision（{state}）"))
        }

        fn size(&self) -> (u32, u32) {
            let info = self.info();
            (u32_field(&info, "width", "pc_info"), u32_field(&info, "height", "pc_info"))
        }

        fn can_undo(&self) -> bool {
            let info = self.info();
            info["canUndo"].as_bool().unwrap_or_else(|| panic!("pc_info: 缺少 canUndo（{info}）"))
        }

        /// 文档里第一个非组图层（`pc_document_new` 的白色背景层）。
        fn layer(&self, ctx: &str) -> u64 {
            let info = self.info();
            first_layer_id(&info["layers"]).unwrap_or_else(|| panic!("{ctx}: 文档没有非组图层（layers={}）", info["layers"]))
        }

        /// 渲染 `region`（1:1 裸 RGBA8），并校验字节数 = `w * h * 4`。
        fn render(&self, region: Region, ctx: &str) -> Vec<u8> {
            let (x, y, w, h) = region;
            let state_ctx = format!("{ctx} pc_render_pixels{region:?}");
            let mut out: *mut PcBuffer = std::ptr::null_mut();
            let state = read_json(pc_render_pixels(self.session, self.doc, x, y, w, h, &mut out), &state_ctx);
            assert_ok(&state, &state_ctx);
            assert!(!out.is_null(), "{ctx}: pc_render_pixels 期望 out 非空, 实际 null");
            let pixels = take_bytes(out, ctx);
            let expected = (w as usize) * (h as usize) * 4;
            assert_eq!(pixels.len(), expected, "{ctx}: 区域 {region:?} 期望 {expected} 字节裸 RGBA, 实际 {}", pixels.len());
            pixels
        }

        fn transform(&self, layer: u64, params: &str, ctx: &str) -> Value {
            let bytes = params.as_bytes();
            read_json(
                pc_layer_transform(self.session, self.doc, layer, bytes.as_ptr(), bytes.len()),
                &format!("{ctx} pc_layer_transform({params})"),
            )
        }

        fn filter(&self, layer: u64, params: &str, ctx: &str) -> Value {
            let bytes = params.as_bytes();
            read_json(
                pc_filter_apply(self.session, self.doc, layer, bytes.as_ptr(), bytes.len()),
                &format!("{ctx} pc_filter_apply({params})"),
            )
        }

        /// 用矩形造内容：先选区、再反相（白 → 黑），选区外像素保持不变。
        fn black_rect(&self, rect: Region, layer: u64, ctx: &str) {
            assert_ok(&self.select_rect(rect, ctx), &format!("{ctx} pc_select"));
            assert_ok(&self.filter(layer, r#"{"filter":"invert"}"#, &format!("{ctx} invert")), &format!("{ctx} pc_filter_apply"));
        }

        fn select_rect(&self, rect: Region, ctx: &str) -> Value {
            let (x0, y0, x1, y1) = rect;
            let params = format!(r#"{{"kind":"rect","x0":{x0},"y0":{y0},"x1":{x1},"y1":{y1}}}"#);
            let bytes = params.as_bytes();
            read_json(
                pc_select(self.session, self.doc, bytes.as_ptr(), bytes.len()),
                &format!("{ctx} pc_select{params}"),
            )
        }

        fn mask_add(&self, layer: u64, fill: &str, ctx: &str) -> Value {
            let bytes = fill.as_bytes();
            read_json(
                pc_layer_mask_add(self.session, self.doc, layer, bytes.as_ptr(), bytes.len()),
                &format!("{ctx} pc_layer_mask_add({fill})"),
            )
        }

        fn mask_fill_from_selection(&self, layer: u64, mode: &str, ctx: &str) -> Value {
            let bytes = mode.as_bytes();
            read_json(
                pc_layer_mask_fill_from_selection(self.session, self.doc, layer, bytes.as_ptr(), bytes.len()),
                &format!("{ctx} pc_layer_mask_fill_from_selection({mode})"),
            )
        }

        fn mask_get(&self, layer: u64, ctx: &str) -> Value {
            let state_ctx = format!("{ctx} pc_layer_mask_get");
            let state = read_json(pc_layer_mask_get(self.session, self.doc, layer), &state_ctx);
            assert_ok(&state, &state_ctx);
            state["data"].clone()
        }

        /// `pc_document_turn`：3 = 顺时针 90°。
        fn turn(&self, code: u32, ctx: &str) -> Value {
            read_json(pc_document_turn(self.session, self.doc, code), &format!("{ctx} pc_document_turn({code})"))
        }

        fn undo(&self, ctx: &str) -> Value {
            read_json(pc_undo(self.session, self.doc), &format!("{ctx} pc_undo"))
        }

        /// 新增调整层，返回其 layerId（用于“调整层不可变换”分支）。
        fn adjustment_add(&self, kind: &str, ctx: &str) -> u64 {
            let kind_bytes = kind.as_bytes();
            let params = b"{}";
            let state = read_json(
                pc_adjustment_add(self.session, self.doc, kind_bytes.as_ptr(), kind_bytes.len(), params.as_ptr(), params.len()),
                &format!("{ctx} pc_adjustment_add({kind})"),
            );
            assert_ok(&state, &format!("{ctx} pc_adjustment_add({kind})"));
            state["data"]["layerId"]
                .as_u64()
                .unwrap_or_else(|| panic!("{ctx}: pc_adjustment_add 未返回 data.layerId（{state}）"))
        }
    }

    // ---------------------------------------------------------------- 用例 e1–e6

    /// e1：`scaleX/Y = 0.5` 把内容缩到一半，`bounds` 与渲染出的内容包围盒同步缩小，
    /// 且成功恰好一次 checkpoint。
    #[test]
    fn e1_scale_halves_content_and_bumps_revision_once() {
        let c = Ctx::white(DOC_W, DOC_H, "e1");
        let layer = c.layer("e1");
        // 白底上反相造出整层黑色内容，这样“非白像素范围”才有意义。
        assert_ok(&c.filter(layer, r#"{"filter":"invert"}"#, "e1 造内容"), "e1 pc_filter_apply");
        let rev0 = c.rev();

        let state = c.transform(layer, r#"{"scaleX":0.5,"scaleY":0.5}"#, "e1");
        assert_ok(&state, "e1 transform");
        let bounds = bounds_field(&state, "e1");
        eprintln!("e1 transform resp={state}");
        assert_eq!(bounds, (50, 25, 150, 75), "e1: scale 0.5 的 bounds 应为内容包围盒 (0,0,200,100) 减半居中");
        let rev1 = c.rev();
        assert_eq!(rev1, rev0 + 1, "e1: 成功应恰好一次 checkpoint（revision {rev0}->{rev1}）");

        let pixels = c.render((0, 0, DOC_W as i32, DOC_H as i32), "e1 变换后");
        let bb = bbox_content(&pixels, DOC_W as i32, DOC_H as i32)
            .unwrap_or_else(|| panic!("e1: 变换后渲染应仍有非白内容"));
        let (w, h) = (bb.2 - bb.0, bb.3 - bb.1);
        // 单次双线性的源包围盒边缘会与默认透明像素混合，±2 覆盖 1px 的边界量化。
        assert!((w - 100).abs() <= 2, "e1: 内容宽应约为 100（原 200 减半）, 实测 {w}（bbox={bb:?}）");
        assert!((h - 50).abs() <= 2, "e1: 内容高应约为 50（原 100 减半）, 实测 {h}（bbox={bb:?}）");
        let (cx, cy) = (bb.0 + w / 2, bb.1 + h / 2);
        assert!((cx - 100).abs() <= 1 && (cy - 50).abs() <= 1, "e1: 内容中心应保持 (100,50), 实测 ({cx},{cy})（bbox={bb:?}）");
        eprintln!("e1 scale=0.5 bounds=[{}, {}, {}, {}] rev={rev0}->{rev1} content={}x{}", bounds.0, bounds.1, bounds.2, bounds.3, w, h);
    }

    /// e2：图层 `{"angle":90}` 与整画布 `pc_document_turn(3)` 的内容逐像素等价。
    ///
    /// 两条路径产物画布尺寸不同（图层旋转保持 200×100，turn 变 100×200），因此只比较内容块：
    /// 先在画布中央造 100×40 的黑色块（`[50,30,150,70]`），旋转 90° 后它是 40×100 的竖块。
    ///
    /// 判据：逐字节相等优先；不相等时要求平均绝对差 ≤ 1。放宽理由：90° 且旋转中心落在
    /// 整数半格时双线性权重应为 0/1，理论上逐字节一致；唯一可能差异是源包围盒边缘像素与
    /// 默认透明像素的混合（turn 是纯像素重映射，不插值），故只接受 ≤1 的边界量化误差，
    /// 且实测值会打印出来，不做无声放宽。
    #[test]
    fn e2_layer_rotate_matches_canvas_turn_content() {
        let region = (50, 30, 150, 70);

        // 路径 A：图层旋转 90°（画布仍 200×100）
        let a = Ctx::white(DOC_W, DOC_H, "e2 A");
        let a_layer = a.layer("e2 A");
        a.black_rect(region, a_layer, "e2 A");
        let a_rev0 = a.rev();
        let a_state = a.transform(a_layer, r#"{"angle":90}"#, "e2 A");
        assert_ok(&a_state, "e2 A transform");
        let a_rev1 = a.rev();
        assert_eq!(a_rev1, a_rev0 + 1, "e2 A: 成功应恰好一次 checkpoint（{a_rev0}->{a_rev1}）");
        let a_pixels = a.render((0, 0, DOC_W as i32, DOC_H as i32), "e2 A 旋转后");
        let a_bb = bbox_content(&a_pixels, DOC_W as i32, DOC_H as i32)
            .unwrap_or_else(|| panic!("e2 A: 旋转后渲染应仍有非白内容"));
        let a_size = (a_bb.2 - a_bb.0, a_bb.3 - a_bb.1);
        assert!(
            (a_size.0 - 40).abs() <= 1 && (a_size.1 - 100).abs() <= 1,
            "e2 A: 内容块旋转 90° 后应为 40x100, 实测 {a_size:?}（bbox={a_bb:?}）"
        );

        // 路径 B：整画布顺时针 90°（画布变 100×200）
        let b = Ctx::white(DOC_W, DOC_H, "e2 B");
        let b_layer = b.layer("e2 B");
        b.black_rect(region, b_layer, "e2 B");
        assert_ok(&b.turn(3, "e2 B"), "e2 B turn");
        assert_eq!(b.size(), (DOC_H, DOC_W), "e2 B: 整画布 90° 后尺寸应为 100x200");
        let b_pixels = b.render((0, 0, DOC_H as i32, DOC_W as i32), "e2 B 旋转后");
        let b_bb = bbox_content(&b_pixels, DOC_H as i32, DOC_W as i32)
            .unwrap_or_else(|| panic!("e2 B: 旋转后渲染应仍有非白内容"));
        let b_size = (b_bb.2 - b_bb.0, b_bb.3 - b_bb.1);
        assert!(
            (b_size.0 - 40).abs() <= 1 && (b_size.1 - 100).abs() <= 1,
            "e2 B: 内容块旋转 90° 后应为 40x100, 实测 {b_size:?}（bbox={b_bb:?}）"
        );
        assert!(
            (a_size.0 - b_size.0).abs() <= 1 && (a_size.1 - b_size.1).abs() <= 1,
            "e2: 两条路径内容包围盒尺寸应一致（图层 {a_size:?} vs 画布 {b_size:?}）"
        );

        // 只比较重叠部分，尺寸差 ≤1px 时按较小一边对齐。
        let (w, h) = (a_size.0.min(b_size.0), a_size.1.min(b_size.1));
        let a_content = a.render((a_bb.0, a_bb.1, w, h), "e2 A 内容区域");
        let b_content = b.render((b_bb.0, b_bb.1, w, h), "e2 B 内容区域");
        let diff = assert_pixels_equivalent(&a_content, &b_content, "e2 图层旋转 vs 画布 turn 的内容");
        eprintln!(
            "e2 angle=90 a_bbox={a_bb:?} b_bbox={b_bb:?} size={w}x{h} byte_equal={} avg_diff={diff:.4}",
            diff == 0.0
        );
    }

    /// e3：图层旋转时蒙版同步旋转——可见区域跟着内容一起转，且 `present` 保持 true。
    ///
    /// 判据：变换后渲染的可见区域（alpha>0）应为旋转后的 40×100 竖块 `[80,0,120,100]`，
    /// 并与“新文档 + 同一旋转后矩形 reveal 蒙版”的渲染等价（逐字节优先，否则平均绝对差 ≤1，
    /// 理由同 e2：90° 双线性权重 0/1，只有包围盒边缘存在量化差）。
    #[test]
    fn e3_mask_follows_layer_rotation() {
        let c = Ctx::white(DOC_W, DOC_H, "e3");
        let layer = c.layer("e3");
        assert_ok(&c.mask_add(layer, "hide", "e3"), "e3 mask add(hide)");
        assert_ok(&c.select_rect((50, 30, 150, 70), "e3"), "e3 pc_select");
        assert_ok(&c.mask_fill_from_selection(layer, "reveal", "e3"), "e3 mask fill(reveal)");
        let before = c.mask_get(layer, "e3");
        assert_eq!(before["present"].as_bool(), Some(true), "e3: 变换前蒙版应 present=true（{before}）");

        let rev0 = c.rev();
        let state = c.transform(layer, r#"{"angle":90}"#, "e3");
        assert_ok(&state, "e3 transform");
        let rev1 = c.rev();
        assert_eq!(rev1, rev0 + 1, "e3: 成功应恰好一次 checkpoint（{rev0}->{rev1}）");

        let after_mask = c.mask_get(layer, "e3");
        assert_eq!(after_mask["present"].as_bool(), Some(true), "e3: 变换后蒙版仍应 present=true（{after_mask}）");

        let after = c.render((0, 0, DOC_W as i32, DOC_H as i32), "e3 变换后");
        let bb = bbox_alpha(&after, DOC_W as i32, DOC_H as i32)
            .unwrap_or_else(|| panic!("e3: 变换后应仍有可见区域"));
        let (w, h) = (bb.2 - bb.0, bb.3 - bb.1);
        assert!(
            (bb.0 - 80).abs() <= 1 && (bb.1 - 0).abs() <= 1,
            "e3: 可见区域左上角应约为 (80,0), 实测 {bb:?}"
        );
        assert!(
            (w - 40).abs() <= 1 && (h - 100).abs() <= 1,
            "e3: 可见区域尺寸应为 40x100, 实测 {w}x{h}（bbox={bb:?}）"
        );

        // 参考：不做图层变换，直接把蒙版做成旋转后的矩形。
        let r = Ctx::white(DOC_W, DOC_H, "e3 参考");
        let r_layer = r.layer("e3 参考");
        assert_ok(&r.mask_add(r_layer, "hide", "e3 参考"), "e3 参考 mask add(hide)");
        assert_ok(&r.select_rect((80, 0, 120, 100), "e3 参考"), "e3 参考 pc_select");
        assert_ok(&r.mask_fill_from_selection(r_layer, "reveal", "e3 参考"), "e3 参考 mask fill(reveal)");
        let expect = r.render((0, 0, DOC_W as i32, DOC_H as i32), "e3 参考渲染");
        let diff = assert_pixels_equivalent(&after, &expect, "e3 变换后 vs 旋转后矩形蒙版参考");
        eprintln!(
            "e3 mask_present=true bbox={bb:?} size={w}x{h} byte_equal={} avg_diff={diff:.4}",
            diff == 0.0
        );
    }

    /// e4：缩放后 `pc_undo` 把尺寸与内容包围盒恢复成 200×100，渲染与变换前逐字节相等。
    #[test]
    fn e4_undo_restores_pixels_after_scale() {
        let c = Ctx::white(DOC_W, DOC_H, "e4");
        let layer = c.layer("e4");
        assert_ok(&c.filter(layer, r#"{"filter":"invert"}"#, "e4 造内容"), "e4 pc_filter_apply");

        let full = (0, 0, DOC_W as i32, DOC_H as i32);
        let before = c.render(full, "e4 变换前");
        let before_bb = bbox_content(&before, DOC_W as i32, DOC_H as i32)
            .unwrap_or_else(|| panic!("e4: 变换前应有黑色内容"));
        assert_eq!(before_bb, (0, 0, 200, 100), "e4: 变换前内容包围盒应为整层");

        let rev0 = c.rev();
        let state = c.transform(layer, r#"{"scaleX":0.5,"scaleY":0.5}"#, "e4");
        assert_ok(&state, "e4 transform");
        let rev1 = c.rev();
        assert_eq!(rev1, rev0 + 1, "e4: 缩放成功应恰好一次 checkpoint（{rev0}->{rev1}）");
        let scaled = c.render(full, "e4 缩放后");
        assert!(scaled != before, "e4: 缩放后渲染应与变换前不同（否则 e4 无意义）");

        let undone = c.undo("e4");
        assert_ok(&undone, "e4 pc_undo");
        assert_eq!(c.size(), (DOC_W, DOC_H), "e4: undo 后画布尺寸应仍为 200x100");
        let restored = c.render(full, "e4 undo 后");
        let restored_bb = bbox_content(&restored, DOC_W as i32, DOC_H as i32)
            .unwrap_or_else(|| panic!("e4: undo 后应仍有黑色内容"));
        assert_eq!(restored_bb, (0, 0, 200, 100), "e4: undo 后内容包围盒应恢复为整层 200x100");
        assert_bytes_equal(&before, &restored, "e4: undo 后渲染必须与变换前逐字节相等");

        let rev2 = c.rev();
        eprintln!(
            "e4 undo size={}x{} bbox={restored_bb:?} byte_equal=true rev={rev0}->{rev1}->{rev2}",
            DOC_W, DOC_H
        );
    }

    /// e5：非法参数全部失败且零副作用（revision、画布尺寸都不变），调整层不可变换。
    #[test]
    fn e5_invalid_params_leave_document_untouched() {
        let c = Ctx::white(DOC_W, DOC_H, "e5");
        let layer = c.layer("e5");
        let adjustment = c.adjustment_add("invert", "e5");
        let rev0 = c.rev();
        let size0 = c.size();

        // 越界 / 非有限 / 坏 JSON 共用 `invalid transform`（§2.3）。
        let cases: [(&str, &str); 5] = [
            (r#"{"scaleX":0}"#, "invalid transform"),
            (r#"{"scaleY":101}"#, "invalid transform"),
            (r#"{"angle":1e6}"#, "invalid transform"),
            (r#"{"dx":100001}"#, "invalid transform"),
            (r#"{"scaleX":"#, "invalid transform"),
        ];
        for (params, needle) in cases {
            let state = c.transform(layer, params, &format!("e5 {params}"));
            assert_fail(&state, &format!("e5 {params}"), needle);
            assert_eq!(c.rev(), rev0, "e5 {params}: 失败不应改 revision");
            assert_eq!(c.size(), size0, "e5 {params}: 失败不应改画布尺寸");
        }

        let state = c.transform(adjustment, r#"{"scaleX":1.5}"#, "e5 调整层");
        assert_fail(&state, "e5 调整层", "layer cannot be transformed");
        assert_eq!(c.rev(), rev0, "e5 调整层: 失败不应改 revision");
        assert_eq!(c.size(), size0, "e5 调整层: 失败不应改画布尺寸");

        eprintln!(
            "e5 invalid_cases=5 adjustment=1 rev={rev0} size={}x{}",
            size0.0, size0.1
        );
    }

    /// e6：全默认参数（`{}` 与显式 1/0）返回成功，但 revision 与 canUndo 都不变（不 checkpoint）。
    #[test]
    fn e6_no_op_transform_skips_checkpoint() {
        let c = Ctx::white(DOC_W, DOC_H, "e6");
        let layer = c.layer("e6");
        let rev0 = c.rev();
        assert!(!c.can_undo(), "e6: 新文档不应有可撤销操作");

        let empty = c.transform(layer, "{}", "e6 {}");
        assert_ok(&empty, "e6 {}");
        assert_eq!(bounds_field(&empty, "e6 {}"), (0, 0, 200, 100), "e6 空参数: 应返回当前内容包围盒");
        assert_eq!(c.rev(), rev0, "e6 {{}}: 无变化不应改 revision");

        let explicit = c.transform(layer, r#"{"scaleX":1,"scaleY":1,"angle":0,"dx":0,"dy":0}"#, "e6 显式默认");
        assert_ok(&explicit, "e6 显式默认");
        assert_eq!(bounds_field(&explicit, "e6 显式默认"), (0, 0, 200, 100), "e6 显式默认: 应返回当前内容包围盒");
        assert_eq!(c.rev(), rev0, "e6 显式默认: 无变化不应改 revision");
        assert!(!c.can_undo(), "e6: 无变化不应产生 checkpoint（canUndo 仍为 false）");

        eprintln!("e6 no_op=2 rev={rev0} unchanged canUndo=false");
    }

    /// e7：并行重采样（输出 ≥ `PARALLEL_RESAMPLE_MIN_PIXELS`）与串行分支逐位等价。
    ///
    /// 两条基线都取自 `resample_affine` 自身，不改生产分派：
    /// - 并行：一次 400×400 输出（160,000 px ≥ 阈值）走 `par_chunks_mut`；
    /// - 串行：把同一输出拆成 4 个 200×200 子矩形（各 40,000 px < 阈值）分别重采样后拼回；
    ///   每行的填充只依赖 `inv` 与目标行号，与所在块无关，所以拼接结果就是串行逐行结果。
    /// 比较用 `f32::to_bits` 严格逐样本相等（不是容差），并用 ABI 返回的
    /// `perf.resamplePixels` 证明真实 `pc_layer_transform` 路径也跨过了并行阈值。
    #[test]
    fn e7_parallel_resample_matches_serial_bits() {
        const SRC: usize = 800;
        const OUT: usize = 400;
        const CH: usize = 4;
        let fmt = photocraft_color::PixelFormat::RGBA32F;

        // 斜条纹 + 双轴渐变 + 半透明 alpha：同色或单通道数据可能让错误插值蒙对，这里不会。
        let mut data = vec![0.0f32; SRC * SRC * CH];
        for y in 0..SRC {
            for x in 0..SRC {
                let t = (((x + y) % 97) as f32) / 96.0;
                let px = &mut data[(y * SRC + x) * CH..][..CH];
                px[0] = t;
                px[1] = x as f32 / (SRC - 1) as f32;
                px[2] = y as f32 / (SRC - 1) as f32;
                px[3] = 0.5 + 0.5 * t;
            }
        }
        let mut src = Surface::with_default(fmt, &[0.0, 0.0, 0.0, 0.0]);
        src.write_region(Rect::new(0, 0, SRC as i32, SRC as i32), &data);

        // 与 `pc_layer_transform` 的 `scaleX/Y = 0.5` 同构：以内容中心缩小一半。
        let center = SRC as f64 / 2.0;
        let a = Affine::translate(-center, -center)
            .then(&Affine::scale(0.5))
            .then(&Affine::translate(center, center));
        let inv = a.inverse().expect("e7: 缩放仿射应可逆");
        let full = turn_map_rect(&a, src.content_bounds());
        assert_eq!((full.width(), full.height()), (OUT as u32, OUT as u32), "e7: 输出应为 {OUT}x{OUT}, 实际 {full:?}");
        let parallel_px = (full.width() * full.height()) as usize;
        assert!(
            cfg!(not(target_arch = "wasm32")) && parallel_px >= PARALLEL_RESAMPLE_MIN_PIXELS,
            "e7: 输出 {parallel_px} px 必须跨过并行阈值 {PARALLEL_RESAMPLE_MIN_PIXELS}，否则本用例没有覆盖并行分支"
        );

        let par = resample_affine(&src, &inv, full);
        let par_samples = par.read_region(full);
        assert_eq!(par_samples.len(), OUT * OUT * CH, "e7: 并行输出样本数应为 {OUT}x{OUT}x{CH}");

        let half = (OUT / 2) as i32;
        let quads = [
            Rect::new(full.x0, full.y0, full.x0 + half, full.y0 + half),
            Rect::new(full.x0 + half, full.y0, full.x1, full.y0 + half),
            Rect::new(full.x0, full.y0 + half, full.x0 + half, full.y1),
            Rect::new(full.x0 + half, full.y0 + half, full.x1, full.y1),
        ];
        let mut ser_samples = vec![0.0f32; OUT * OUT * CH];
        let chunk_px = (half * half) as usize;
        for quad in quads {
            assert!(
                chunk_px < PARALLEL_RESAMPLE_MIN_PIXELS,
                "e7: 串行分块 {quad:?} 只有 {chunk_px} px，必须低于阈值 {PARALLEL_RESAMPLE_MIN_PIXELS}"
            );
            let block = resample_affine(&src, &inv, quad).read_region(quad);
            let (qw, qh) = (quad.width() as usize, quad.height() as usize);
            let (ox, oy) = ((quad.x0 - full.x0) as usize, (quad.y0 - full.y0) as usize);
            for row in 0..qh {
                let from = row * qw * CH;
                let to = ((oy + row) * OUT + ox) * CH;
                ser_samples[to..to + qw * CH].copy_from_slice(&block[from..from + qw * CH]);
            }
        }

        let diff_count = par_samples
            .iter()
            .zip(&ser_samples)
            .filter(|(p, s)| p.to_bits() != s.to_bits())
            .count();
        if let Some(i) = par_samples.iter().zip(&ser_samples).position(|(p, s)| p.to_bits() != s.to_bits()) {
            panic!(
                "e7: 首个不等样本 #{i}（像素 {} 通道 {}）：并行 bits={:#010x} 串行 bits={:#010x}",
                i / CH,
                i % CH,
                par_samples[i].to_bits(),
                ser_samples[i].to_bits()
            );
        }
        assert_eq!(diff_count, 0, "e7: 并行与串行结果应逐位相等，实际 {diff_count} 个样本不同");

        // ABI 侧：同一仿射在真实 `pc_layer_transform` 里也要跨过阈值（perf 按实际重采样像素累加）。
        let c = Ctx::white(SRC as u32, SRC as u32, "e7");
        let layer = c.layer("e7");
        let state = c.transform(layer, r#"{"scaleX":0.5,"scaleY":0.5}"#, "e7");
        assert_ok(&state, "e7");
        assert_eq!(
            bounds_field(&state, "e7"),
            (200, 200, 600, 600),
            "e7: 800×800 缩放一半后内容包围盒应为 [200,200,600,600]"
        );
        let resample_pixels = state["perf"]["resamplePixels"]
            .as_u64()
            .unwrap_or_else(|| panic!("e7: 缺少 perf.resamplePixels（{state}）"));
        assert!(
            resample_pixels >= PARALLEL_RESAMPLE_MIN_PIXELS as u64,
            "e7: ABI 实际重采样 {resample_pixels} px，必须 ≥ 阈值 {PARALLEL_RESAMPLE_MIN_PIXELS}"
        );

        eprintln!(
            "e7 parallel_px={parallel_px} threshold={PARALLEL_RESAMPLE_MIN_PIXELS} serial_chunks={} serial_chunk_px={chunk_px} serial_px={} diff_count={diff_count} first_diff=none resamplePixels={resample_pixels}",
            quads.len(),
            OUT * OUT
        );
    }

    /// M1：宿主 5 档重采样曲线。默认跳过，只在显式 `--ignored` 运行时执行，
    /// 因此不改变默认 `cargo test --lib` 的通过计数。
    ///
    /// 与 `pc_layer_transform` 同构：以内容中心为不动点做纯缩放 `T(-c)·S(s)·T(c)`，
    /// 输出区域取该仿射的 AABB（`turn_map_rect`），整块交给 `resample_affine`。
    /// 每档重复 `REPEATS` 次，各段耗时取中位数以压掉单次抖动。
    #[ignore = "perf curve: 5-scale resample profile, run explicitly with --ignored"]
    #[test]
    fn m1_perf_curve_host() {
        const SRC_W: usize = 2400;
        const SRC_H: usize = 1500;
        const CH: usize = 4;
        const REPEATS: usize = 5;
        let fmt = photocraft_color::PixelFormat::RGBA32F;

        // 斜条纹 + 双轴渐变 + 半透明 alpha：内容非平凡，重采样不会被 prune 剪空或退化为常数场。
        let mut data = vec![0.0f32; SRC_W * SRC_H * CH];
        for y in 0..SRC_H {
            for x in 0..SRC_W {
                let t = (((x + y) % 97) as f32) / 96.0;
                let px = &mut data[(y * SRC_W + x) * CH..][..CH];
                px[0] = t;
                px[1] = x as f32 / (SRC_W - 1) as f32;
                px[2] = y as f32 / (SRC_H - 1) as f32;
                px[3] = 0.5 + 0.5 * t;
            }
        }
        let mut src = Surface::with_default(fmt, &[0.0, 0.0, 0.0, 0.0]);
        src.write_region(Rect::new(0, 0, SRC_W as i32, SRC_H as i32), &data);
        let bounds = src.content_bounds();
        let center = [(f64::from(bounds.x0) + f64::from(bounds.x1)) / 2.0, (f64::from(bounds.y0) + f64::from(bounds.y1)) / 2.0];

        let median = |mut v: [f64; REPEATS]| -> f64 {
            v.sort_by(|a, b| a.partial_cmp(b).expect("M1: 耗时必须是有限值"));
            v[REPEATS / 2]
        };

        for s in [0.20f64, 0.35, 0.50, 0.70, 1.00] {
            let a = Affine::translate(-center[0], -center[1])
                .then(&Affine::scale(s))
                .then(&Affine::translate(center[0], center[1]));
            let inv = a.inverse().unwrap_or_else(|| panic!("M1: s={s:.2} 仿射应可逆"));
            let dest = turn_map_rect(&a, bounds);
            assert!(!dest.is_empty(), "M1: s={s:.2} 输出 AABB 不应为空");

            let mut px = [0u64; REPEATS];
            let mut read = [0.0f64; REPEATS];
            let mut fill = [0.0f64; REPEATS];
            let mut write = [0.0f64; REPEATS];
            let mut prune = [0.0f64; REPEATS];
            let mut total = [0.0f64; REPEATS];
            for i in 0..REPEATS {
                let before = resample_snapshot();
                let out = resample_affine(&src, &inv, dest);
                let after = resample_snapshot();
                assert!(!out.content_bounds().is_empty(), "M1: s={s:.2} 第 {} 次输出被 prune 剪空", i + 1);
                px[i] = after.pixels.saturating_sub(before.pixels);
                read[i] = after.read_nanos.saturating_sub(before.read_nanos) as f64 / 1e6;
                fill[i] = after.fill_nanos.saturating_sub(before.fill_nanos) as f64 / 1e6;
                write[i] = after.write_nanos.saturating_sub(before.write_nanos) as f64 / 1e6;
                prune[i] = after.prune_nanos.saturating_sub(before.prune_nanos) as f64 / 1e6;
                total[i] = after.nanos.saturating_sub(before.nanos) as f64 / 1e6;
            }
            assert!(px.iter().all(|v| *v == px[0]), "M1: s={s:.2} 各次 px 应一致，实际 {px:?}");
            eprintln!(
                "[M1-CURVE] s={s:.2} px={} readMs={:.3} fillMs={:.3} writeMs={:.3} pruneMs={:.3} resampleMs={:.3} threads={}",
                px[0],
                median(read),
                median(fill),
                median(write),
                median(prune),
                median(total),
                rayon_threads()
            );
        }
        eprintln!("[M1-CURVE] done n={REPEATS}");
    }
}

