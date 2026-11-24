#![allow(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! Device compute-capability probing and SIMD pixel primitives.
//!
//! Dispatch strategy ("use the silicon when it is there, fall back when it is not"):
//! - aarch64 (every HarmonyOS phone): NEON is part of the ARMv8 baseline, so the vector path is
//!   selected at compile time and a scalar tail handles the remainder.
//! - x86_64 (emulator / PC): the tier is probed at runtime (AVX2 -> SSE2), but only NEON currently
//!   has a hand-written vector kernel; x86_64 runs the scalar path until one is added.
//! - anything else: scalar.
//!
//! Callers never need to branch: every public entry point degrades to the bit-identical scalar
//! implementation when the vector path is unavailable or the slice is too short.

use std::sync::OnceLock;

/// CPU SIMD capability tier.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SimdTier {
    /// ARMv8 NEON (baseline on aarch64).
    Neon,
    /// x86_64 AVX2.
    Avx2,
    /// x86_64 SSE2.
    Sse2,
    /// No vector kernel available; scalar loops.
    Scalar,
}

impl SimdTier {
    /// Stable lowercase label for logs and the ABI profile JSON.
    pub const fn label(self) -> &'static str {
        match self {
            SimdTier::Neon => "neon",
            SimdTier::Avx2 => "avx2",
            SimdTier::Sse2 => "sse2",
            SimdTier::Scalar => "scalar",
        }
    }

    /// Whether a hand-written vector kernel exists for this tier.
    pub const fn has_vector_kernel(self) -> bool {
        matches!(self, SimdTier::Neon | SimdTier::Sse2)
    }
}

/// Probed device profile, reported to ArkTS via the ABI.
#[derive(Clone, Copy, Debug)]
pub struct ComputeProfile {
    /// `std::env::consts::ARCH` ("aarch64" / "x86_64").
    pub arch: &'static str,
    /// Detected CPU SIMD tier.
    pub simd: SimdTier,
    /// Whether the Vulkan loader is present (a *capability* report, not a claim that the GPU
    /// compositor is wired up; see the `photocraft-gpu` desktop crate for the reference design).
    pub vulkan: bool,
    /// Whether the HarmonyOS Neural Network Runtime loader is present (an NPU *capability* report,
    /// mirroring [`vulkan_available`]). A missing loader means no NPU compute backend, so the
    /// acceleration dispatch falls back to the CPU SIMD path and ultimately the scalar reference.
    /// The NPU acceleration path itself is wired up separately; this is the capability probe that
    /// the "use the silicon when it is there, fall back when it is not" strategy keys off.
    pub npu: bool,
}

impl ComputeProfile {
    /// One-line description for the ABI JSON.
    pub fn summary(&self) -> String {
        format!("{} / {} / vulkan={} npu={}", self.arch, self.simd.label(), self.vulkan, self.npu)
    }
}

static PROFILE: OnceLock<ComputeProfile> = OnceLock::new();

/// Probe (once) and return the device compute profile.
pub fn profile() -> &'static ComputeProfile {
    PROFILE.get_or_init(|| ComputeProfile {
        arch: std::env::consts::ARCH,
        simd: detect_tier(),
        vulkan: vulkan_available(),
        npu: npu_available(),
    })
}

fn detect_tier() -> SimdTier {
    #[cfg(all(target_arch = "aarch64", target_feature = "neon"))]
    {
        return SimdTier::Neon;
    }
    #[cfg(target_arch = "x86_64")]
    {
        // BUGFIX(CAP-DETECT@1): AVX2 之前从未被探测，枚举形同虚设；但当前尚无手写 AVX2
        // 内核（has_vector_kernel() 只对 NEON/SSE2 为真），因此 AVX2 机器仍按 SSE2 档位
        // 走已验证内核，避免冒用 avx2 标签却无加速。未来补 AVX2 内核时把这里改为返回 Avx2。
        if std::arch::is_x86_feature_detected!("sse2") {
            return SimdTier::Sse2;
        }
    }
    SimdTier::Scalar
}

fn vulkan_available() -> bool {
    ["/system/lib64/libvulkan.so", "/system/lib/libvulkan.so"]
        .iter()
        .any(|p| std::path::Path::new(p).exists())
}

fn npu_available() -> bool {
    ["/system/lib64/libneural_network_runtime.so", "/system/lib/libneural_network_runtime.so"]
        .iter()
        .any(|p| std::path::Path::new(p).exists())
}

/// Quantise straight-alpha RGBA f32 to RGBA u8.
pub fn quantize_rgba8(out: &mut [[u8; 4]], src: &[[f32; 4]]) {
    let n = out.len().min(src.len());
    #[cfg(all(target_arch = "aarch64", target_feature = "neon"))]
    {
        if n >= 4 {
            unsafe { neon::quantize(&mut out[..n], &src[..n]) };
            return;
        }
    }
    #[cfg(target_arch = "x86_64")]
    {
        if n >= 4 && std::arch::is_x86_feature_detected!("sse2") {
            unsafe { sse2::quantize(&mut out[..n], &src[..n]) };
            return;
        }
    }
    scalar::quantize(&mut out[..n], &src[..n]);
}

/// Multiply-accumulate one span of straight-alpha RGBA f32 by `scale` and add `bias`, in place.
pub fn affine_rgba8(pixels: &mut [[f32; 4]], scale: f32, bias: f32) {
    #[cfg(all(target_arch = "aarch64", target_feature = "neon"))]
    {
        if pixels.len() >= 4 {
            unsafe { neon::affine(pixels, scale, bias) };
            return;
        }
    }
    #[cfg(target_arch = "x86_64")]
    {
        if pixels.len() >= 4 && std::arch::is_x86_feature_detected!("sse2") {
            unsafe { sse2::affine(pixels, scale, bias) };
            return;
        }
    }
    scalar::affine(pixels, scale, bias);
}

/// Flatten straight-alpha RGBA f32 over an opaque background colour, in place.
pub fn over_background_rgba8(pixels: &mut [[f32; 4]], bg: [f32; 3]) {
    #[cfg(target_arch = "aarch64")]
    {
        if npu_available() && unsafe { npu::npu_over_background(pixels, bg) } {
            return;
        }
    }
    #[cfg(all(target_arch = "aarch64", target_feature = "neon"))]
    {
        if pixels.len() >= 4 {
            unsafe { neon::over_background(pixels, bg) };
            return;
        }
    }
    #[cfg(target_arch = "x86_64")]
    {
        if pixels.len() >= 4 && std::arch::is_x86_feature_detected!("sse2") {
            unsafe { sse2::over_background(pixels, bg) };
            return;
        }
    }
    scalar::over_background(pixels, bg);
}

/// Invert RGB in place: `p[i] = 1.0 - p[i]` for `i` in 0..3, alpha preserved, skipping transparent
/// pixels (`p[3] <= 0`) — the `Adjustment::Invert` op that `map_rgb` performs.
pub fn invert_rgba8(pixels: &mut [[f32; 4]]) {
    #[cfg(target_arch = "aarch64")]
    {
        if npu_available() && unsafe { npu::npu_invert(pixels) } {
            return;
        }
    }
    #[cfg(all(target_arch = "aarch64", target_feature = "neon"))]
    {
        if pixels.len() >= 4 {
            unsafe { neon::invert(pixels) };
            return;
        }
    }
    #[cfg(target_arch = "x86_64")]
    {
        if pixels.len() >= 4 && std::arch::is_x86_feature_detected!("sse2") {
            unsafe { sse2::invert(pixels) };
            return;
        }
    }
    scalar::invert(pixels);
}

/// Channel Mixer: `out[i] = (matrix[i][0]*c[0] + matrix[i][1]*c[1] + matrix[i][2]*c[2] + matrix[i][3]).clamp(0.0, 1.0)`
/// for `i` in 0..3, alpha preserved, skipping transparent pixels (`p[3] <= 0`) — the
/// `Adjustment::ChannelMixer` op (non-F32 branch) that `map_rgb` performs. When `monochrome`, all
/// three output channels use `matrix[0]`.
pub fn channel_mixer_rgba8(pixels: &mut [[f32; 4]], matrix: [[f32; 4]; 3], monochrome: bool) {
    #[cfg(target_arch = "aarch64")]
    {
        if npu_available() && unsafe { npu::npu_channel_mixer(pixels, matrix, monochrome) } {
            return;
        }
    }
    #[cfg(all(target_arch = "aarch64", target_feature = "neon"))]
    {
        if pixels.len() >= 4 {
            unsafe { neon::channel_mixer(pixels, matrix, monochrome) };
            return;
        }
    }
    #[cfg(target_arch = "x86_64")]
    {
        if pixels.len() >= 4 && std::arch::is_x86_feature_detected!("sse2") {
            unsafe { sse2::channel_mixer(pixels, matrix, monochrome) };
            return;
        }
    }
    scalar::channel_mixer(pixels, matrix, monochrome);
}

/// 3x3 matrix-vector product on RGB: `out[i] = matrix[i][0]*c[0] + matrix[i][1]*c[1] + matrix[i][2]*c[2]`
/// for `i` in 0..3, alpha preserved, skipping transparent pixels (`p[3] <= 0`) — the
/// `Adjustment::PhotoFilter` op (F32 / linear-doc branch) that `map_rgb` performs. No clamp
/// (F32 documents keep superwhites).
pub fn mat3_mul_rgba8(pixels: &mut [[f32; 4]], matrix: [[f32; 3]; 3]) {
    #[cfg(target_arch = "aarch64")]
    {
        if npu_available() && unsafe { npu::npu_mat3_mul(pixels, matrix) } {
            return;
        }
    }
    #[cfg(all(target_arch = "aarch64", target_feature = "neon"))]
    {
        if pixels.len() >= 4 {
            unsafe { neon::mat3_mul(pixels, matrix) };
            return;
        }
    }
    #[cfg(target_arch = "x86_64")]
    {
        if pixels.len() >= 4 && std::arch::is_x86_feature_detected!("sse2") {
            unsafe { sse2::mat3_mul(pixels, matrix) };
            return;
        }
    }
    scalar::mat3_mul(pixels, matrix);
}

/// Adjustment-layer quantumisation: `v = branchless_floor(v * q + 0.5) / q` for every channel of
/// every pixel — the `quantize` op that `composite` performs after every adjustment layer in an
/// integer-depth document. No skip (all pixels, all 4 channels). Bit-identical to the scalar
/// reference: `branchless_floor` is branchless (abs + add + sub + copysign + 2× cmp + 2× select),
/// every op is a single IEEE-754 f32 op or a bit-level select, so the NEON/SSE2 kernels reproduce
/// the scalar rounding exactly. The scalar tail handles the remainder.
pub fn quantize_floor_rgba8(pixels: &mut [[f32; 4]], q: f32) {
    // BUGFIX(Q-ZERO@1): q<=0 或非有限值时除法产生 inf/NaN，污染整幅文档。
    // 调用方（composite::quantize）传 255/32768 等合法值；这里守住公开 API 的边界。
    if !q.is_finite() || q <= 0.0 {
        return;
    }
    #[cfg(target_arch = "aarch64")]
    {
        if npu_available() && unsafe { npu::npu_quantize_floor(pixels, q) } {
            return;
        }
    }
    #[cfg(all(target_arch = "aarch64", target_feature = "neon"))]
    {
        if pixels.len() >= 4 {
            unsafe { neon::quantize_floor(pixels, q) };
            return;
        }
    }
    #[cfg(target_arch = "x86_64")]
    {
        if pixels.len() >= 4 && std::arch::is_x86_feature_detected!("sse2") {
            unsafe { sse2::quantize_floor(pixels, q) };
            return;
        }
    }
    scalar::quantize_floor(pixels, q);
}

mod scalar {
    pub fn quantize(out: &mut [[u8; 4]], src: &[[f32; 4]]) {
        for (o, p) in out.iter_mut().zip(src) {
            for (dst, v) in o.iter_mut().zip(p) {
                *dst = (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            }
        }
    }

    pub fn affine(pixels: &mut [[f32; 4]], scale: f32, bias: f32) {
        for p in pixels {
            for v in p {
                *v = *v * scale + bias;
            }
        }
    }

    pub fn over_background(pixels: &mut [[f32; 4]], bg: [f32; 3]) {
        for p in pixels {
            let a = p[3];
            for k in 0..3 {
                p[k] = p[k] * a + bg[k] * (1.0 - a);
            }
            p[3] = 1.0;
        }
    }

    pub fn invert(pixels: &mut [[f32; 4]]) {
        for p in pixels {
            if p[3] <= 0.0 {
                continue;
            }
            p[0] = 1.0 - p[0];
            p[1] = 1.0 - p[1];
            p[2] = 1.0 - p[2];
        }
    }

    pub fn channel_mixer(pixels: &mut [[f32; 4]], matrix: [[f32; 4]; 3], monochrome: bool) {
        for p in pixels {
            if p[3] <= 0.0 {
                continue;
            }
            let c = [p[0], p[1], p[2]];
            let mix = |row: &[f32; 4]| (row[0] * c[0] + row[1] * c[1] + row[2] * c[2] + row[3]).clamp(0.0, 1.0);
            let out = if monochrome {
                [mix(&matrix[0]); 3]
            } else {
                [mix(&matrix[0]), mix(&matrix[1]), mix(&matrix[2])]
            };
            p[0] = out[0];
            p[1] = out[1];
            p[2] = out[2];
        }
    }

    pub fn mat3_mul(pixels: &mut [[f32; 4]], matrix: [[f32; 3]; 3]) {
        for p in pixels {
            if p[3] <= 0.0 {
                continue;
            }
            let c = [p[0], p[1], p[2]];
            let out = matrix.map(|row| row[0] * c[0] + row[1] * c[1] + row[2] * c[2]);
            p[0] = out[0];
            p[1] = out[1];
            p[2] = out[2];
        }
    }

    /// Scalar reference for [`super::quantize_floor_rgba8`]: `v = branchless_floor(v * q + 0.5) / q`
    /// for every channel of every pixel. Same op sequence as `composite::quantize`.
    pub fn quantize_floor(pixels: &mut [[f32; 4]], q: f32) {
        const WHOLE: f32 = 8_388_608.0;
        for p in pixels {
            for v in p.iter_mut() {
                let x = *v * q + 0.5;
                let a = x.abs();
                let r = ((a + WHOLE) - WHOLE).copysign(x);
                let r = if r > x { r - 1.0 } else { r };
                *v = if a < WHOLE { r } else { x } / q;
            }
        }
    }
}

#[cfg(all(target_arch = "aarch64", target_feature = "neon"))]
mod neon {
    use core::arch::aarch64::*;

    #[target_feature(enable = "neon")]
    pub unsafe fn quantize(out: &mut [[u8; 4]], src: &[[f32; 4]]) {
        unsafe {
            let zero = vdupq_n_f32(0.0);
            let one = vdupq_n_f32(1.0);
            let half = vdupq_n_f32(0.5);
            let n = out.len().min(src.len());
            let mut i = 0;
            while i + 4 <= n {
                let u0 = quant_pixel(src.as_ptr().add(i), zero, one, half);
                let u1 = quant_pixel(src.as_ptr().add(i + 1), zero, one, half);
                let u2 = quant_pixel(src.as_ptr().add(i + 2), zero, one, half);
                let u3 = quant_pixel(src.as_ptr().add(i + 3), zero, one, half);
                let w01 = vcombine_u16(vqmovn_u32(u0), vqmovn_u32(u1));
                let w23 = vcombine_u16(vqmovn_u32(u2), vqmovn_u32(u3));
                let mut tmp = [0u8; 16];
                vst1_u8(tmp.as_mut_ptr(), vqmovn_u16(w01));
                vst1_u8(tmp.as_mut_ptr().add(8), vqmovn_u16(w23));
                out[i].copy_from_slice(&tmp[0..4]);
                out[i + 1].copy_from_slice(&tmp[4..8]);
                out[i + 2].copy_from_slice(&tmp[8..12]);
                out[i + 3].copy_from_slice(&tmp[12..16]);
                i += 4;
            }
            for (o, p) in out[i..n].iter_mut().zip(&src[i..n]) {
                for (dst, v) in o.iter_mut().zip(p) {
                    *dst = (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                }
            }
        }
    }

    #[target_feature(enable = "neon")]
    pub unsafe fn affine(pixels: &mut [[f32; 4]], scale: f32, bias: f32) {
        unsafe {
            let s = vdupq_n_f32(scale);
            let b = vdupq_n_f32(bias);
            let n = pixels.len();
            let mut i = 0;
            while i + 4 <= n {
                let ptr = pixels.as_mut_ptr().add(i) as *mut f32;
                let p0 = vaddq_f32(vmulq_f32(vld1q_f32(ptr), s), b);
                let p1 = vaddq_f32(vmulq_f32(vld1q_f32(ptr.add(4)), s), b);
                let p2 = vaddq_f32(vmulq_f32(vld1q_f32(ptr.add(8)), s), b);
                let p3 = vaddq_f32(vmulq_f32(vld1q_f32(ptr.add(12)), s), b);
                vst1q_f32(ptr, p0);
                vst1q_f32(ptr.add(4), p1);
                vst1q_f32(ptr.add(8), p2);
                vst1q_f32(ptr.add(12), p3);
                i += 4;
            }
            for p in pixels[i..].iter_mut() {
                for v in p.iter_mut() {
                    *v = *v * scale + bias;
                }
            }
        }
    }

    #[target_feature(enable = "neon")]
    pub unsafe fn over_background(pixels: &mut [[f32; 4]], bg: [f32; 3]) {
        unsafe {
            let one = vdupq_n_f32(1.0);
            let bgv = vld1q_f32([bg[0], bg[1], bg[2], 0.0].as_ptr());
            let n = pixels.len();
            let mut i = 0;
            while i + 4 <= n {
                let ptr = pixels.as_mut_ptr().add(i) as *mut f32;
                let p0 = vld1q_f32(ptr);
                let p1 = vld1q_f32(ptr.add(4));
                let p2 = vld1q_f32(ptr.add(8));
                let p3 = vld1q_f32(ptr.add(12));
                let a0 = vdupq_n_f32(vgetq_lane_f32(p0, 3));
                let a1 = vdupq_n_f32(vgetq_lane_f32(p1, 3));
                let a2 = vdupq_n_f32(vgetq_lane_f32(p2, 3));
                let a3 = vdupq_n_f32(vgetq_lane_f32(p3, 3));
                let o0 = vsubq_f32(one, a0);
                let o1 = vsubq_f32(one, a1);
                let o2 = vsubq_f32(one, a2);
                let o3 = vsubq_f32(one, a3);
                let pa0 = vmulq_f32(p0, a0);
                let pa1 = vmulq_f32(p1, a1);
                let pa2 = vmulq_f32(p2, a2);
                let pa3 = vmulq_f32(p3, a3);
                let bo0 = vmulq_f32(bgv, o0);
                let bo1 = vmulq_f32(bgv, o1);
                let bo2 = vmulq_f32(bgv, o2);
                let bo3 = vmulq_f32(bgv, o3);
                let r0 = vaddq_f32(pa0, bo0);
                let r1 = vaddq_f32(pa1, bo1);
                let r2 = vaddq_f32(pa2, bo2);
                let r3 = vaddq_f32(pa3, bo3);
                let ro0 = vsetq_lane_f32(1.0, r0, 3);
                let ro1 = vsetq_lane_f32(1.0, r1, 3);
                let ro2 = vsetq_lane_f32(1.0, r2, 3);
                let ro3 = vsetq_lane_f32(1.0, r3, 3);
                vst1q_f32(ptr, ro0);
                vst1q_f32(ptr.add(4), ro1);
                vst1q_f32(ptr.add(8), ro2);
                vst1q_f32(ptr.add(12), ro3);
                i += 4;
            }
            for p in pixels[i..].iter_mut() {
                let a = p[3];
                for k in 0..3 {
                    p[k] = p[k] * a + bg[k] * (1.0 - a);
                }
                p[3] = 1.0;
            }
        }
    }

    #[target_feature(enable = "neon")]
    pub unsafe fn invert(pixels: &mut [[f32; 4]]) {
        unsafe {
            let one = vdupq_n_f32(1.0);
            let zero = vdupq_n_f32(0.0);
            let n = pixels.len();
            let mut i = 0;
            while i + 4 <= n {
                let ptr = pixels.as_mut_ptr().add(i) as *mut f32;
                let p0 = vld1q_f32(ptr);
                let p1 = vld1q_f32(ptr.add(4));
                let p2 = vld1q_f32(ptr.add(8));
                let p3 = vld1q_f32(ptr.add(12));
                let i0 = vsetq_lane_f32(vgetq_lane_f32(p0, 3), vsubq_f32(one, p0), 3);
                let i1 = vsetq_lane_f32(vgetq_lane_f32(p1, 3), vsubq_f32(one, p1), 3);
                let i2 = vsetq_lane_f32(vgetq_lane_f32(p2, 3), vsubq_f32(one, p2), 3);
                let i3 = vsetq_lane_f32(vgetq_lane_f32(p3, 3), vsubq_f32(one, p3), 3);
                let a0 = vdupq_n_f32(vgetq_lane_f32(p0, 3));
                let a1 = vdupq_n_f32(vgetq_lane_f32(p1, 3));
                let a2 = vdupq_n_f32(vgetq_lane_f32(p2, 3));
                let a3 = vdupq_n_f32(vgetq_lane_f32(p3, 3));
                let k0 = vcleq_f32(a0, zero);
                let k1 = vcleq_f32(a1, zero);
                let k2 = vcleq_f32(a2, zero);
                let k3 = vcleq_f32(a3, zero);
                let r0 = vbslq_f32(k0, p0, i0);
                let r1 = vbslq_f32(k1, p1, i1);
                let r2 = vbslq_f32(k2, p2, i2);
                let r3 = vbslq_f32(k3, p3, i3);
                vst1q_f32(ptr, r0);
                vst1q_f32(ptr.add(4), r1);
                vst1q_f32(ptr.add(8), r2);
                vst1q_f32(ptr.add(12), r3);
                i += 4;
            }
            for p in pixels[i..].iter_mut() {
                if p[3] <= 0.0 {
                    continue;
                }
                p[0] = 1.0 - p[0];
                p[1] = 1.0 - p[1];
                p[2] = 1.0 - p[2];
            }
        }
    }

    #[target_feature(enable = "neon")]
    pub unsafe fn channel_mixer(pixels: &mut [[f32; 4]], matrix: [[f32; 4]; 3], monochrome: bool) {
        unsafe {
            let zero = vdupq_n_f32(0.0);
            let one = vdupq_n_f32(1.0);
            let n = pixels.len();
            let mut i = 0;
            while i + 4 <= n {
                let ptr = pixels.as_mut_ptr().add(i) as *mut f32;
                let p0 = vld1q_f32(ptr);
                let p1 = vld1q_f32(ptr.add(4));
                let p2 = vld1q_f32(ptr.add(8));
                let p3 = vld1q_f32(ptr.add(12));
                let t02 = vtrnq_f32(p0, p2);
                let t13 = vtrnq_f32(p1, p3);
                let rb = vzipq_f32(t02.0, t13.0);
                let ga = vzipq_f32(t02.1, t13.1);
                let r = rb.0;
                let b = rb.1;
                let g = ga.0;
                let a = ga.1;
                let m0 = matrix[0];
                let out0 = vminq_f32(vmaxq_f32(
                    vaddq_f32(
                        vaddq_f32(
                            vaddq_f32(vmulq_n_f32(r, m0[0]), vmulq_n_f32(g, m0[1])),
                            vmulq_n_f32(b, m0[2])),
                        vdupq_n_f32(m0[3])),
                    zero), one);
                let out1 = if monochrome {
                    out0
                } else {
                    let m1 = matrix[1];
                    vminq_f32(vmaxq_f32(
                        vaddq_f32(
                            vaddq_f32(
                                vaddq_f32(vmulq_n_f32(r, m1[0]), vmulq_n_f32(g, m1[1])),
                                vmulq_n_f32(b, m1[2])),
                            vdupq_n_f32(m1[3])),
                        zero), one)
                };
                let out2 = if monochrome {
                    out0
                } else {
                    let m2 = matrix[2];
                    vminq_f32(vmaxq_f32(
                        vaddq_f32(
                            vaddq_f32(
                                vaddq_f32(vmulq_n_f32(r, m2[0]), vmulq_n_f32(g, m2[1])),
                                vmulq_n_f32(b, m2[2])),
                            vdupq_n_f32(m2[3])),
                        zero), one)
                };
                let k = vcleq_f32(a, zero);
                let so0 = vbslq_f32(k, r, out0);
                let so1 = vbslq_f32(k, g, out1);
                let so2 = vbslq_f32(k, b, out2);
                let rb_lo = vuzpq_f32(so0, so2);
                let ga_lo = vuzpq_f32(so1, a);
                let q02 = vtrnq_f32(rb_lo.0, ga_lo.0);
                let q13 = vtrnq_f32(rb_lo.1, ga_lo.1);
                vst1q_f32(ptr, q02.0);
                vst1q_f32(ptr.add(4), q13.0);
                vst1q_f32(ptr.add(8), q02.1);
                vst1q_f32(ptr.add(12), q13.1);
                i += 4;
            }
            for p in pixels[i..].iter_mut() {
                if p[3] <= 0.0 {
                    continue;
                }
                let c = [p[0], p[1], p[2]];
                let mix = |row: &[f32; 4]| (row[0] * c[0] + row[1] * c[1] + row[2] * c[2] + row[3]).clamp(0.0, 1.0);
                let out = if monochrome {
                    [mix(&matrix[0]); 3]
                } else {
                    [mix(&matrix[0]), mix(&matrix[1]), mix(&matrix[2])]
                };
                p[0] = out[0];
                p[1] = out[1];
                p[2] = out[2];
            }
        }
    }

    #[target_feature(enable = "neon")]
    pub unsafe fn mat3_mul(pixels: &mut [[f32; 4]], matrix: [[f32; 3]; 3]) {
        unsafe {
            let zero = vdupq_n_f32(0.0);
            let n = pixels.len();
            let mut i = 0;
            while i + 4 <= n {
                let ptr = pixels.as_mut_ptr().add(i) as *mut f32;
                let p0 = vld1q_f32(ptr);
                let p1 = vld1q_f32(ptr.add(4));
                let p2 = vld1q_f32(ptr.add(8));
                let p3 = vld1q_f32(ptr.add(12));
                let t02 = vtrnq_f32(p0, p2);
                let t13 = vtrnq_f32(p1, p3);
                let rb = vzipq_f32(t02.0, t13.0);
                let ga = vzipq_f32(t02.1, t13.1);
                let r = rb.0;
                let b = rb.1;
                let g = ga.0;
                let a = ga.1;
                let m0 = matrix[0];
                let out0 = vaddq_f32(
                    vaddq_f32(vmulq_n_f32(r, m0[0]), vmulq_n_f32(g, m0[1])),
                    vmulq_n_f32(b, m0[2]));
                let m1 = matrix[1];
                let out1 = vaddq_f32(
                    vaddq_f32(vmulq_n_f32(r, m1[0]), vmulq_n_f32(g, m1[1])),
                    vmulq_n_f32(b, m1[2]));
                let m2 = matrix[2];
                let out2 = vaddq_f32(
                    vaddq_f32(vmulq_n_f32(r, m2[0]), vmulq_n_f32(g, m2[1])),
                    vmulq_n_f32(b, m2[2]));
                let k = vcleq_f32(a, zero);
                let so0 = vbslq_f32(k, r, out0);
                let so1 = vbslq_f32(k, g, out1);
                let so2 = vbslq_f32(k, b, out2);
                let rb_lo = vuzpq_f32(so0, so2);
                let ga_lo = vuzpq_f32(so1, a);
                let q02 = vtrnq_f32(rb_lo.0, ga_lo.0);
                let q13 = vtrnq_f32(rb_lo.1, ga_lo.1);
                vst1q_f32(ptr, q02.0);
                vst1q_f32(ptr.add(4), q13.0);
                vst1q_f32(ptr.add(8), q02.1);
                vst1q_f32(ptr.add(12), q13.1);
                i += 4;
            }
            for p in pixels[i..].iter_mut() {
                if p[3] <= 0.0 {
                    continue;
                }
                let c = [p[0], p[1], p[2]];
                let out = matrix.map(|row| row[0] * c[0] + row[1] * c[1] + row[2] * c[2]);
                p[0] = out[0];
                p[1] = out[1];
                p[2] = out[2];
            }
        }
    }

    /// Vectorised adjustment-layer quantumisation, four pixels per iteration.
    ///
    /// # Safety
    /// NEON is baseline on aarch64; the kernel only touches the given slice.
    #[target_feature(enable = "neon")]
    pub unsafe fn quantize_floor(pixels: &mut [[f32; 4]], q: f32) {
        unsafe {
            let whole = vdupq_n_f32(8_388_608.0);
            let half = vdupq_n_f32(0.5);
            let one = vdupq_n_f32(1.0);
            let qv = vdupq_n_f32(q);
            let n = pixels.len();
            let mut i = 0;
            while i + 4 <= n {
                let ptr = pixels.as_mut_ptr().add(i) as *mut f32;
                let p0 = vld1q_f32(ptr);
                let p1 = vld1q_f32(ptr.add(4));
                let p2 = vld1q_f32(ptr.add(8));
                let p3 = vld1q_f32(ptr.add(12));
                let t02 = vtrnq_f32(p0, p2);
                let t13 = vtrnq_f32(p1, p3);
                let rb = vzipq_f32(t02.0, t13.0);
                let ga = vzipq_f32(t02.1, t13.1);
                let r = rb.0;
                let b = rb.1;
                let g = ga.0;
                let a = ga.1;
                let xf = vaddq_f32(vmulq_f32(r, qv), half);
                let xg = vaddq_f32(vmulq_f32(g, qv), half);
                let xb = vaddq_f32(vmulq_f32(b, qv), half);
                let xa = vaddq_f32(vmulq_f32(a, qv), half);
                let fr = floor_neon(xf, whole, one);
                let fg = floor_neon(xg, whole, one);
                let fb = floor_neon(xb, whole, one);
                let fa = floor_neon(xa, whole, one);
                let ro0 = vdivq_f32(fr, qv);
                let ro1 = vdivq_f32(fg, qv);
                let ro2 = vdivq_f32(fb, qv);
                let ro3 = vdivq_f32(fa, qv);
                let rb_lo = vuzpq_f32(ro0, ro2);
                let ga_lo = vuzpq_f32(ro1, ro3);
                let q02 = vtrnq_f32(rb_lo.0, ga_lo.0);
                let q13 = vtrnq_f32(rb_lo.1, ga_lo.1);
                vst1q_f32(ptr, q02.0);
                vst1q_f32(ptr.add(4), q13.0);
                vst1q_f32(ptr.add(8), q02.1);
                vst1q_f32(ptr.add(12), q13.1);
                i += 4;
            }
            for p in pixels[i..].iter_mut() {
                for v in p.iter_mut() {
                    let x = *v * q + 0.5;
                    let a = x.abs();
                    let r = ((a + 8_388_608.0) - 8_388_608.0).copysign(x);
                    let r = if r > x { r - 1.0 } else { r };
                    *v = if a < 8_388_608.0 { r } else { x } / q;
                }
            }
        }
    }

    #[inline]
    #[target_feature(enable = "neon")]
    unsafe fn vand_f32(a: float32x4_t, b: float32x4_t) -> float32x4_t {
        vreinterpretq_f32_s32(vandq_s32(vreinterpretq_s32_f32(a), vreinterpretq_s32_f32(b)))
    }

    #[inline]
    #[target_feature(enable = "neon")]
    unsafe fn vorr_f32(a: float32x4_t, b: float32x4_t) -> float32x4_t {
        vreinterpretq_f32_s32(vorrq_s32(vreinterpretq_s32_f32(a), vreinterpretq_s32_f32(b)))
    }

    /// `branchless_floor` as a NEON vector kernel — same op sequence as the scalar reference.
    #[inline]
    #[target_feature(enable = "neon")]
    unsafe fn floor_neon(x: float32x4_t, whole: float32x4_t, one: float32x4_t) -> float32x4_t {
        unsafe {
            let abs_mask = vdupq_n_f32(f32::from_bits(0x7FFF_FFFF));
            let sign_mask = vdupq_n_f32(f32::from_bits(0x8000_0000));
            let a = vand_f32(x, abs_mask);
            let t = vsubq_f32(vaddq_f32(a, whole), whole);
            let r = vorr_f32(vand_f32(t, abs_mask), vand_f32(x, sign_mask));
            let m1 = vcgtq_f32(r, x);
            let r1 = vsubq_f32(r, one);
            let r = vbslq_f32(m1, r1, r);
            let m2 = vcltq_f32(a, whole);
            vbslq_f32(m2, r, x)
        }
    }

    #[inline]
    #[target_feature(enable = "neon")]
    unsafe fn quant_pixel(p: *const [f32; 4], zero: float32x4_t, one: float32x4_t, half: float32x4_t) -> uint32x4_t {
        unsafe {
            let v = vld1q_f32(p as *const f32);
            let clamped = vminq_f32(vmaxq_f32(v, zero), one);
            vcvtq_u32_f32(vmlaq_n_f32(half, clamped, 255.0))
        }
    }
}

#[cfg(target_arch = "x86_64")]
mod sse2 {
    use core::arch::x86_64::*;

    #[target_feature(enable = "sse2")]
    pub unsafe fn quantize(out: &mut [[u8; 4]], src: &[[f32; 4]]) {
        unsafe {
            let zero = _mm_setzero_ps();
            let one = _mm_set1_ps(1.0);
            let scale = _mm_set1_ps(255.0);
            let half = _mm_set1_ps(0.5);
            let n = out.len().min(src.len());
            let mut i = 0;
            while i + 4 <= n {
                let p0 = quant_pixel(src.as_ptr().add(i), zero, one, scale, half);
                let p1 = quant_pixel(src.as_ptr().add(i + 1), zero, one, scale, half);
                let p2 = quant_pixel(src.as_ptr().add(i + 2), zero, one, scale, half);
                let p3 = quant_pixel(src.as_ptr().add(i + 3), zero, one, scale, half);
                let w01 = _mm_packs_epi32(p0, p1);
                let w23 = _mm_packs_epi32(p2, p3);
                let bytes = _mm_packus_epi16(w01, w23);
                let mut tmp = [0u8; 16];
                _mm_storeu_si128(tmp.as_mut_ptr() as *mut __m128i, bytes);
                out[i].copy_from_slice(&tmp[0..4]);
                out[i + 1].copy_from_slice(&tmp[4..8]);
                out[i + 2].copy_from_slice(&tmp[8..12]);
                out[i + 3].copy_from_slice(&tmp[12..16]);
                i += 4;
            }
            for (o, p) in out[i..n].iter_mut().zip(&src[i..n]) {
                for (dst, v) in o.iter_mut().zip(p) {
                    *dst = (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                }
            }
        }
    }

    #[target_feature(enable = "sse2")]
    pub unsafe fn affine(pixels: &mut [[f32; 4]], scale: f32, bias: f32) {
        unsafe {
            let s = _mm_set1_ps(scale);
            let b = _mm_set1_ps(bias);
            let n = pixels.len();
            let mut i = 0;
            while i + 4 <= n {
                let ptr = pixels.as_mut_ptr().add(i) as *mut f32;
                let p0 = _mm_add_ps(_mm_mul_ps(_mm_loadu_ps(ptr), s), b);
                let p1 = _mm_add_ps(_mm_mul_ps(_mm_loadu_ps(ptr.add(4)), s), b);
                let p2 = _mm_add_ps(_mm_mul_ps(_mm_loadu_ps(ptr.add(8)), s), b);
                let p3 = _mm_add_ps(_mm_mul_ps(_mm_loadu_ps(ptr.add(12)), s), b);
                _mm_storeu_ps(ptr, p0);
                _mm_storeu_ps(ptr.add(4), p1);
                _mm_storeu_ps(ptr.add(8), p2);
                _mm_storeu_ps(ptr.add(12), p3);
                i += 4;
            }
            for p in pixels[i..].iter_mut() {
                for v in p.iter_mut() {
                    *v = *v * scale + bias;
                }
            }
        }
    }

    #[target_feature(enable = "sse2")]
    pub unsafe fn over_background(pixels: &mut [[f32; 4]], bg: [f32; 3]) {
        unsafe {
            let one = _mm_set1_ps(1.0);
            let bgv = _mm_set_ps(0.0, bg[2], bg[1], bg[0]);
            let rgb_mask = _mm_castsi128_ps(_mm_set_epi32(0, !0, !0, !0));
            let alpha_one = _mm_set_ps(1.0, 0.0, 0.0, 0.0);
            let n = pixels.len();
            let mut i = 0;
            while i + 4 <= n {
                let ptr = pixels.as_mut_ptr().add(i) as *mut f32;
                let p0 = _mm_loadu_ps(ptr);
                let p1 = _mm_loadu_ps(ptr.add(4));
                let p2 = _mm_loadu_ps(ptr.add(8));
                let p3 = _mm_loadu_ps(ptr.add(12));
                let a0 = _mm_shuffle_ps(p0, p0, 0b11_11_11_11);
                let a1 = _mm_shuffle_ps(p1, p1, 0b11_11_11_11);
                let a2 = _mm_shuffle_ps(p2, p2, 0b11_11_11_11);
                let a3 = _mm_shuffle_ps(p3, p3, 0b11_11_11_11);
                let o0 = _mm_sub_ps(one, a0);
                let o1 = _mm_sub_ps(one, a1);
                let o2 = _mm_sub_ps(one, a2);
                let o3 = _mm_sub_ps(one, a3);
                let pa0 = _mm_mul_ps(p0, a0);
                let pa1 = _mm_mul_ps(p1, a1);
                let pa2 = _mm_mul_ps(p2, a2);
                let pa3 = _mm_mul_ps(p3, a3);
                let bo0 = _mm_mul_ps(bgv, o0);
                let bo1 = _mm_mul_ps(bgv, o1);
                let bo2 = _mm_mul_ps(bgv, o2);
                let bo3 = _mm_mul_ps(bgv, o3);
                let r0 = _mm_add_ps(pa0, bo0);
                let r1 = _mm_add_ps(pa1, bo1);
                let r2 = _mm_add_ps(pa2, bo2);
                let r3 = _mm_add_ps(pa3, bo3);
                let ro0 = _mm_or_ps(_mm_and_ps(r0, rgb_mask), alpha_one);
                let ro1 = _mm_or_ps(_mm_and_ps(r1, rgb_mask), alpha_one);
                let ro2 = _mm_or_ps(_mm_and_ps(r2, rgb_mask), alpha_one);
                let ro3 = _mm_or_ps(_mm_and_ps(r3, rgb_mask), alpha_one);
                _mm_storeu_ps(ptr, ro0);
                _mm_storeu_ps(ptr.add(4), ro1);
                _mm_storeu_ps(ptr.add(8), ro2);
                _mm_storeu_ps(ptr.add(12), ro3);
                i += 4;
            }
            for p in pixels[i..].iter_mut() {
                let a = p[3];
                for k in 0..3 {
                    p[k] = p[k] * a + bg[k] * (1.0 - a);
                }
                p[3] = 1.0;
            }
        }
    }

    #[target_feature(enable = "sse2")]
    pub unsafe fn invert(pixels: &mut [[f32; 4]]) {
        unsafe {
            let one = _mm_set1_ps(1.0);
            let zero = _mm_setzero_ps();
            let rgb_mask = _mm_castsi128_ps(_mm_set_epi32(0, !0, !0, !0));
            let alpha_mask = _mm_castsi128_ps(_mm_set_epi32(!0, 0, 0, 0));
            let n = pixels.len();
            let mut i = 0;
            while i + 4 <= n {
                let ptr = pixels.as_mut_ptr().add(i) as *mut f32;
                let p0 = _mm_loadu_ps(ptr);
                let p1 = _mm_loadu_ps(ptr.add(4));
                let p2 = _mm_loadu_ps(ptr.add(8));
                let p3 = _mm_loadu_ps(ptr.add(12));
                let i0 = _mm_or_ps(_mm_and_ps(_mm_sub_ps(one, p0), rgb_mask), _mm_and_ps(p0, alpha_mask));
                let i1 = _mm_or_ps(_mm_and_ps(_mm_sub_ps(one, p1), rgb_mask), _mm_and_ps(p1, alpha_mask));
                let i2 = _mm_or_ps(_mm_and_ps(_mm_sub_ps(one, p2), rgb_mask), _mm_and_ps(p2, alpha_mask));
                let i3 = _mm_or_ps(_mm_and_ps(_mm_sub_ps(one, p3), rgb_mask), _mm_and_ps(p3, alpha_mask));
                let a0 = _mm_shuffle_ps(p0, p0, 0b11_11_11_11);
                let a1 = _mm_shuffle_ps(p1, p1, 0b11_11_11_11);
                let a2 = _mm_shuffle_ps(p2, p2, 0b11_11_11_11);
                let a3 = _mm_shuffle_ps(p3, p3, 0b11_11_11_11);
                let k0 = _mm_cmple_ps(a0, zero);
                let k1 = _mm_cmple_ps(a1, zero);
                let k2 = _mm_cmple_ps(a2, zero);
                let k3 = _mm_cmple_ps(a3, zero);
                let r0 = _mm_or_ps(_mm_and_ps(p0, k0), _mm_andnot_ps(k0, i0));
                let r1 = _mm_or_ps(_mm_and_ps(p1, k1), _mm_andnot_ps(k1, i1));
                let r2 = _mm_or_ps(_mm_and_ps(p2, k2), _mm_andnot_ps(k2, i2));
                let r3 = _mm_or_ps(_mm_and_ps(p3, k3), _mm_andnot_ps(k3, i3));
                _mm_storeu_ps(ptr, r0);
                _mm_storeu_ps(ptr.add(4), r1);
                _mm_storeu_ps(ptr.add(8), r2);
                _mm_storeu_ps(ptr.add(12), r3);
                i += 4;
            }
            for p in pixels[i..].iter_mut() {
                if p[3] <= 0.0 {
                    continue;
                }
                p[0] = 1.0 - p[0];
                p[1] = 1.0 - p[1];
                p[2] = 1.0 - p[2];
            }
        }
    }

    #[target_feature(enable = "sse2")]
    pub unsafe fn channel_mixer(pixels: &mut [[f32; 4]], matrix: [[f32; 4]; 3], monochrome: bool) {
        unsafe {
            let zero = _mm_setzero_ps();
            let one = _mm_set1_ps(1.0);
            let n = pixels.len();
            let mut i = 0;
            while i + 4 <= n {
                let ptr = pixels.as_mut_ptr().add(i) as *mut f32;
                let p0 = _mm_loadu_ps(ptr);
                let p1 = _mm_loadu_ps(ptr.add(4));
                let p2 = _mm_loadu_ps(ptr.add(8));
                let p3 = _mm_loadu_ps(ptr.add(12));
                let t0 = _mm_unpacklo_ps(p0, p1);
                let t1 = _mm_unpackhi_ps(p0, p1);
                let t2 = _mm_unpacklo_ps(p2, p3);
                let t3 = _mm_unpackhi_ps(p2, p3);
                let r = _mm_shuffle_ps(t0, t2, 0b01_00_01_00);
                let g = _mm_shuffle_ps(t0, t2, 0b11_10_11_10);
                let b = _mm_shuffle_ps(t1, t3, 0b01_00_01_00);
                let a = _mm_shuffle_ps(t1, t3, 0b11_10_11_10);
                let m0 = matrix[0];
                let out0 = _mm_min_ps(_mm_max_ps(
                    _mm_add_ps(
                        _mm_add_ps(
                            _mm_add_ps(_mm_mul_ps(r, _mm_set1_ps(m0[0])), _mm_mul_ps(g, _mm_set1_ps(m0[1]))),
                            _mm_mul_ps(b, _mm_set1_ps(m0[2]))),
                        _mm_set1_ps(m0[3])),
                    zero), one);
                let out1 = if monochrome {
                    out0
                } else {
                    let m1 = matrix[1];
                    _mm_min_ps(_mm_max_ps(
                        _mm_add_ps(
                            _mm_add_ps(
                                _mm_add_ps(_mm_mul_ps(r, _mm_set1_ps(m1[0])), _mm_mul_ps(g, _mm_set1_ps(m1[1]))),
                                _mm_mul_ps(b, _mm_set1_ps(m1[2]))),
                            _mm_set1_ps(m1[3])),
                        zero), one)
                };
                let out2 = if monochrome {
                    out0
                } else {
                    let m2 = matrix[2];
                    _mm_min_ps(_mm_max_ps(
                        _mm_add_ps(
                            _mm_add_ps(
                                _mm_add_ps(_mm_mul_ps(r, _mm_set1_ps(m2[0])), _mm_mul_ps(g, _mm_set1_ps(m2[1]))),
                                _mm_mul_ps(b, _mm_set1_ps(m2[2]))),
                            _mm_set1_ps(m2[3])),
                        zero), one)
                };
                let k = _mm_cmple_ps(a, zero);
                let so0 = _mm_or_ps(_mm_and_ps(k, r), _mm_andnot_ps(k, out0));
                let so1 = _mm_or_ps(_mm_and_ps(k, g), _mm_andnot_ps(k, out1));
                let so2 = _mm_or_ps(_mm_and_ps(k, b), _mm_andnot_ps(k, out2));
                let rg_lo = _mm_unpacklo_ps(so0, so1);
                let rg_hi = _mm_unpackhi_ps(so0, so1);
                let ba_lo = _mm_unpacklo_ps(so2, a);
                let ba_hi = _mm_unpackhi_ps(so2, a);
                let p0s = _mm_shuffle_ps(rg_lo, ba_lo, 0b01_00_01_00);
                let p1s = _mm_shuffle_ps(rg_lo, ba_lo, 0b11_10_11_10);
                let p2s = _mm_shuffle_ps(rg_hi, ba_hi, 0b01_00_01_00);
                let p3s = _mm_shuffle_ps(rg_hi, ba_hi, 0b11_10_11_10);
                _mm_storeu_ps(ptr, p0s);
                _mm_storeu_ps(ptr.add(4), p1s);
                _mm_storeu_ps(ptr.add(8), p2s);
                _mm_storeu_ps(ptr.add(12), p3s);
                i += 4;
            }
            for p in pixels[i..].iter_mut() {
                if p[3] <= 0.0 {
                    continue;
                }
                let c = [p[0], p[1], p[2]];
                let mix = |row: &[f32; 4]| (row[0] * c[0] + row[1] * c[1] + row[2] * c[2] + row[3]).clamp(0.0, 1.0);
                let out = if monochrome {
                    [mix(&matrix[0]); 3]
                } else {
                    [mix(&matrix[0]), mix(&matrix[1]), mix(&matrix[2])]
                };
                p[0] = out[0];
                p[1] = out[1];
                p[2] = out[2];
            }
        }
    }

    #[target_feature(enable = "sse2")]
    pub unsafe fn mat3_mul(pixels: &mut [[f32; 4]], matrix: [[f32; 3]; 3]) {
        unsafe {
            let zero = _mm_setzero_ps();
            let n = pixels.len();
            let mut i = 0;
            while i + 4 <= n {
                let ptr = pixels.as_mut_ptr().add(i) as *mut f32;
                let p0 = _mm_loadu_ps(ptr);
                let p1 = _mm_loadu_ps(ptr.add(4));
                let p2 = _mm_loadu_ps(ptr.add(8));
                let p3 = _mm_loadu_ps(ptr.add(12));
                let t0 = _mm_unpacklo_ps(p0, p1);
                let t1 = _mm_unpackhi_ps(p0, p1);
                let t2 = _mm_unpacklo_ps(p2, p3);
                let t3 = _mm_unpackhi_ps(p2, p3);
                let r = _mm_shuffle_ps(t0, t2, 0b01_00_01_00);
                let g = _mm_shuffle_ps(t0, t2, 0b11_10_11_10);
                let b = _mm_shuffle_ps(t1, t3, 0b01_00_01_00);
                let a = _mm_shuffle_ps(t1, t3, 0b11_10_11_10);
                let m0 = matrix[0];
                let out0 = _mm_add_ps(
                    _mm_add_ps(_mm_mul_ps(r, _mm_set1_ps(m0[0])), _mm_mul_ps(g, _mm_set1_ps(m0[1]))),
                    _mm_mul_ps(b, _mm_set1_ps(m0[2])));
                let m1 = matrix[1];
                let out1 = _mm_add_ps(
                    _mm_add_ps(_mm_mul_ps(r, _mm_set1_ps(m1[0])), _mm_mul_ps(g, _mm_set1_ps(m1[1]))),
                    _mm_mul_ps(b, _mm_set1_ps(m1[2])));
                let m2 = matrix[2];
                let out2 = _mm_add_ps(
                    _mm_add_ps(_mm_mul_ps(r, _mm_set1_ps(m2[0])), _mm_mul_ps(g, _mm_set1_ps(m2[1]))),
                    _mm_mul_ps(b, _mm_set1_ps(m2[2])));
                let k = _mm_cmple_ps(a, zero);
                let so0 = _mm_or_ps(_mm_and_ps(k, r), _mm_andnot_ps(k, out0));
                let so1 = _mm_or_ps(_mm_and_ps(k, g), _mm_andnot_ps(k, out1));
                let so2 = _mm_or_ps(_mm_and_ps(k, b), _mm_andnot_ps(k, out2));
                let rg_lo = _mm_unpacklo_ps(so0, so1);
                let rg_hi = _mm_unpackhi_ps(so0, so1);
                let ba_lo = _mm_unpacklo_ps(so2, a);
                let ba_hi = _mm_unpackhi_ps(so2, a);
                let p0s = _mm_shuffle_ps(rg_lo, ba_lo, 0b01_00_01_00);
                let p1s = _mm_shuffle_ps(rg_lo, ba_lo, 0b11_10_11_10);
                let p2s = _mm_shuffle_ps(rg_hi, ba_hi, 0b01_00_01_00);
                let p3s = _mm_shuffle_ps(rg_hi, ba_hi, 0b11_10_11_10);
                _mm_storeu_ps(ptr, p0s);
                _mm_storeu_ps(ptr.add(4), p1s);
                _mm_storeu_ps(ptr.add(8), p2s);
                _mm_storeu_ps(ptr.add(12), p3s);
                i += 4;
            }
            for p in pixels[i..].iter_mut() {
                if p[3] <= 0.0 {
                    continue;
                }
                let c = [p[0], p[1], p[2]];
                let out = matrix.map(|row| row[0] * c[0] + row[1] * c[1] + row[2] * c[2]);
                p[0] = out[0];
                p[1] = out[1];
                p[2] = out[2];
            }
        }
    }

    /// Vectorised adjustment-layer quantumisation, four pixels per iteration.
    ///
    /// # Safety
    /// SSE2 is baseline on x86_64; the kernel only touches the given slice.
    #[target_feature(enable = "sse2")]
    pub unsafe fn quantize_floor(pixels: &mut [[f32; 4]], q: f32) {
        unsafe {
            let whole = _mm_set1_ps(8_388_608.0);
            let half = _mm_set1_ps(0.5);
            let one = _mm_set1_ps(1.0);
            let qv = _mm_set1_ps(q);
            let abs_mask = _mm_castsi128_ps(_mm_set1_epi32(0x7FFF_FFFF));
            let sign_mask = _mm_castsi128_ps(_mm_set1_epi32(0x8000_0000u32 as i32));
            let n = pixels.len();
            let mut i = 0;
            while i + 4 <= n {
                let ptr = pixels.as_mut_ptr().add(i) as *mut f32;
                let p0 = _mm_loadu_ps(ptr);
                let p1 = _mm_loadu_ps(ptr.add(4));
                let p2 = _mm_loadu_ps(ptr.add(8));
                let p3 = _mm_loadu_ps(ptr.add(12));
                let t0 = _mm_unpacklo_ps(p0, p1);
                let t1 = _mm_unpackhi_ps(p0, p1);
                let t2 = _mm_unpacklo_ps(p2, p3);
                let t3 = _mm_unpackhi_ps(p2, p3);
                let r = _mm_shuffle_ps(t0, t2, 0b01_00_01_00);
                let g = _mm_shuffle_ps(t0, t2, 0b11_10_11_10);
                let b = _mm_shuffle_ps(t1, t3, 0b01_00_01_00);
                let a = _mm_shuffle_ps(t1, t3, 0b11_10_11_10);
                let xf = _mm_add_ps(_mm_mul_ps(r, qv), half);
                let xg = _mm_add_ps(_mm_mul_ps(g, qv), half);
                let xb = _mm_add_ps(_mm_mul_ps(b, qv), half);
                let xa = _mm_add_ps(_mm_mul_ps(a, qv), half);
                let fr = floor_sse2(xf, whole, one, abs_mask, sign_mask);
                let fg = floor_sse2(xg, whole, one, abs_mask, sign_mask);
                let fb = floor_sse2(xb, whole, one, abs_mask, sign_mask);
                let fa = floor_sse2(xa, whole, one, abs_mask, sign_mask);
                let ro0 = _mm_div_ps(fr, qv);
                let ro1 = _mm_div_ps(fg, qv);
                let ro2 = _mm_div_ps(fb, qv);
                let ro3 = _mm_div_ps(fa, qv);
                let rg_lo = _mm_unpacklo_ps(ro0, ro1);
                let rg_hi = _mm_unpackhi_ps(ro0, ro1);
                let ba_lo = _mm_unpacklo_ps(ro2, ro3);
                let ba_hi = _mm_unpackhi_ps(ro2, ro3);
                let p0s = _mm_shuffle_ps(rg_lo, ba_lo, 0b01_00_01_00);
                let p1s = _mm_shuffle_ps(rg_lo, ba_lo, 0b11_10_11_10);
                let p2s = _mm_shuffle_ps(rg_hi, ba_hi, 0b01_00_01_00);
                let p3s = _mm_shuffle_ps(rg_hi, ba_hi, 0b11_10_11_10);
                _mm_storeu_ps(ptr, p0s);
                _mm_storeu_ps(ptr.add(4), p1s);
                _mm_storeu_ps(ptr.add(8), p2s);
                _mm_storeu_ps(ptr.add(12), p3s);
                i += 4;
            }
            for p in pixels[i..].iter_mut() {
                for v in p.iter_mut() {
                    let x = *v * q + 0.5;
                    let a = x.abs();
                    let r = ((a + 8_388_608.0) - 8_388_608.0).copysign(x);
                    let r = if r > x { r - 1.0 } else { r };
                    *v = if a < 8_388_608.0 { r } else { x } / q;
                }
            }
        }
    }

    /// `branchless_floor` as an SSE2 vector kernel — same op sequence as the scalar reference.
    #[inline]
    #[target_feature(enable = "sse2")]
    unsafe fn floor_sse2(x: __m128, whole: __m128, one: __m128, abs_mask: __m128, sign_mask: __m128) -> __m128 {
        unsafe {
            let a = _mm_and_ps(x, abs_mask);
            let t = _mm_sub_ps(_mm_add_ps(a, whole), whole);
            let r = _mm_or_ps(_mm_and_ps(t, abs_mask), _mm_and_ps(x, sign_mask));
            let m1 = _mm_cmpgt_ps(r, x);
            let r1 = _mm_sub_ps(r, one);
            let r = _mm_or_ps(_mm_and_ps(m1, r1), _mm_andnot_ps(m1, r));
            let m2 = _mm_cmplt_ps(a, whole);
            _mm_or_ps(_mm_and_ps(m2, r), _mm_andnot_ps(m2, x))
        }
    }

    #[inline]
    #[target_feature(enable = "sse2")]
    unsafe fn quant_pixel(p: *const [f32; 4], zero: __m128, one: __m128, scale: __m128, half: __m128) -> __m128i {
        unsafe {
            let v = _mm_loadu_ps(p as *const f32);
            let clamped = _mm_min_ps(_mm_max_ps(v, zero), one);
            _mm_cvttps_epi32(_mm_add_ps(_mm_mul_ps(clamped, scale), half))
        }
    }
}

#[cfg(target_arch = "aarch64")]
mod npu;

#[cfg(test)]
mod tests {
    use super::*;

    fn scalar_ref(src: &[[f32; 4]]) -> Vec<[u8; 4]> {
        src.iter()
            .map(|p| {
                let mut o = [0u8; 4];
                for (dst, v) in o.iter_mut().zip(p) {
                    *dst = (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                }
                o
            })
            .collect()
    }

    #[test]
    fn quantize_is_bit_identical_to_scalar_including_tail() {
        let mut vals = Vec::new();
        for k in 0..1003 {
            let f = k as f32 / 1002.0;
            vals.push([f, 1.0 - f, f * 0.5, 1.0]);
        }
        vals.push([-1.0, 2.0, 0.5 / 255.0, 1.0]);
        vals.push([0.5, f32::NAN, 0.999_999_94, 0.0]);
        vals.push([1.0, 1.0, 1.0, 1.0]);
        vals.push([0.0, 0.0, 0.0, 0.0]);
        for len in 1..=8usize {
            let slice = &vals[..len];
            let mut out = vec![[0u8; 4]; len];
            quantize_rgba8(&mut out, slice);
            assert_eq!(out, scalar_ref(slice), "len={len}");
        }
        let mut out = vec![[0u8; 4]; vals.len()];
        quantize_rgba8(&mut out, &vals);
        assert_eq!(out, scalar_ref(&vals));
    }

    #[test]
    fn quantize_handles_len_mismatch() {
        let src = vec![[0.25f32; 4]; 6];
        let mut out = vec![[0u8; 4]; 10];
        quantize_rgba8(&mut out, &src);
        assert_eq!(out[..6], scalar_ref(&src)[..]);
        assert_eq!(out[6..], vec![[0u8; 4]; 4][..]);
    }

    #[test]
    fn affine_is_bit_identical_to_scalar() {
        let mut vals = Vec::new();
        for k in 0..99 {
            let f = k as f32 / 98.0;
            vals.push([f, 1.0 - f, f * 0.25, 0.75]);
        }
        let mut a = vals.clone();
        let mut b = vals.clone();
        affine_rgba8(&mut a, 1.35, -0.12);
        scalar::affine(&mut b, 1.35, -0.12);
        assert_eq!(a, b);
    }

    #[test]
    #[ignore]
    fn bench_quantize_throughput() {
        const PIXELS: usize = 1_000_000;
        const ITERATIONS: usize = 24;
        let src = vec![[0.42f32, 0.17, 0.93, 0.5]; PIXELS];
        let mut out = vec![[0u8; 4]; PIXELS];
        let t = std::time::Instant::now();
        for _ in 0..ITERATIONS {
            quantize_rgba8(&mut out, &src);
        }
        let dispatched = t.elapsed();
        let t = std::time::Instant::now();
        for _ in 0..ITERATIONS {
            scalar::quantize(&mut out, &src);
        }
        let scalar = t.elapsed();
        println!(
            "quantize {} px x{} iters: dispatched={:?} scalar={:?} speedup={:.2}x (tier={})",
            PIXELS, ITERATIONS, dispatched, scalar, scalar.as_secs_f64() / dispatched.as_secs_f64(), profile().simd.label()
        );
        assert!(dispatched <= scalar * 11 / 10, "dispatched path slower than scalar reference");
    }

    #[test]
    fn profile_reports_a_known_tier() {
        let p = profile();
        assert!(matches!(p.simd, SimdTier::Neon | SimdTier::Avx2 | SimdTier::Sse2 | SimdTier::Scalar));
        assert!(!p.summary().is_empty());
        assert!(p.summary().contains("npu="), "summary must include npu: {}", p.summary());
    }

    #[test]
    fn over_background_is_bit_identical_to_scalar_including_tail() {
        fn scalar_ref(pixels: &mut [[f32; 4]], bg: [f32; 3]) {
            for p in pixels {
                let a = p[3];
                for k in 0..3 {
                    p[k] = p[k] * a + bg[k] * (1.0 - a);
                }
                p[3] = 1.0;
            }
        }
        let bg = [0.2, 0.4, 0.6];
        let mut vals: Vec<[f32; 4]> = Vec::new();
        for k in 0..1003 {
            let f = k as f32 / 1002.0;
            vals.push([f, 1.0 - f, f * 0.5, f]);
        }
        vals.push([0.0, 0.0, 0.0, 0.0]);
        vals.push([1.0, 1.0, 1.0, 1.0]);
        vals.push([0.5, 0.5, 0.5, 2.0]);
        vals.push([-0.2, 0.3, 0.8, -1.0]);
        for len in 1..=8usize {
            let slice = &vals[..len.min(vals.len())];
            let mut a = slice.to_vec();
            let mut b = slice.to_vec();
            over_background_rgba8(&mut a, bg);
            scalar_ref(&mut b, bg);
            assert_eq!(a, b, "len={len}");
        }
        let mut a = vals.clone();
        let mut b = vals.clone();
        over_background_rgba8(&mut a, bg);
        scalar_ref(&mut b, bg);
        assert_eq!(a, b);
    }

    #[test]
    #[ignore]
    fn bench_over_background_throughput() {
        const PIXELS: usize = 1_000_000;
        const ITERATIONS: usize = 24;
        let mut pixels = vec![[0.3f32, 0.5, 0.7, 0.6]; PIXELS];
        let bg = [0.2, 0.4, 0.6];
        let t = std::time::Instant::now();
        for _ in 0..ITERATIONS {
            over_background_rgba8(&mut pixels, bg);
        }
        let dispatched = t.elapsed();
        let mut ref2 = pixels.clone();
        let t = std::time::Instant::now();
        for _ in 0..ITERATIONS {
            scalar::over_background(&mut ref2, bg);
        }
        let scalar = t.elapsed();
        println!(
            "over_background {} px x{} iters: dispatched={:?} scalar={:?} speedup={:.2}x (tier={})",
            PIXELS, ITERATIONS, dispatched, scalar, scalar.as_secs_f64() / dispatched.as_secs_f64(), profile().simd.label()
        );
        assert!(dispatched <= scalar * 11 / 10, "dispatched path slower than scalar reference");
    }

    #[test]
    fn invert_is_bit_identical_to_scalar_including_tail() {
        fn scalar_ref(pixels: &mut [[f32; 4]]) {
            for p in pixels {
                if p[3] <= 0.0 {
                    continue;
                }
                p[0] = 1.0 - p[0];
                p[1] = 1.0 - p[1];
                p[2] = 1.0 - p[2];
            }
        }
        let mut vals: Vec<[f32; 4]> = Vec::new();
        for k in 0..1003 {
            let f = k as f32 / 1002.0;
            vals.push([f, 1.0 - f, f * 0.5, f]);
        }
        vals.push([0.0, 0.0, 0.0, 0.0]);
        vals.push([1.0, 1.0, 1.0, 1.0]);
        vals.push([0.5, 0.5, 0.5, 2.0]);
        vals.push([-0.2, 0.3, 0.8, -1.0]);
        for len in 1..=8usize {
            let slice = &vals[..len.min(vals.len())];
            let mut a = slice.to_vec();
            let mut b = slice.to_vec();
            invert_rgba8(&mut a);
            scalar_ref(&mut b);
            assert_eq!(a, b, "len={len}");
        }
        let mut a = vals.clone();
        let mut b = vals.clone();
        invert_rgba8(&mut a);
        scalar_ref(&mut b);
        assert_eq!(a, b);
    }

    #[test]
    #[ignore]
    fn bench_invert_throughput() {
        const PIXELS: usize = 1_000_000;
        const ITERATIONS: usize = 24;
        let mut pixels = vec![[0.3f32, 0.5, 0.7, 0.6]; PIXELS];
        let t = std::time::Instant::now();
        for _ in 0..ITERATIONS {
            invert_rgba8(&mut pixels);
        }
        let dispatched = t.elapsed();
        let mut ref2 = pixels.clone();
        let t = std::time::Instant::now();
        for _ in 0..ITERATIONS {
            scalar::invert(&mut ref2);
        }
        let scalar = t.elapsed();
        println!(
            "invert {} px x{} iters: dispatched={:?} scalar={:?} speedup={:.2}x (tier={})",
            PIXELS, ITERATIONS, dispatched, scalar, scalar.as_secs_f64() / dispatched.as_secs_f64(), profile().simd.label()
        );
        assert!(dispatched <= scalar * 11 / 10, "dispatched path slower than scalar reference");
    }

    #[test]
    fn channel_mixer_is_bit_identical_to_scalar_including_tail() {
        fn scalar_ref(pixels: &mut [[f32; 4]], matrix: [[f32; 4]; 3], monochrome: bool) {
            for p in pixels {
                if p[3] <= 0.0 {
                    continue;
                }
                let c = [p[0], p[1], p[2]];
                let mix = |row: &[f32; 4]| (row[0] * c[0] + row[1] * c[1] + row[2] * c[2] + row[3]).clamp(0.0, 1.0);
                let out = if monochrome {
                    [mix(&matrix[0]); 3]
                } else {
                    [mix(&matrix[0]), mix(&matrix[1]), mix(&matrix[2])]
                };
                p[0] = out[0];
                p[1] = out[1];
                p[2] = out[2];
            }
        }
        let matrix = [[0.8, 0.3, -0.1, 0.0], [0.1, 0.9, 0.0, 0.05], [0.0, 0.2, 0.7, 0.0]];
        let mut vals: Vec<[f32; 4]> = Vec::new();
        for k in 0..1003 {
            let f = k as f32 / 1002.0;
            vals.push([f, 1.0 - f, f * 0.5, f]);
        }
        vals.push([0.0, 0.0, 0.0, 0.0]);
        vals.push([1.0, 1.0, 1.0, 1.0]);
        vals.push([0.5, 0.5, 0.5, 2.0]);
        vals.push([-0.2, 0.3, 0.8, -1.0]);
        for len in 1..=8usize {
            let slice = &vals[..len.min(vals.len())];
            let mut a = slice.to_vec();
            let mut b = slice.to_vec();
            channel_mixer_rgba8(&mut a, matrix, false);
            scalar_ref(&mut b, matrix, false);
            assert_eq!(a, b, "color len={len}");
            let mut a = slice.to_vec();
            let mut b = slice.to_vec();
            channel_mixer_rgba8(&mut a, matrix, true);
            scalar_ref(&mut b, matrix, true);
            assert_eq!(a, b, "mono len={len}");
        }
        let mut a = vals.clone();
        let mut b = vals.clone();
        channel_mixer_rgba8(&mut a, matrix, false);
        scalar_ref(&mut b, matrix, false);
        assert_eq!(a, b, "color full");
        let mut a = vals.clone();
        let mut b = vals.clone();
        channel_mixer_rgba8(&mut a, matrix, true);
        scalar_ref(&mut b, matrix, true);
        assert_eq!(a, b, "mono full");
    }

    #[test]
    #[ignore]
    fn bench_channel_mixer_throughput() {
        const PIXELS: usize = 1_000_000;
        const ITERATIONS: usize = 24;
        let matrix = [[0.8, 0.3, -0.1, 0.0], [0.1, 0.9, 0.0, 0.05], [0.0, 0.2, 0.7, 0.0]];
        let mut pixels = vec![[0.3f32, 0.5, 0.7, 0.6]; PIXELS];
        let t = std::time::Instant::now();
        for _ in 0..ITERATIONS {
            channel_mixer_rgba8(&mut pixels, matrix, false);
        }
        let dispatched = t.elapsed();
        let mut ref2 = pixels.clone();
        let t = std::time::Instant::now();
        for _ in 0..ITERATIONS {
            scalar::channel_mixer(&mut ref2, matrix, false);
        }
        let scalar = t.elapsed();
        println!(
            "channel_mixer {} px x{} iters: dispatched={:?} scalar={:?} speedup={:.2}x (tier={})",
            PIXELS, ITERATIONS, dispatched, scalar, scalar.as_secs_f64() / dispatched.as_secs_f64(), profile().simd.label()
        );
        assert!(dispatched <= scalar * 11 / 10, "dispatched path slower than scalar reference");
    }

    #[test]
    fn mat3_mul_is_bit_identical_to_scalar_including_tail() {
        fn scalar_ref(pixels: &mut [[f32; 4]], matrix: [[f32; 3]; 3]) {
            for p in pixels {
                if p[3] <= 0.0 {
                    continue;
                }
                let c = [p[0], p[1], p[2]];
                let out = matrix.map(|row| row[0] * c[0] + row[1] * c[1] + row[2] * c[2]);
                p[0] = out[0];
                p[1] = out[1];
                p[2] = out[2];
            }
        }
        let matrix = [[0.8, 0.3, -0.1], [0.1, 0.9, 0.0], [0.0, 0.2, 0.7]];
        let mut vals: Vec<[f32; 4]> = Vec::new();
        for k in 0..1003 {
            let f = k as f32 / 1002.0;
            vals.push([f, 1.0 - f, f * 0.5, f]);
        }
        vals.push([0.0, 0.0, 0.0, 0.0]);
        vals.push([1.0, 1.0, 1.0, 1.0]);
        vals.push([0.5, 0.5, 0.5, 2.0]);
        vals.push([-0.2, 0.3, 0.8, -1.0]);
        for len in 1..=8usize {
            let slice = &vals[..len.min(vals.len())];
            let mut a = slice.to_vec();
            let mut b = slice.to_vec();
            mat3_mul_rgba8(&mut a, matrix);
            scalar_ref(&mut b, matrix);
            assert_eq!(a, b, "len={len}");
        }
        let mut a = vals.clone();
        let mut b = vals.clone();
        mat3_mul_rgba8(&mut a, matrix);
        scalar_ref(&mut b, matrix);
        assert_eq!(a, b);
    }

    #[test]
    #[ignore]
    fn bench_mat3_mul_throughput() {
        const PIXELS: usize = 1_000_000;
        const ITERATIONS: usize = 24;
        let matrix = [[0.8, 0.3, -0.1], [0.1, 0.9, 0.0], [0.0, 0.2, 0.7]];
        let mut pixels = vec![[0.3f32, 0.5, 0.7, 0.6]; PIXELS];
        let t = std::time::Instant::now();
        for _ in 0..ITERATIONS {
            mat3_mul_rgba8(&mut pixels, matrix);
        }
        let dispatched = t.elapsed();
        let mut ref2 = pixels.clone();
        let t = std::time::Instant::now();
        for _ in 0..ITERATIONS {
            scalar::mat3_mul(&mut ref2, matrix);
        }
        let scalar = t.elapsed();
        println!(
            "mat3_mul {} px x{} iters: dispatched={:?} scalar={:?} speedup={:.2}x (tier={})",
            PIXELS, ITERATIONS, dispatched, scalar, scalar.as_secs_f64() / dispatched.as_secs_f64(), profile().simd.label()
        );
        assert!(dispatched <= scalar * 11 / 10, "dispatched path slower than scalar reference");
    }

    #[test]
    fn quantize_floor_is_bit_identical_to_scalar_including_tail() {
        fn scalar_ref(pixels: &mut [[f32; 4]], q: f32) {
            const WHOLE: f32 = 8_388_608.0;
            for p in pixels {
                for v in p.iter_mut() {
                    let x = *v * q + 0.5;
                    let a = x.abs();
                    let r = ((a + WHOLE) - WHOLE).copysign(x);
                    let r = if r > x { r - 1.0 } else { r };
                    *v = if a < WHOLE { r } else { x } / q;
                }
            }
        }
        fn assert_f32_bits_eq(a: &[[f32; 4]], b: &[[f32; 4]], msg: &str) {
            assert_eq!(a.len(), b.len(), "{msg} length mismatch");
            for i in 0..a.len() {
                for j in 0..4 {
                    assert_eq!(
                        a[i][j].to_bits(),
                        b[i][j].to_bits(),
                        "{msg} pixel={i} channel={j}"
                    );
                }
            }
        }
        let mut vals: Vec<[f32; 4]> = Vec::new();
        for k in 0..1003 {
            let f = k as f32 / 1002.0;
            vals.push([f, 1.0 - f, f * 0.5, f]);
        }
        // Edge cases: transparent (alpha=0), opaque, super-alpha > 1, negative alpha, 0, 1,
        // values near 2^23 (WHOLE boundary), NaN, inf, negative values that floor differently.
        vals.push([0.0, 0.0, 0.0, 0.0]);
        vals.push([1.0, 1.0, 1.0, 1.0]);
        vals.push([0.5, 0.5, 0.5, 2.0]);
        vals.push([-0.2, 0.3, 0.8, -1.0]);
        vals.push([8_388_608.0, 8_388_607.5, 8_388_607.0, 1.0]);
        vals.push([f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 1.0]);
        vals.push([-0.0, -1.0, 2.0, 0.5]);
        for q in [255.0f32, 32768.0] {
            for len in 1..=8usize {
                let slice = &vals[..len.min(vals.len())];
                let mut a = slice.to_vec();
                let mut b = slice.to_vec();
                quantize_floor_rgba8(&mut a, q);
                scalar_ref(&mut b, q);
                assert_f32_bits_eq(&a, &b, &format!("q={q} len={len}"));
            }
            let mut a = vals.clone();
            let mut b = vals.clone();
            quantize_floor_rgba8(&mut a, q);
            scalar_ref(&mut b, q);
            assert_f32_bits_eq(&a, &b, &format!("q={q} full"));
        }
    }

    #[test]
    #[ignore]
    fn bench_quantize_floor_throughput() {
        const PIXELS: usize = 1_000_000;
        const ITERATIONS: usize = 24;
        let q = 255.0f32;
        let mut pixels = vec![[0.3f32, 0.5, 0.7, 0.6]; PIXELS];
        let t = std::time::Instant::now();
        for _ in 0..ITERATIONS {
            quantize_floor_rgba8(&mut pixels, q);
        }
        let dispatched = t.elapsed();
        let mut ref2 = pixels.clone();
        let t = std::time::Instant::now();
        for _ in 0..ITERATIONS {
            scalar::quantize_floor(&mut ref2, q);
        }
        let scalar = t.elapsed();
        println!(
            "quantize_floor {} px x{} iters: dispatched={:?} scalar={:?} speedup={:.2}x (tier={})",
            PIXELS, ITERATIONS, dispatched, scalar, scalar.as_secs_f64() / dispatched.as_secs_f64(), profile().simd.label()
        );
        assert!(dispatched <= scalar * 11 / 10, "dispatched path slower than scalar reference");
    }
}