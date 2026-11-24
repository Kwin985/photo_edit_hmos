//! NPU acceleration via Neural Network Runtime (NNR) — aarch64 only.
//!
//! This module provides NPU execution paths for `quantize_floor_rgba8`,
//! `invert_rgba8`, and `over_background_rgba8`. The NNR C API is used to
//! construct computation graphs and run them on the NPU device. When NPU is
//! unavailable or any NNR call fails, the caller falls back to the SIMD
//! (NEON/SSE2) / scalar path in the parent module.

use core::ffi::{c_void, c_int};

// === NNR opaque types ===

type OH_NNModel = c_void;
type OH_NNCompilation = c_void;
type OH_NNExecutor = c_void;
type NN_TensorDesc = c_void;
type NN_Tensor = c_void;
type OH_NN_ReturnCode = c_int;

// === NNR constants ===

const OH_NN_SUCCESS: c_int = 0;
const OH_NN_FLOAT32: c_int = 11;
const OH_NN_INT32: c_int = 4;
const OH_NN_INT8: c_int = 2;
const OH_NN_BOOL: c_int = 1;
const OH_NN_FORMAT_NHWC: c_int = 2;
const OH_NN_FORMAT_ND: c_int = 3;
const OH_NN_OPS_SCALE: c_int = 26;
const OH_NN_OPS_FLOOR: c_int = 93;
const OH_NN_OPS_DIV: c_int = 11;
const OH_NN_OPS_SLICE: c_int = 29;
const OH_NN_OPS_GREATER: c_int = 71;
const OH_NN_OPS_BROADCAST_TO: c_int = 69;
const OH_NN_OPS_SELECT: c_int = 62;
const OH_NN_OPS_ADD: c_int = 1;
const OH_NN_OPS_MUL: c_int = 22;
const OH_NN_OPS_SUB: c_int = 38;
const OH_NN_OPS_CONCAT: c_int = 7;
const OH_NN_OPS_BIAS_ADD: c_int = 5;
const OH_NN_OPS_MATMUL: c_int = 19;
const OH_NN_OPS_MAXIMUM: c_int = 20;
const OH_NN_OPS_MINIMUM: c_int = 97;

// === OH_NN_UInt32Array ===

#[repr(C)]
struct OH_NN_UInt32Array {
    data: *mut u32,
    size: u32,
}

// === NNR FFI: runtime library (model construction) ===

#[link(name = "neural_network_runtime")]
unsafe extern "C" {
    fn OH_NNModel_Construct() -> *mut OH_NNModel;
    fn OH_NNModel_AddTensorToModel(
        model: *mut OH_NNModel,
        tensorDesc: *const NN_TensorDesc,
    ) -> OH_NN_ReturnCode;
    fn OH_NNModel_SetTensorData(
        model: *mut OH_NNModel,
        index: u32,
        dataBuffer: *const c_void,
        length: usize,
    ) -> OH_NN_ReturnCode;
    fn OH_NNModel_AddOperation(
        model: *mut OH_NNModel,
        op: c_int,
        paramIndices: *const OH_NN_UInt32Array,
        inputIndices: *const OH_NN_UInt32Array,
        outputIndices: *const OH_NN_UInt32Array,
    ) -> OH_NN_ReturnCode;
    fn OH_NNModel_SpecifyInputsAndOutputs(
        model: *mut OH_NNModel,
        inputIndices: *const OH_NN_UInt32Array,
        outputIndices: *const OH_NN_UInt32Array,
    ) -> OH_NN_ReturnCode;
    fn OH_NNModel_Finish(model: *mut OH_NNModel) -> OH_NN_ReturnCode;
    fn OH_NNModel_Destroy(model: *mut *mut OH_NNModel);
}

// === NNR FFI: core library (compilation, execution, tensor, device) ===

#[link(name = "neural_network_core")]
unsafe extern "C" {
    fn OH_NNCompilation_Construct(model: *const OH_NNModel) -> *mut OH_NNCompilation;
    fn OH_NNCompilation_SetDevice(
        compilation: *mut OH_NNCompilation,
        deviceID: usize,
    ) -> OH_NN_ReturnCode;
    fn OH_NNCompilation_Build(compilation: *mut OH_NNCompilation) -> OH_NN_ReturnCode;
    fn OH_NNCompilation_Destroy(compilation: *mut *mut OH_NNCompilation);
    fn OH_NNExecutor_Construct(compilation: *mut OH_NNCompilation) -> *mut OH_NNExecutor;
    fn OH_NNExecutor_RunSync(
        executor: *mut OH_NNExecutor,
        inputTensor: *const *mut NN_Tensor,
        inputCount: usize,
        outputTensor: *const *mut NN_Tensor,
        outputCount: usize,
    ) -> OH_NN_ReturnCode;
    fn OH_NNExecutor_Destroy(executor: *mut *mut OH_NNExecutor);
    fn OH_NNTensorDesc_Create() -> *mut NN_TensorDesc;
    fn OH_NNTensorDesc_Destroy(tensorDesc: *mut *mut NN_TensorDesc) -> OH_NN_ReturnCode;
    fn OH_NNTensorDesc_SetDataType(
        tensorDesc: *mut NN_TensorDesc,
        dataType: c_int,
    ) -> OH_NN_ReturnCode;
    fn OH_NNTensorDesc_SetShape(
        tensorDesc: *mut NN_TensorDesc,
        shape: *const i32,
        shapeLength: usize,
    ) -> OH_NN_ReturnCode;
    fn OH_NNTensorDesc_SetFormat(
        tensorDesc: *mut NN_TensorDesc,
        format: c_int,
    ) -> OH_NN_ReturnCode;
    fn OH_NNTensor_CreateWithSize(
        deviceID: usize,
        tensorDesc: *const NN_TensorDesc,
        size: usize,
    ) -> *mut NN_Tensor;
    fn OH_NNTensor_Destroy(tensor: *mut *mut NN_Tensor) -> OH_NN_ReturnCode;
    fn OH_NNTensor_GetDataBuffer(tensor: *const NN_Tensor) -> *mut c_void;
    // BUGFIX(NN-DEVICE@1): 查询系统已注册的计算设备，避免把 device 0（通常第一个是 CPU）
    // 误当作 NPU。SDK 文档：OH_NNDevice_GetType 返回 OH_NN_ACCELERATOR(3)/OH_NN_GPU(2)/OH_NN_CPU(1)。
    fn OH_NNDevice_GetAllDevicesID(allDevicesID: *mut *const usize, deviceCount: *mut u32) -> OH_NN_ReturnCode;
    fn OH_NNDevice_GetType(deviceID: usize, deviceType: *mut c_int) -> OH_NN_ReturnCode;
}

/// 选择真正的硬件加速设备 ID：优先专用加速器（3），其次 GPU（2）；
/// 若查询失败或只找到 CPU，返回 `None` 让调用方回落到 CPU SIMD 路径——
/// 避免"声称 NPU 加速实际跑在 CPU 上"的语义错误。
/// 注：NNR 的设备 ID 是查询返回的数组元素，不是数组下标。
fn select_accelerator_device() -> Option<usize> {
    const OH_NN_ACCELERATOR: c_int = 3;
    const OH_NN_GPU: c_int = 2;
    let mut ids: *const usize = core::ptr::null();
    let mut count: u32 = 0;
    if unsafe { OH_NNDevice_GetAllDevicesID(&mut ids, &mut count) } != OH_NN_SUCCESS || count == 0 || ids.is_null() {
        return None;
    }
    let list = unsafe { core::slice::from_raw_parts(ids, count as usize) };
    // 两轮：先找加速器，再找 GPU。
    for want in [OH_NN_ACCELERATOR, OH_NN_GPU] {
        for &id in list {
            let mut ty: c_int = 0;
            if unsafe { OH_NNDevice_GetType(id, &mut ty) } == OH_NN_SUCCESS && ty == want {
                return Some(id);
            }
        }
    }
    None
}

/// 解析 `OH_NNCompilation_SetDevice` 的目标设备：优先硬件加速器；
/// 无加速器时不再伪装成 NPU，返回 false 让调用方回落 CPU SIMD。
fn set_accelerator_device(compilation: *mut OH_NNCompilation) -> bool {
    match select_accelerator_device() {
        Some(id) => unsafe { OH_NNCompilation_SetDevice(compilation, id) == OH_NN_SUCCESS },
        None => false,
    }
}

// === Helper: create a tensor desc ===

fn create_desc(
    data_type: c_int,
    shape: &[i32],
    format: c_int,
) -> Option<*mut NN_TensorDesc> {
    let mut desc = unsafe { OH_NNTensorDesc_Create() };
    if desc.is_null() {
        return None;
    }
    if unsafe { OH_NNTensorDesc_SetDataType(desc, data_type) } != OH_NN_SUCCESS {
        unsafe { OH_NNTensorDesc_Destroy(&mut desc) };
        return None;
    }
    if unsafe { OH_NNTensorDesc_SetShape(desc, shape.as_ptr(), shape.len()) } != OH_NN_SUCCESS {
        unsafe { OH_NNTensorDesc_Destroy(&mut desc) };
        return None;
    }
    if unsafe { OH_NNTensorDesc_SetFormat(desc, format) } != OH_NN_SUCCESS {
        unsafe { OH_NNTensorDesc_Destroy(&mut desc) };
        return None;
    }
    Some(desc)
}

// === NPU context for resource management ===

struct NpuCtx {
    model: *mut OH_NNModel,
    compilation: *mut OH_NNCompilation,
    executor: *mut OH_NNExecutor,
    input_tensor: *mut NN_Tensor,
    output_tensor: *mut NN_Tensor,
    descs: [*mut NN_TensorDesc; 25],
    desc_count: usize,
}

impl NpuCtx {
    fn new() -> Self {
        Self {
            model: core::ptr::null_mut(),
            compilation: core::ptr::null_mut(),
            executor: core::ptr::null_mut(),
            input_tensor: core::ptr::null_mut(),
            output_tensor: core::ptr::null_mut(),
            descs: [core::ptr::null_mut(); 25],
            desc_count: 0,
        }
    }

    fn add_desc(&mut self, desc: *mut NN_TensorDesc) {
        if self.desc_count < 25 {
            self.descs[self.desc_count] = desc;
            self.desc_count += 1;
        }
    }
}

impl Drop for NpuCtx {
    fn drop(&mut self) {
        if !self.input_tensor.is_null() {
            unsafe { OH_NNTensor_Destroy(&mut self.input_tensor) };
        }
        if !self.output_tensor.is_null() {
            unsafe { OH_NNTensor_Destroy(&mut self.output_tensor) };
        }
        if !self.executor.is_null() {
            unsafe { OH_NNExecutor_Destroy(&mut self.executor) };
        }
        if !self.compilation.is_null() {
            unsafe { OH_NNCompilation_Destroy(&mut self.compilation) };
        }
        for i in 0..self.desc_count {
            if !self.descs[i].is_null() {
                unsafe { OH_NNTensorDesc_Destroy(&mut self.descs[i]) };
            }
        }
        if !self.model.is_null() {
            unsafe { OH_NNModel_Destroy(&mut self.model) };
        }
    }
}

// === NPU quantize_floor ===
//
// NNR computation graph:
//   tensor 0: input (src)       — [1, n, 1, 4] NHWC FLOAT32  (model input)
//   tensor 1: x (intermediate)  — [1, n, 1, 4] NHWC FLOAT32
//   tensor 2: f (intermediate)  — [1, n, 1, 4] NHWC FLOAT32
//   tensor 3: out               — [1, n, 1, 4] NHWC FLOAT32  (model output)
//   tensor 4: scale             — [4] ND FLOAT32  (per-channel: q, q, q, q)
//   tensor 5: bias              — [4] ND FLOAT32  (per-channel: 0.5 x4)
//   tensor 6: q_div             — [1] ND FLOAT32  (scalar divisor: q)
//   tensor 7: axis              — [1] ND INT32    (value: 3 = channel dim)
//   tensor 8: activationType    — [1] ND INT8     (value: 0 = FUSED_NONE)
//
// Operations:
//   SCALE:  params=[7,8],  inputs=[0,4,5],  outputs=[1]   x = src * q + 0.5
//   FLOOR:  params=[],     inputs=[1],      outputs=[2]   f = floor(x)
//   DIV:    params=[8],    inputs=[2,6],    outputs=[3]   out = f / q

/// Attempt NPU execution of `quantize_floor`. Returns `true` on success,
/// `false` if NPU is unavailable or any NNR call fails (caller falls back).
pub unsafe fn npu_quantize_floor(pixels: &mut [[f32; 4]], q: f32) -> bool {
    unsafe {
        let n = pixels.len();
        if n == 0 {
            return true;
        }

        let shape: [i32; 4] = [1, n as i32, 1, 4];
        let data_size = n * 4 * 4; // n pixels * 4 channels * 4 bytes per f32

        let mut ctx = NpuCtx::new();

        // Create model
        ctx.model = OH_NNModel_Construct();
        if ctx.model.is_null() {
            return false;
        }

        // Create tensor descs
        let desc_input = match create_desc(OH_NN_FLOAT32, &shape, OH_NN_FORMAT_NHWC) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_input);

        let desc_x = match create_desc(OH_NN_FLOAT32, &shape, OH_NN_FORMAT_NHWC) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_x);

        let desc_f = match create_desc(OH_NN_FLOAT32, &shape, OH_NN_FORMAT_NHWC) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_f);

        let desc_out = match create_desc(OH_NN_FLOAT32, &shape, OH_NN_FORMAT_NHWC) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_out);

        let channel_shape: [i32; 1] = [4];
        let scalar_shape: [i32; 1] = [1];

        let desc_scale = match create_desc(OH_NN_FLOAT32, &channel_shape, OH_NN_FORMAT_ND) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_scale);

        let desc_bias = match create_desc(OH_NN_FLOAT32, &channel_shape, OH_NN_FORMAT_ND) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_bias);

        let desc_q = match create_desc(OH_NN_FLOAT32, &scalar_shape, OH_NN_FORMAT_ND) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_q);

        let desc_axis = match create_desc(OH_NN_INT32, &scalar_shape, OH_NN_FORMAT_ND) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_axis);

        let desc_act = match create_desc(OH_NN_INT8, &scalar_shape, OH_NN_FORMAT_ND) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_act);

        // Add all descs to model (indices 0..8)
        for i in 0..ctx.desc_count {
            if OH_NNModel_AddTensorToModel(ctx.model, ctx.descs[i]) != OH_NN_SUCCESS {
                return false;
            }
        }

        // Set constant tensor data
        let scale_data = [q; 4];
        if OH_NNModel_SetTensorData(ctx.model, 4, scale_data.as_ptr() as *const c_void, 16)
            != OH_NN_SUCCESS
        {
            return false;
        }

        let bias_data = [0.5f32; 4];
        if OH_NNModel_SetTensorData(ctx.model, 5, bias_data.as_ptr() as *const c_void, 16)
            != OH_NN_SUCCESS
        {
            return false;
        }

        let q_data = [q];
        if OH_NNModel_SetTensorData(ctx.model, 6, q_data.as_ptr() as *const c_void, 4)
            != OH_NN_SUCCESS
        {
            return false;
        }

        let axis_data = [3i32];
        if OH_NNModel_SetTensorData(ctx.model, 7, axis_data.as_ptr() as *const c_void, 4)
            != OH_NN_SUCCESS
        {
            return false;
        }

        let act_data = [0i8];
        if OH_NNModel_SetTensorData(ctx.model, 8, act_data.as_ptr() as *const c_void, 1)
            != OH_NN_SUCCESS
        {
            return false;
        }

        // Add operations
        let mut scale_params_arr = [7u32, 8];
        let mut scale_inputs_arr = [0u32, 4, 5];
        let mut scale_outputs_arr = [1u32];
        let scale_params = OH_NN_UInt32Array {
            data: scale_params_arr.as_mut_ptr(),
            size: 2,
        };
        let scale_inputs = OH_NN_UInt32Array {
            data: scale_inputs_arr.as_mut_ptr(),
            size: 3,
        };
        let scale_outputs = OH_NN_UInt32Array {
            data: scale_outputs_arr.as_mut_ptr(),
            size: 1,
        };
        if OH_NNModel_AddOperation(
            ctx.model,
            OH_NN_OPS_SCALE,
            &scale_params,
            &scale_inputs,
            &scale_outputs,
        ) != OH_NN_SUCCESS
        {
            return false;
        }

        let mut floor_inputs_arr = [1u32];
        let mut floor_outputs_arr = [2u32];
        let floor_params = OH_NN_UInt32Array {
            data: core::ptr::null_mut(),
            size: 0,
        };
        let floor_inputs = OH_NN_UInt32Array {
            data: floor_inputs_arr.as_mut_ptr(),
            size: 1,
        };
        let floor_outputs = OH_NN_UInt32Array {
            data: floor_outputs_arr.as_mut_ptr(),
            size: 1,
        };
        if OH_NNModel_AddOperation(
            ctx.model,
            OH_NN_OPS_FLOOR,
            &floor_params,
            &floor_inputs,
            &floor_outputs,
        ) != OH_NN_SUCCESS
        {
            return false;
        }

        let mut div_params_arr = [8u32];
        let mut div_inputs_arr = [2u32, 6];
        let mut div_outputs_arr = [3u32];
        let div_params = OH_NN_UInt32Array {
            data: div_params_arr.as_mut_ptr(),
            size: 1,
        };
        let div_inputs = OH_NN_UInt32Array {
            data: div_inputs_arr.as_mut_ptr(),
            size: 2,
        };
        let div_outputs = OH_NN_UInt32Array {
            data: div_outputs_arr.as_mut_ptr(),
            size: 1,
        };
        if OH_NNModel_AddOperation(
            ctx.model,
            OH_NN_OPS_DIV,
            &div_params,
            &div_inputs,
            &div_outputs,
        ) != OH_NN_SUCCESS
        {
            return false;
        }

        // Specify inputs/outputs
        let mut model_inputs_arr = [0u32];
        let mut model_outputs_arr = [3u32];
        let model_inputs = OH_NN_UInt32Array {
            data: model_inputs_arr.as_mut_ptr(),
            size: 1,
        };
        let model_outputs = OH_NN_UInt32Array {
            data: model_outputs_arr.as_mut_ptr(),
            size: 1,
        };
        if OH_NNModel_SpecifyInputsAndOutputs(ctx.model, &model_inputs, &model_outputs)
            != OH_NN_SUCCESS
        {
            return false;
        }

        // Finish model
        if OH_NNModel_Finish(ctx.model) != OH_NN_SUCCESS {
            return false;
        }

        // Compile
        ctx.compilation = OH_NNCompilation_Construct(ctx.model);
        if ctx.compilation.is_null() {
            return false;
        }
        if !set_accelerator_device(ctx.compilation) {
            return false;
        }
        if OH_NNCompilation_Build(ctx.compilation) != OH_NN_SUCCESS {
            return false;
        }

        // Create executor
        ctx.executor = OH_NNExecutor_Construct(ctx.compilation);
        if ctx.executor.is_null() {
            return false;
        }

        // Create input/output tensors
        ctx.input_tensor = OH_NNTensor_CreateWithSize(0, desc_input, data_size);
        if ctx.input_tensor.is_null() {
            return false;
        }
        ctx.output_tensor = OH_NNTensor_CreateWithSize(0, desc_out, data_size);
        if ctx.output_tensor.is_null() {
            return false;
        }

        // Fill input data
        let input_buf = OH_NNTensor_GetDataBuffer(ctx.input_tensor) as *mut u8;
        if input_buf.is_null() {
            return false;
        }
        core::ptr::copy_nonoverlapping(
            pixels.as_ptr() as *const u8,
            input_buf,
            data_size,
        );

        // Run inference
        let inputs = [ctx.input_tensor];
        let outputs = [ctx.output_tensor];
        if OH_NNExecutor_RunSync(ctx.executor, inputs.as_ptr(), 1, outputs.as_ptr(), 1)
            != OH_NN_SUCCESS
        {
            return false;
        }

        // Read output data
        let output_buf = OH_NNTensor_GetDataBuffer(ctx.output_tensor) as *mut u8;
        if output_buf.is_null() {
            return false;
        }
        core::ptr::copy_nonoverlapping(
            output_buf,
            pixels.as_mut_ptr() as *mut u8,
            data_size,
        );

        true
    }
}

// === NPU invert ===
//
// NNR computation graph (reproduces scalar `invert`):
//   tensor 0:  input (src)         — [1, n, 1, 4] NHWC FLOAT32  (model input)
//   tensor 1:  out                 — [1, n, 1, 4] NHWC FLOAT32  (model output)
//   tensor 2:  scale               — [4] ND FLOAT32  (per-channel: -1, -1, -1, 1)
//   tensor 3:  bias                — [4] ND FLOAT32  (per-channel: 1, 1, 1, 0)
//   tensor 4:  axis                — [1] ND INT32    (value: 3 = channel dim)
//   tensor 5:  activationType      — [1] ND INT8     (value: 0 = FUSED_NONE)
//   tensor 6:  begin               — [1] ND INT32    (value: 3 = alpha channel)
//   tensor 7:  size                — [1] ND INT32    (value: 1)
//   tensor 8:  zero                — [1] ND FLOAT32  (value: 0.0)
//   tensor 9:  broadcast_shape     — [4] ND INT32    (value: 1, n, 1, 4)
//   tensor 10: alpha               — [1, n, 1, 1] NHWC FLOAT32  (SLICE output)
//   tensor 11: mask                — [1, n, 1, 1] NHWC BOOL     (GREATER output)
//   tensor 12: mask_broadcast      — [1, n, 1, 4] NHWC BOOL     (BROADCAST_TO output)
//   tensor 13: inverted            — [1, n, 1, 4] NHWC FLOAT32  (SCALE output)
//
// Operations:
//   SLICE:         params=[4],      inputs=[0, 6, 7],   outputs=[10]  alpha = src[..., 3:4]
//   GREATER:       params=[],       inputs=[10, 8],     outputs=[11]  mask = alpha > 0
//   BROADCAST_TO:  params=[9],      inputs=[11],        outputs=[12]  mask_b = broadcast(mask, [1,n,1,4])
//   SCALE:         params=[4, 5],   inputs=[0, 2, 3],   outputs=[13]  inv = src * scale + bias
//   SELECT:        params=[],       inputs=[12, 13, 0], outputs=[1]   out = where(mask_b, inv, src)
//
// Semantics: where alpha > 0 → [1-r, 1-g, 1-b, a]; where alpha <= 0 → [r, g, b, a].
// Bit-identical to scalar `invert` (alpha preserved, transparent skipped).

/// Attempt NPU execution of `invert`. Returns `true` on success,
/// `false` if NPU is unavailable or any NNR call fails (caller falls back).
pub unsafe fn npu_invert(pixels: &mut [[f32; 4]]) -> bool {
    unsafe {
        let n = pixels.len();
        if n == 0 {
            return true;
        }

        let shape: [i32; 4] = [1, n as i32, 1, 4];
        let data_size = n * 4 * 4; // n pixels * 4 channels * 4 bytes per f32

        let mut ctx = NpuCtx::new();

        // Create model
        ctx.model = OH_NNModel_Construct();
        if ctx.model.is_null() {
            return false;
        }

        // Create tensor descs (indices 0..13)
        let desc_input = match create_desc(OH_NN_FLOAT32, &shape, OH_NN_FORMAT_NHWC) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_input);

        let desc_out = match create_desc(OH_NN_FLOAT32, &shape, OH_NN_FORMAT_NHWC) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_out);

        let channel_shape: [i32; 1] = [4];
        let scalar_shape: [i32; 1] = [1];
        let broadcast_shape: [i32; 4] = [1, n as i32, 1, 4];
        let alpha_shape: [i32; 4] = [1, n as i32, 1, 1];

        let desc_scale = match create_desc(OH_NN_FLOAT32, &channel_shape, OH_NN_FORMAT_ND) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_scale);

        let desc_bias = match create_desc(OH_NN_FLOAT32, &channel_shape, OH_NN_FORMAT_ND) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_bias);

        let desc_axis = match create_desc(OH_NN_INT32, &scalar_shape, OH_NN_FORMAT_ND) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_axis);

        let desc_act = match create_desc(OH_NN_INT8, &scalar_shape, OH_NN_FORMAT_ND) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_act);

        let desc_begin = match create_desc(OH_NN_INT32, &scalar_shape, OH_NN_FORMAT_ND) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_begin);

        let desc_size = match create_desc(OH_NN_INT32, &scalar_shape, OH_NN_FORMAT_ND) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_size);

        let desc_zero = match create_desc(OH_NN_FLOAT32, &scalar_shape, OH_NN_FORMAT_ND) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_zero);

        let desc_bshape = match create_desc(OH_NN_INT32, &broadcast_shape, OH_NN_FORMAT_ND) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_bshape);

        let desc_alpha = match create_desc(OH_NN_FLOAT32, &alpha_shape, OH_NN_FORMAT_NHWC) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_alpha);

        let desc_mask = match create_desc(OH_NN_BOOL, &alpha_shape, OH_NN_FORMAT_NHWC) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_mask);

        let desc_mask_b = match create_desc(OH_NN_BOOL, &shape, OH_NN_FORMAT_NHWC) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_mask_b);

        let desc_inv = match create_desc(OH_NN_FLOAT32, &shape, OH_NN_FORMAT_NHWC) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_inv);

        // Add all descs to model (indices 0..13)
        for i in 0..ctx.desc_count {
            if OH_NNModel_AddTensorToModel(ctx.model, ctx.descs[i]) != OH_NN_SUCCESS {
                return false;
            }
        }

        // Set constant tensor data
        let scale_data = [-1.0f32, -1.0, -1.0, 1.0];
        if OH_NNModel_SetTensorData(ctx.model, 2, scale_data.as_ptr() as *const c_void, 16)
            != OH_NN_SUCCESS
        {
            return false;
        }

        let bias_data = [1.0f32, 1.0, 1.0, 0.0];
        if OH_NNModel_SetTensorData(ctx.model, 3, bias_data.as_ptr() as *const c_void, 16)
            != OH_NN_SUCCESS
        {
            return false;
        }

        let axis_data = [3i32];
        if OH_NNModel_SetTensorData(ctx.model, 4, axis_data.as_ptr() as *const c_void, 4)
            != OH_NN_SUCCESS
        {
            return false;
        }

        let act_data = [0i8];
        if OH_NNModel_SetTensorData(ctx.model, 5, act_data.as_ptr() as *const c_void, 1)
            != OH_NN_SUCCESS
        {
            return false;
        }

        let begin_data = [3i32];
        if OH_NNModel_SetTensorData(ctx.model, 6, begin_data.as_ptr() as *const c_void, 4)
            != OH_NN_SUCCESS
        {
            return false;
        }

        let size_data = [1i32];
        if OH_NNModel_SetTensorData(ctx.model, 7, size_data.as_ptr() as *const c_void, 4)
            != OH_NN_SUCCESS
        {
            return false;
        }

        let zero_data = [0.0f32];
        if OH_NNModel_SetTensorData(ctx.model, 8, zero_data.as_ptr() as *const c_void, 4)
            != OH_NN_SUCCESS
        {
            return false;
        }

        let bshape_data = [1i32, n as i32, 1, 4];
        if OH_NNModel_SetTensorData(ctx.model, 9, bshape_data.as_ptr() as *const c_void, 16)
            != OH_NN_SUCCESS
        {
            return false;
        }

        // Add operations
        // SLICE: params=[4], inputs=[0,6,7], outputs=[10]
        let mut slice_params_arr = [4u32];
        let mut slice_inputs_arr = [0u32, 6, 7];
        let mut slice_outputs_arr = [10u32];
        let slice_params = OH_NN_UInt32Array {
            data: slice_params_arr.as_mut_ptr(),
            size: 1,
        };
        let slice_inputs = OH_NN_UInt32Array {
            data: slice_inputs_arr.as_mut_ptr(),
            size: 3,
        };
        let slice_outputs = OH_NN_UInt32Array {
            data: slice_outputs_arr.as_mut_ptr(),
            size: 1,
        };
        if OH_NNModel_AddOperation(
            ctx.model,
            OH_NN_OPS_SLICE,
            &slice_params,
            &slice_inputs,
            &slice_outputs,
        ) != OH_NN_SUCCESS
        {
            return false;
        }

        // GREATER: params=[], inputs=[10,8], outputs=[11]
        let mut greater_inputs_arr = [10u32, 8];
        let mut greater_outputs_arr = [11u32];
        let greater_params = OH_NN_UInt32Array {
            data: core::ptr::null_mut(),
            size: 0,
        };
        let greater_inputs = OH_NN_UInt32Array {
            data: greater_inputs_arr.as_mut_ptr(),
            size: 2,
        };
        let greater_outputs = OH_NN_UInt32Array {
            data: greater_outputs_arr.as_mut_ptr(),
            size: 1,
        };
        if OH_NNModel_AddOperation(
            ctx.model,
            OH_NN_OPS_GREATER,
            &greater_params,
            &greater_inputs,
            &greater_outputs,
        ) != OH_NN_SUCCESS
        {
            return false;
        }

        // BROADCAST_TO: params=[9], inputs=[11], outputs=[12]
        let mut bcast_params_arr = [9u32];
        let mut bcast_inputs_arr = [11u32];
        let mut bcast_outputs_arr = [12u32];
        let bcast_params = OH_NN_UInt32Array {
            data: bcast_params_arr.as_mut_ptr(),
            size: 1,
        };
        let bcast_inputs = OH_NN_UInt32Array {
            data: bcast_inputs_arr.as_mut_ptr(),
            size: 1,
        };
        let bcast_outputs = OH_NN_UInt32Array {
            data: bcast_outputs_arr.as_mut_ptr(),
            size: 1,
        };
        if OH_NNModel_AddOperation(
            ctx.model,
            OH_NN_OPS_BROADCAST_TO,
            &bcast_params,
            &bcast_inputs,
            &bcast_outputs,
        ) != OH_NN_SUCCESS
        {
            return false;
        }

        // SCALE: params=[4,5], inputs=[0,2,3], outputs=[13]
        let mut scale_params_arr = [4u32, 5];
        let mut scale_inputs_arr = [0u32, 2, 3];
        let mut scale_outputs_arr = [13u32];
        let scale_params = OH_NN_UInt32Array {
            data: scale_params_arr.as_mut_ptr(),
            size: 2,
        };
        let scale_inputs = OH_NN_UInt32Array {
            data: scale_inputs_arr.as_mut_ptr(),
            size: 3,
        };
        let scale_outputs = OH_NN_UInt32Array {
            data: scale_outputs_arr.as_mut_ptr(),
            size: 1,
        };
        if OH_NNModel_AddOperation(
            ctx.model,
            OH_NN_OPS_SCALE,
            &scale_params,
            &scale_inputs,
            &scale_outputs,
        ) != OH_NN_SUCCESS
        {
            return false;
        }

        // SELECT: params=[], inputs=[12,13,0], outputs=[1]
        let mut select_inputs_arr = [12u32, 13, 0];
        let mut select_outputs_arr = [1u32];
        let select_params = OH_NN_UInt32Array {
            data: core::ptr::null_mut(),
            size: 0,
        };
        let select_inputs = OH_NN_UInt32Array {
            data: select_inputs_arr.as_mut_ptr(),
            size: 3,
        };
        let select_outputs = OH_NN_UInt32Array {
            data: select_outputs_arr.as_mut_ptr(),
            size: 1,
        };
        if OH_NNModel_AddOperation(
            ctx.model,
            OH_NN_OPS_SELECT,
            &select_params,
            &select_inputs,
            &select_outputs,
        ) != OH_NN_SUCCESS
        {
            return false;
        }

        // Specify inputs/outputs
        let mut model_inputs_arr = [0u32];
        let mut model_outputs_arr = [1u32];
        let model_inputs = OH_NN_UInt32Array {
            data: model_inputs_arr.as_mut_ptr(),
            size: 1,
        };
        let model_outputs = OH_NN_UInt32Array {
            data: model_outputs_arr.as_mut_ptr(),
            size: 1,
        };
        if OH_NNModel_SpecifyInputsAndOutputs(ctx.model, &model_inputs, &model_outputs)
            != OH_NN_SUCCESS
        {
            return false;
        }

        // Finish model
        if OH_NNModel_Finish(ctx.model) != OH_NN_SUCCESS {
            return false;
        }

        // Compile
        ctx.compilation = OH_NNCompilation_Construct(ctx.model);
        if ctx.compilation.is_null() {
            return false;
        }
        if !set_accelerator_device(ctx.compilation) {
            return false;
        }
        if OH_NNCompilation_Build(ctx.compilation) != OH_NN_SUCCESS {
            return false;
        }

        // Create executor
        ctx.executor = OH_NNExecutor_Construct(ctx.compilation);
        if ctx.executor.is_null() {
            return false;
        }

        // Create input/output tensors
        ctx.input_tensor = OH_NNTensor_CreateWithSize(0, desc_input, data_size);
        if ctx.input_tensor.is_null() {
            return false;
        }
        ctx.output_tensor = OH_NNTensor_CreateWithSize(0, desc_out, data_size);
        if ctx.output_tensor.is_null() {
            return false;
        }

        // Fill input data
        let input_buf = OH_NNTensor_GetDataBuffer(ctx.input_tensor) as *mut u8;
        if input_buf.is_null() {
            return false;
        }
        core::ptr::copy_nonoverlapping(
            pixels.as_ptr() as *const u8,
            input_buf,
            data_size,
        );

        // Run inference
        let inputs = [ctx.input_tensor];
        let outputs = [ctx.output_tensor];
        if OH_NNExecutor_RunSync(ctx.executor, inputs.as_ptr(), 1, outputs.as_ptr(), 1)
            != OH_NN_SUCCESS
        {
            return false;
        }

        // Read output data
        let output_buf = OH_NNTensor_GetDataBuffer(ctx.output_tensor) as *mut u8;
        if output_buf.is_null() {
            return false;
        }
        core::ptr::copy_nonoverlapping(
            output_buf,
            pixels.as_mut_ptr() as *mut u8,
            data_size,
        );

        true
    }
}

// === NPU over_background ===
//
// NNR computation graph (reproduces scalar `over_background`):
//   tensor 0:  input (src)         — [1, n, 1, 4] NHWC FLOAT32  (model input)
//   tensor 1:  out                 — [1, n, 1, 4] NHWC FLOAT32  (model output)
//   tensor 2:  bg                  — [4] ND FLOAT32  (per-channel: bg0, bg1, bg2, 0)
//   tensor 3:  one                 — [4] ND FLOAT32  (per-channel: 1, 1, 1, 1)
//   tensor 4:  axis                — [1] ND INT32    (value: 3 = channel dim)
//   tensor 5:  activationType      — [1] ND INT8     (value: 0 = FUSED_NONE)
//   tensor 6:  begin               — [1] ND INT32    (value: 3 = alpha channel)
//   tensor 7:  size                — [1] ND INT32    (value: 1)
//   tensor 8:  bshape              — [4] ND INT32    (value: 1, n, 1, 4)
//   tensor 9:  alpha               — [1, n, 1, 1] NHWC FLOAT32  (SLICE output)
//   tensor 10: a_broadcast         — [1, n, 1, 4] NHWC FLOAT32  (BROADCAST_TO output)
//   tensor 11: one_minus_a         — [1, n, 1, 4] NHWC FLOAT32  (SUB output)
//   tensor 12: bg_broadcast        — [1, n, 1, 4] NHWC FLOAT32  (BROADCAST_TO output)
//   tensor 13: src_mul_a           — [1, n, 1, 4] NHWC FLOAT32  (MUL output)
//   tensor 14: bg_mul_oma          — [1, n, 1, 4] NHWC FLOAT32  (MUL output)
//   tensor 15: rgb_out             — [1, n, 1, 4] NHWC FLOAT32  (ADD output)
//   tensor 16: scale               — [4] ND FLOAT32  (per-channel: 1, 1, 1, 0)
//   tensor 17: bias                — [4] ND FLOAT32  (per-channel: 0, 0, 0, 1)
//
// Operations:
//   SLICE:         params=[4],      inputs=[0, 6, 7],    outputs=[9]   alpha = src[..., 3:4]
//   BROADCAST_TO:  params=[8],      inputs=[9],          outputs=[10]  a_b = broadcast(alpha, [1,n,1,4])
//   SUB:           params=[5],      inputs=[3, 10],      outputs=[11]  one_minus_a = one - a_b
//   BROADCAST_TO:  params=[8],      inputs=[2],          outputs=[12]  bg_b = broadcast(bg, [1,n,1,4])
//   MUL:           params=[5],      inputs=[0, 10],      outputs=[13]  src_mul_a = src * a_b
//   MUL:           params=[5],      inputs=[12, 11],     outputs=[14]  bg_mul_oma = bg_b * one_minus_a
//   ADD:           params=[5],      inputs=[13, 14],     outputs=[15]  rgb_out = src_mul_a + bg_mul_oma
//   SCALE:         params=[4, 5],   inputs=[15, 16, 17], outputs=[1]   out = SCALE(rgb_out, scale, bias, axis=3)
//
// Semantics: out[k] = src[k]*a + bg[k]*(1-a) for k=0..2; out[3] = 1.0.
// SCALE step: out[0..3] = rgb_out[k]*scale[k] + bias[k] = rgb_out[k]*1 + 0 = rgb_out[k] for k=0..2;
//             out[3] = rgb_out[3]*0 + 1 = 1.0.
// Bit-identical to scalar `over_background`: every op is a single IEEE-754 f32 mul/add/sub,
// same op order (mul → sub → mul → mul → add → scale-mul+scale-bias).

/// Attempt NPU execution of `over_background`. Returns `true` on success,
/// `false` if NPU is unavailable or any NNR call fails (caller falls back).
pub unsafe fn npu_over_background(pixels: &mut [[f32; 4]], bg: [f32; 3]) -> bool {
    unsafe {
        let n = pixels.len();
        if n == 0 {
            return true;
        }

        let shape: [i32; 4] = [1, n as i32, 1, 4];
        let data_size = n * 4 * 4; // n pixels * 4 channels * 4 bytes per f32

        let mut ctx = NpuCtx::new();

        // Create model
        ctx.model = OH_NNModel_Construct();
        if ctx.model.is_null() {
            return false;
        }

        // Create tensor descs (indices 0..17)
        let desc_input = match create_desc(OH_NN_FLOAT32, &shape, OH_NN_FORMAT_NHWC) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_input);

        let desc_out = match create_desc(OH_NN_FLOAT32, &shape, OH_NN_FORMAT_NHWC) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_out);

        let channel_shape: [i32; 1] = [4];
        let scalar_shape: [i32; 1] = [1];
        let bshape: [i32; 4] = [1, n as i32, 1, 4];
        let alpha_shape: [i32; 4] = [1, n as i32, 1, 1];

        let desc_bg = match create_desc(OH_NN_FLOAT32, &channel_shape, OH_NN_FORMAT_ND) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_bg);

        let desc_one = match create_desc(OH_NN_FLOAT32, &channel_shape, OH_NN_FORMAT_ND) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_one);

        let desc_axis = match create_desc(OH_NN_INT32, &scalar_shape, OH_NN_FORMAT_ND) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_axis);

        let desc_act = match create_desc(OH_NN_INT8, &scalar_shape, OH_NN_FORMAT_ND) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_act);

        let desc_begin = match create_desc(OH_NN_INT32, &scalar_shape, OH_NN_FORMAT_ND) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_begin);

        let desc_size = match create_desc(OH_NN_INT32, &scalar_shape, OH_NN_FORMAT_ND) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_size);

        let desc_bshape = match create_desc(OH_NN_INT32, &bshape, OH_NN_FORMAT_ND) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_bshape);

        let desc_alpha = match create_desc(OH_NN_FLOAT32, &alpha_shape, OH_NN_FORMAT_NHWC) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_alpha);

        let desc_a_b = match create_desc(OH_NN_FLOAT32, &shape, OH_NN_FORMAT_NHWC) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_a_b);

        let desc_oma = match create_desc(OH_NN_FLOAT32, &shape, OH_NN_FORMAT_NHWC) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_oma);

        let desc_bg_b = match create_desc(OH_NN_FLOAT32, &shape, OH_NN_FORMAT_NHWC) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_bg_b);

        let desc_sma = match create_desc(OH_NN_FLOAT32, &shape, OH_NN_FORMAT_NHWC) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_sma);

        let desc_bma = match create_desc(OH_NN_FLOAT32, &shape, OH_NN_FORMAT_NHWC) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_bma);

        let desc_rgb = match create_desc(OH_NN_FLOAT32, &shape, OH_NN_FORMAT_NHWC) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_rgb);

        let desc_scale = match create_desc(OH_NN_FLOAT32, &channel_shape, OH_NN_FORMAT_ND) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_scale);

        let desc_bias = match create_desc(OH_NN_FLOAT32, &channel_shape, OH_NN_FORMAT_ND) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_bias);

        // Add all descs to model (indices 0..17)
        for i in 0..ctx.desc_count {
            if OH_NNModel_AddTensorToModel(ctx.model, ctx.descs[i]) != OH_NN_SUCCESS {
                return false;
            }
        }

        // Set constant tensor data
        // bg: [bg0, bg1, bg2, 0] — alpha channel bg=0 (scalar sets p[3]=1.0 directly)
        let bg_data = [bg[0], bg[1], bg[2], 0.0f32];
        if OH_NNModel_SetTensorData(ctx.model, 2, bg_data.as_ptr() as *const c_void, 16)
            != OH_NN_SUCCESS
        {
            return false;
        }

        let one_data = [1.0f32; 4];
        if OH_NNModel_SetTensorData(ctx.model, 3, one_data.as_ptr() as *const c_void, 16)
            != OH_NN_SUCCESS
        {
            return false;
        }

        let axis_data = [3i32];
        if OH_NNModel_SetTensorData(ctx.model, 4, axis_data.as_ptr() as *const c_void, 4)
            != OH_NN_SUCCESS
        {
            return false;
        }

        let act_data = [0i8];
        if OH_NNModel_SetTensorData(ctx.model, 5, act_data.as_ptr() as *const c_void, 1)
            != OH_NN_SUCCESS
        {
            return false;
        }

        let begin_data = [3i32];
        if OH_NNModel_SetTensorData(ctx.model, 6, begin_data.as_ptr() as *const c_void, 4)
            != OH_NN_SUCCESS
        {
            return false;
        }

        let size_data = [1i32];
        if OH_NNModel_SetTensorData(ctx.model, 7, size_data.as_ptr() as *const c_void, 4)
            != OH_NN_SUCCESS
        {
            return false;
        }

        let bshape_data = [1i32, n as i32, 1, 4];
        if OH_NNModel_SetTensorData(ctx.model, 8, bshape_data.as_ptr() as *const c_void, 16)
            != OH_NN_SUCCESS
        {
            return false;
        }

        // scale: [1,1,1,0] — alpha channel scale=0 (so rgb_out[3]*0 = 0, then +bias[3]=1)
        let scale_data = [1.0f32, 1.0, 1.0, 0.0];
        if OH_NNModel_SetTensorData(ctx.model, 16, scale_data.as_ptr() as *const c_void, 16)
            != OH_NN_SUCCESS
        {
            return false;
        }

        // bias: [0,0,0,1] — alpha channel bias=1 (so out[3] = 0 + 1 = 1.0)
        let bias_data = [0.0f32, 0.0, 0.0, 1.0];
        if OH_NNModel_SetTensorData(ctx.model, 17, bias_data.as_ptr() as *const c_void, 16)
            != OH_NN_SUCCESS
        {
            return false;
        }

        // Add operations
        // SLICE: params=[4], inputs=[0,6,7], outputs=[9]
        let mut slice_params_arr = [4u32];
        let mut slice_inputs_arr = [0u32, 6, 7];
        let mut slice_outputs_arr = [9u32];
        let slice_params = OH_NN_UInt32Array {
            data: slice_params_arr.as_mut_ptr(),
            size: 1,
        };
        let slice_inputs = OH_NN_UInt32Array {
            data: slice_inputs_arr.as_mut_ptr(),
            size: 3,
        };
        let slice_outputs = OH_NN_UInt32Array {
            data: slice_outputs_arr.as_mut_ptr(),
            size: 1,
        };
        if OH_NNModel_AddOperation(
            ctx.model,
            OH_NN_OPS_SLICE,
            &slice_params,
            &slice_inputs,
            &slice_outputs,
        ) != OH_NN_SUCCESS
        {
            return false;
        }

        // BROADCAST_TO: params=[8], inputs=[9], outputs=[10]
        let mut abcast_params_arr = [8u32];
        let mut abcast_inputs_arr = [9u32];
        let mut abcast_outputs_arr = [10u32];
        let abcast_params = OH_NN_UInt32Array {
            data: abcast_params_arr.as_mut_ptr(),
            size: 1,
        };
        let abcast_inputs = OH_NN_UInt32Array {
            data: abcast_inputs_arr.as_mut_ptr(),
            size: 1,
        };
        let abcast_outputs = OH_NN_UInt32Array {
            data: abcast_outputs_arr.as_mut_ptr(),
            size: 1,
        };
        if OH_NNModel_AddOperation(
            ctx.model,
            OH_NN_OPS_BROADCAST_TO,
            &abcast_params,
            &abcast_inputs,
            &abcast_outputs,
        ) != OH_NN_SUCCESS
        {
            return false;
        }

        // SUB: params=[5], inputs=[3,10], outputs=[11]
        let mut sub_params_arr = [5u32];
        let mut sub_inputs_arr = [3u32, 10];
        let mut sub_outputs_arr = [11u32];
        let sub_params = OH_NN_UInt32Array {
            data: sub_params_arr.as_mut_ptr(),
            size: 1,
        };
        let sub_inputs = OH_NN_UInt32Array {
            data: sub_inputs_arr.as_mut_ptr(),
            size: 2,
        };
        let sub_outputs = OH_NN_UInt32Array {
            data: sub_outputs_arr.as_mut_ptr(),
            size: 1,
        };
        if OH_NNModel_AddOperation(
            ctx.model,
            OH_NN_OPS_SUB,
            &sub_params,
            &sub_inputs,
            &sub_outputs,
        ) != OH_NN_SUCCESS
        {
            return false;
        }

        // BROADCAST_TO: params=[8], inputs=[2], outputs=[12]
        let mut bbcast_params_arr = [8u32];
        let mut bbcast_inputs_arr = [2u32];
        let mut bbcast_outputs_arr = [12u32];
        let bbcast_params = OH_NN_UInt32Array {
            data: bbcast_params_arr.as_mut_ptr(),
            size: 1,
        };
        let bbcast_inputs = OH_NN_UInt32Array {
            data: bbcast_inputs_arr.as_mut_ptr(),
            size: 1,
        };
        let bbcast_outputs = OH_NN_UInt32Array {
            data: bbcast_outputs_arr.as_mut_ptr(),
            size: 1,
        };
        if OH_NNModel_AddOperation(
            ctx.model,
            OH_NN_OPS_BROADCAST_TO,
            &bbcast_params,
            &bbcast_inputs,
            &bbcast_outputs,
        ) != OH_NN_SUCCESS
        {
            return false;
        }

        // MUL: params=[5], inputs=[0,10], outputs=[13]
        let mut mul1_params_arr = [5u32];
        let mut mul1_inputs_arr = [0u32, 10];
        let mut mul1_outputs_arr = [13u32];
        let mul1_params = OH_NN_UInt32Array {
            data: mul1_params_arr.as_mut_ptr(),
            size: 1,
        };
        let mul1_inputs = OH_NN_UInt32Array {
            data: mul1_inputs_arr.as_mut_ptr(),
            size: 2,
        };
        let mul1_outputs = OH_NN_UInt32Array {
            data: mul1_outputs_arr.as_mut_ptr(),
            size: 1,
        };
        if OH_NNModel_AddOperation(
            ctx.model,
            OH_NN_OPS_MUL,
            &mul1_params,
            &mul1_inputs,
            &mul1_outputs,
        ) != OH_NN_SUCCESS
        {
            return false;
        }

        // MUL: params=[5], inputs=[12,11], outputs=[14]
        let mut mul2_params_arr = [5u32];
        let mut mul2_inputs_arr = [12u32, 11];
        let mut mul2_outputs_arr = [14u32];
        let mul2_params = OH_NN_UInt32Array {
            data: mul2_params_arr.as_mut_ptr(),
            size: 1,
        };
        let mul2_inputs = OH_NN_UInt32Array {
            data: mul2_inputs_arr.as_mut_ptr(),
            size: 2,
        };
        let mul2_outputs = OH_NN_UInt32Array {
            data: mul2_outputs_arr.as_mut_ptr(),
            size: 1,
        };
        if OH_NNModel_AddOperation(
            ctx.model,
            OH_NN_OPS_MUL,
            &mul2_params,
            &mul2_inputs,
            &mul2_outputs,
        ) != OH_NN_SUCCESS
        {
            return false;
        }

        // ADD: params=[5], inputs=[13,14], outputs=[15]
        let mut add_params_arr = [5u32];
        let mut add_inputs_arr = [13u32, 14];
        let mut add_outputs_arr = [15u32];
        let add_params = OH_NN_UInt32Array {
            data: add_params_arr.as_mut_ptr(),
            size: 1,
        };
        let add_inputs = OH_NN_UInt32Array {
            data: add_inputs_arr.as_mut_ptr(),
            size: 2,
        };
        let add_outputs = OH_NN_UInt32Array {
            data: add_outputs_arr.as_mut_ptr(),
            size: 1,
        };
        if OH_NNModel_AddOperation(
            ctx.model,
            OH_NN_OPS_ADD,
            &add_params,
            &add_inputs,
            &add_outputs,
        ) != OH_NN_SUCCESS
        {
            return false;
        }

        // SCALE: params=[4,5], inputs=[15,16,17], outputs=[1]
        let mut scale_params_arr = [4u32, 5];
        let mut scale_inputs_arr = [15u32, 16, 17];
        let mut scale_outputs_arr = [1u32];
        let scale_params = OH_NN_UInt32Array {
            data: scale_params_arr.as_mut_ptr(),
            size: 2,
        };
        let scale_inputs = OH_NN_UInt32Array {
            data: scale_inputs_arr.as_mut_ptr(),
            size: 3,
        };
        let scale_outputs = OH_NN_UInt32Array {
            data: scale_outputs_arr.as_mut_ptr(),
            size: 1,
        };
        if OH_NNModel_AddOperation(
            ctx.model,
            OH_NN_OPS_SCALE,
            &scale_params,
            &scale_inputs,
            &scale_outputs,
        ) != OH_NN_SUCCESS
        {
            return false;
        }

        // Specify inputs/outputs
        let mut model_inputs_arr = [0u32];
        let mut model_outputs_arr = [1u32];
        let model_inputs = OH_NN_UInt32Array {
            data: model_inputs_arr.as_mut_ptr(),
            size: 1,
        };
        let model_outputs = OH_NN_UInt32Array {
            data: model_outputs_arr.as_mut_ptr(),
            size: 1,
        };
        if OH_NNModel_SpecifyInputsAndOutputs(ctx.model, &model_inputs, &model_outputs)
            != OH_NN_SUCCESS
        {
            return false;
        }

        // Finish model
        if OH_NNModel_Finish(ctx.model) != OH_NN_SUCCESS {
            return false;
        }

        // Compile
        ctx.compilation = OH_NNCompilation_Construct(ctx.model);
        if ctx.compilation.is_null() {
            return false;
        }
        if !set_accelerator_device(ctx.compilation) {
            return false;
        }
        if OH_NNCompilation_Build(ctx.compilation) != OH_NN_SUCCESS {
            return false;
        }

        // Create executor
        ctx.executor = OH_NNExecutor_Construct(ctx.compilation);
        if ctx.executor.is_null() {
            return false;
        }

        // Create input/output tensors
        ctx.input_tensor = OH_NNTensor_CreateWithSize(0, desc_input, data_size);
        if ctx.input_tensor.is_null() {
            return false;
        }
        ctx.output_tensor = OH_NNTensor_CreateWithSize(0, desc_out, data_size);
        if ctx.output_tensor.is_null() {
            return false;
        }

        // Fill input data
        let input_buf = OH_NNTensor_GetDataBuffer(ctx.input_tensor) as *mut u8;
        if input_buf.is_null() {
            return false;
        }
        core::ptr::copy_nonoverlapping(
            pixels.as_ptr() as *const u8,
            input_buf,
            data_size,
        );

        // Run inference
        let inputs = [ctx.input_tensor];
        let outputs = [ctx.output_tensor];
        if OH_NNExecutor_RunSync(ctx.executor, inputs.as_ptr(), 1, outputs.as_ptr(), 1)
            != OH_NN_SUCCESS
        {
            return false;
        }

        // Read output data
        let output_buf = OH_NNTensor_GetDataBuffer(ctx.output_tensor) as *mut u8;
        if output_buf.is_null() {
            return false;
        }
        core::ptr::copy_nonoverlapping(
            output_buf,
            pixels.as_mut_ptr() as *mut u8,
            data_size,
        );

        true
    }
}
// === NPU channel_mixer ===
//
// NNR computation graph (reproduces scalar `channel_mixer`):
//   tensor 0:  input (src)         — [1, n, 1, 4] NHWC FLOAT32  (model input)
//   tensor 1:  out                 — [1, n, 1, 4] NHWC FLOAT32  (model output)
//   tensor 2:  weights             — [3, 3] ND FLOAT32  (first 3 elements of each matrix row)
//   tensor 3:  bias                — [3] ND FLOAT32  (4th element of each matrix row)
//   tensor 4:  tx                  — [1] ND INT8     (value: 0 = TransposeX false)
//   tensor 5:  ty                  — [1] ND INT8     (value: 1 = TransposeY true)
//   tensor 6:  activationType      — [1] ND INT8     (value: 0 = FUSED_NONE)
//   tensor 7:  axis                — [1] ND INT32    (value: 3 = channel dim)
//   tensor 8:  rgb_begin           — [1] ND INT32    (value: 0)
//   tensor 9:  rgb_size            — [1] ND INT32    (value: 3)
//   tensor 10: alpha_begin         — [1] ND INT32    (value: 3)
//   tensor 11: alpha_size          — [1] ND INT32    (value: 1)
//   tensor 12: bshape              — [4] ND INT32    (value: 1, n, 1, 4)
//   tensor 13: concat_axis         — [1] ND INT32    (value: 3)
//   tensor 14: zero                — [1] ND FLOAT32  (value: 0.0)
//   tensor 15: one                 — [1] ND FLOAT32  (value: 1.0)
//   tensor 16: alpha               — [1, n, 1, 1] NHWC FLOAT32  (SLICE output)
//   tensor 17: mask                — [1, n, 1, 1] NHWC BOOL     (GREATER output)
//   tensor 18: mask_broadcast      — [1, n, 1, 4] NHWC BOOL     (BROADCAST_TO output)
//   tensor 19: rgb                 — [1, n, 1, 3] NHWC FLOAT32  (SLICE output)
//   tensor 20: matmul_out          — [1, n, 1, 3] NHWC FLOAT32  (MATMUL output)
//   tensor 21: bias_out            — [1, n, 1, 3] NHWC FLOAT32  (BIAS_ADD output)
//   tensor 22: clamped_lo          — [1, n, 1, 3] NHWC FLOAT32  (MAXIMUM output)
//   tensor 23: clipped             — [1, n, 1, 3] NHWC FLOAT32  (MINIMUM output)
//   tensor 24: result              — [1, n, 1, 4] NHWC FLOAT32  (CONCAT output)
//
// Operations:
//   SLICE:       params=[7],       inputs=[0, 10, 11], outputs=[16]  alpha = src[..., 3:4]
//   GREATER:     params=[],        inputs=[16, 14],    outputs=[17]  mask = alpha > 0
//   BROADCAST_TO: params=[12],     inputs=[17],        outputs=[18]  mask_b = broadcast(mask, [1,n,1,4])
//   SLICE:       params=[7],       inputs=[0, 8, 9],   outputs=[19]  rgb = src[..., 0:3]
//   MATMUL:      params=[4, 5, 6], inputs=[19, 2],     outputs=[20]  matmul_out = rgb @ weights^T
//   BIAS_ADD:    params=[],        inputs=[20, 3],     outputs=[21]  bias_out = matmul_out + bias
//   MAXIMUM:     params=[],        inputs=[21, 14],    outputs=[22]  clamped_lo = max(bias_out, 0)
//   MINIMUM:     params=[],        inputs=[22, 15],    outputs=[23]  clipped = min(clamped_lo, 1)
//   CONCAT:      params=[13],      inputs=[23, 16],    outputs=[24]  result = concat([clipped, alpha], axis=3)
//   SELECT:      params=[],        inputs=[18, 24, 0], outputs=[1]   out = where(mask_b, result, src)
//
// Semantics: out[i] = (matrix[i][0]*c[0] + matrix[i][1]*c[1] + matrix[i][2]*c[2] + matrix[i][3]).clamp(0.0, 1.0)
// for i in 0..3, alpha preserved. Where alpha <= 0 → original pixel (skip).
// monochrome: weights rows 1/2 = row 0, bias = [m03, m03, m03] → all 3 outputs use matrix[0].
// Bit-identical to scalar `channel_mixer`: every op is single IEEE-754 f32 mul/add, same op order.

/// Attempt NPU execution of `channel_mixer`. Returns `true` on success,
/// `false` if NPU is unavailable or any NNR call fails (caller falls back).
pub unsafe fn npu_channel_mixer(pixels: &mut [[f32; 4]], matrix: [[f32; 4]; 3], monochrome: bool) -> bool {
    unsafe {
        let n = pixels.len();
        if n == 0 {
            return true;
        }

        let shape: [i32; 4] = [1, n as i32, 1, 4];
        let data_size = n * 4 * 4;

        let mut ctx = NpuCtx::new();

        // Create model
        ctx.model = OH_NNModel_Construct();
        if ctx.model.is_null() {
            return false;
        }

        // Create tensor descs (indices 0..24)
        let desc_input = match create_desc(OH_NN_FLOAT32, &shape, OH_NN_FORMAT_NHWC) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_input);

        let desc_out = match create_desc(OH_NN_FLOAT32, &shape, OH_NN_FORMAT_NHWC) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_out);

        let weights_shape: [i32; 2] = [3, 3];
        let channel_shape: [i32; 1] = [3];
        let scalar_shape: [i32; 1] = [1];
        let bshape: [i32; 4] = [1, n as i32, 1, 4];
        let alpha_shape: [i32; 4] = [1, n as i32, 1, 1];
        let rgb_shape: [i32; 4] = [1, n as i32, 1, 3];

        let desc_weights = match create_desc(OH_NN_FLOAT32, &weights_shape, OH_NN_FORMAT_ND) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_weights);

        let desc_bias = match create_desc(OH_NN_FLOAT32, &channel_shape, OH_NN_FORMAT_ND) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_bias);

        let desc_tx = match create_desc(OH_NN_INT8, &scalar_shape, OH_NN_FORMAT_ND) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_tx);

        let desc_ty = match create_desc(OH_NN_INT8, &scalar_shape, OH_NN_FORMAT_ND) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_ty);

        let desc_act = match create_desc(OH_NN_INT8, &scalar_shape, OH_NN_FORMAT_ND) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_act);

        let desc_axis = match create_desc(OH_NN_INT32, &scalar_shape, OH_NN_FORMAT_ND) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_axis);

        let desc_rgb_begin = match create_desc(OH_NN_INT32, &scalar_shape, OH_NN_FORMAT_ND) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_rgb_begin);

        let desc_rgb_size = match create_desc(OH_NN_INT32, &scalar_shape, OH_NN_FORMAT_ND) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_rgb_size);

        let desc_alpha_begin = match create_desc(OH_NN_INT32, &scalar_shape, OH_NN_FORMAT_ND) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_alpha_begin);

        let desc_alpha_size = match create_desc(OH_NN_INT32, &scalar_shape, OH_NN_FORMAT_ND) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_alpha_size);

        let desc_bshape = match create_desc(OH_NN_INT32, &bshape, OH_NN_FORMAT_ND) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_bshape);

        let desc_concat_axis = match create_desc(OH_NN_INT32, &scalar_shape, OH_NN_FORMAT_ND) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_concat_axis);

        let desc_zero = match create_desc(OH_NN_FLOAT32, &scalar_shape, OH_NN_FORMAT_ND) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_zero);

        let desc_one = match create_desc(OH_NN_FLOAT32, &scalar_shape, OH_NN_FORMAT_ND) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_one);

        let desc_alpha = match create_desc(OH_NN_FLOAT32, &alpha_shape, OH_NN_FORMAT_NHWC) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_alpha);

        let desc_mask = match create_desc(OH_NN_BOOL, &alpha_shape, OH_NN_FORMAT_NHWC) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_mask);

        let desc_mask_b = match create_desc(OH_NN_BOOL, &shape, OH_NN_FORMAT_NHWC) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_mask_b);

        let desc_rgb = match create_desc(OH_NN_FLOAT32, &rgb_shape, OH_NN_FORMAT_NHWC) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_rgb);

        let desc_matmul_out = match create_desc(OH_NN_FLOAT32, &rgb_shape, OH_NN_FORMAT_NHWC) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_matmul_out);

        let desc_bias_out = match create_desc(OH_NN_FLOAT32, &rgb_shape, OH_NN_FORMAT_NHWC) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_bias_out);

        let desc_clamped_lo = match create_desc(OH_NN_FLOAT32, &rgb_shape, OH_NN_FORMAT_NHWC) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_clamped_lo);

        let desc_clipped = match create_desc(OH_NN_FLOAT32, &rgb_shape, OH_NN_FORMAT_NHWC) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_clipped);

        let desc_result = match create_desc(OH_NN_FLOAT32, &shape, OH_NN_FORMAT_NHWC) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_result);

        // Add all descs to model (indices 0..24)
        for i in 0..ctx.desc_count {
            if OH_NNModel_AddTensorToModel(ctx.model, ctx.descs[i]) != OH_NN_SUCCESS {
                return false;
            }
        }

        // Set constant tensor data
        // weights: first 3 elements of each matrix row, row-major [3,3]
        let weights_data = if monochrome {
            [matrix[0][0], matrix[0][1], matrix[0][2],
             matrix[0][0], matrix[0][1], matrix[0][2],
             matrix[0][0], matrix[0][1], matrix[0][2]]
        } else {
            [matrix[0][0], matrix[0][1], matrix[0][2],
             matrix[1][0], matrix[1][1], matrix[1][2],
             matrix[2][0], matrix[2][1], matrix[2][2]]
        };
        if OH_NNModel_SetTensorData(ctx.model, 2, weights_data.as_ptr() as *const c_void, 36)
            != OH_NN_SUCCESS
        {
            return false;
        }

        // bias: 4th element of each matrix row
        let bias_data = if monochrome {
            [matrix[0][3], matrix[0][3], matrix[0][3]]
        } else {
            [matrix[0][3], matrix[1][3], matrix[2][3]]
        };
        if OH_NNModel_SetTensorData(ctx.model, 3, bias_data.as_ptr() as *const c_void, 12)
            != OH_NN_SUCCESS
        {
            return false;
        }

        let tx_data = [0i8];
        if OH_NNModel_SetTensorData(ctx.model, 4, tx_data.as_ptr() as *const c_void, 1)
            != OH_NN_SUCCESS
        {
            return false;
        }

        let ty_data = [1i8];
        if OH_NNModel_SetTensorData(ctx.model, 5, ty_data.as_ptr() as *const c_void, 1)
            != OH_NN_SUCCESS
        {
            return false;
        }

        let act_data = [0i8];
        if OH_NNModel_SetTensorData(ctx.model, 6, act_data.as_ptr() as *const c_void, 1)
            != OH_NN_SUCCESS
        {
            return false;
        }

        let axis_data = [3i32];
        if OH_NNModel_SetTensorData(ctx.model, 7, axis_data.as_ptr() as *const c_void, 4)
            != OH_NN_SUCCESS
        {
            return false;
        }

        let rgb_begin_data = [0i32];
        if OH_NNModel_SetTensorData(ctx.model, 8, rgb_begin_data.as_ptr() as *const c_void, 4)
            != OH_NN_SUCCESS
        {
            return false;
        }

        let rgb_size_data = [3i32];
        if OH_NNModel_SetTensorData(ctx.model, 9, rgb_size_data.as_ptr() as *const c_void, 4)
            != OH_NN_SUCCESS
        {
            return false;
        }

        let alpha_begin_data = [3i32];
        if OH_NNModel_SetTensorData(ctx.model, 10, alpha_begin_data.as_ptr() as *const c_void, 4)
            != OH_NN_SUCCESS
        {
            return false;
        }

        let alpha_size_data = [1i32];
        if OH_NNModel_SetTensorData(ctx.model, 11, alpha_size_data.as_ptr() as *const c_void, 4)
            != OH_NN_SUCCESS
        {
            return false;
        }

        let bshape_data = [1i32, n as i32, 1, 4];
        if OH_NNModel_SetTensorData(ctx.model, 12, bshape_data.as_ptr() as *const c_void, 16)
            != OH_NN_SUCCESS
        {
            return false;
        }

        let concat_axis_data = [3i32];
        if OH_NNModel_SetTensorData(ctx.model, 13, concat_axis_data.as_ptr() as *const c_void, 4)
            != OH_NN_SUCCESS
        {
            return false;
        }

        let zero_data = [0.0f32];
        if OH_NNModel_SetTensorData(ctx.model, 14, zero_data.as_ptr() as *const c_void, 4)
            != OH_NN_SUCCESS
        {
            return false;
        }

        let one_data = [1.0f32];
        if OH_NNModel_SetTensorData(ctx.model, 15, one_data.as_ptr() as *const c_void, 4)
            != OH_NN_SUCCESS
        {
            return false;
        }

        // Add operations
        // SLICE (alpha): params=[7], inputs=[0,10,11], outputs=[16]
        let mut s1_params_arr = [7u32];
        let mut s1_inputs_arr = [0u32, 10, 11];
        let mut s1_outputs_arr = [16u32];
        let s1_params = OH_NN_UInt32Array {
            data: s1_params_arr.as_mut_ptr(),
            size: 1,
        };
        let s1_inputs = OH_NN_UInt32Array {
            data: s1_inputs_arr.as_mut_ptr(),
            size: 3,
        };
        let s1_outputs = OH_NN_UInt32Array {
            data: s1_outputs_arr.as_mut_ptr(),
            size: 1,
        };
        if OH_NNModel_AddOperation(
            ctx.model,
            OH_NN_OPS_SLICE,
            &s1_params,
            &s1_inputs,
            &s1_outputs,
        ) != OH_NN_SUCCESS
        {
            return false;
        }

        // GREATER: params=[], inputs=[16,14], outputs=[17]
        let mut gt_inputs_arr = [16u32, 14];
        let mut gt_outputs_arr = [17u32];
        let gt_params = OH_NN_UInt32Array {
            data: core::ptr::null_mut(),
            size: 0,
        };
        let gt_inputs = OH_NN_UInt32Array {
            data: gt_inputs_arr.as_mut_ptr(),
            size: 2,
        };
        let gt_outputs = OH_NN_UInt32Array {
            data: gt_outputs_arr.as_mut_ptr(),
            size: 1,
        };
        if OH_NNModel_AddOperation(
            ctx.model,
            OH_NN_OPS_GREATER,
            &gt_params,
            &gt_inputs,
            &gt_outputs,
        ) != OH_NN_SUCCESS
        {
            return false;
        }

        // BROADCAST_TO: params=[12], inputs=[17], outputs=[18]
        let mut bc_params_arr = [12u32];
        let mut bc_inputs_arr = [17u32];
        let mut bc_outputs_arr = [18u32];
        let bc_params = OH_NN_UInt32Array {
            data: bc_params_arr.as_mut_ptr(),
            size: 1,
        };
        let bc_inputs = OH_NN_UInt32Array {
            data: bc_inputs_arr.as_mut_ptr(),
            size: 1,
        };
        let bc_outputs = OH_NN_UInt32Array {
            data: bc_outputs_arr.as_mut_ptr(),
            size: 1,
        };
        if OH_NNModel_AddOperation(
            ctx.model,
            OH_NN_OPS_BROADCAST_TO,
            &bc_params,
            &bc_inputs,
            &bc_outputs,
        ) != OH_NN_SUCCESS
        {
            return false;
        }

        // SLICE (RGB): params=[7], inputs=[0,8,9], outputs=[19]
        let mut s2_params_arr = [7u32];
        let mut s2_inputs_arr = [0u32, 8, 9];
        let mut s2_outputs_arr = [19u32];
        let s2_params = OH_NN_UInt32Array {
            data: s2_params_arr.as_mut_ptr(),
            size: 1,
        };
        let s2_inputs = OH_NN_UInt32Array {
            data: s2_inputs_arr.as_mut_ptr(),
            size: 3,
        };
        let s2_outputs = OH_NN_UInt32Array {
            data: s2_outputs_arr.as_mut_ptr(),
            size: 1,
        };
        if OH_NNModel_AddOperation(
            ctx.model,
            OH_NN_OPS_SLICE,
            &s2_params,
            &s2_inputs,
            &s2_outputs,
        ) != OH_NN_SUCCESS
        {
            return false;
        }

        // MATMUL: params=[4,5,6], inputs=[19,2], outputs=[20]
        let mut mm_params_arr = [4u32, 5, 6];
        let mut mm_inputs_arr = [19u32, 2];
        let mut mm_outputs_arr = [20u32];
        let mm_params = OH_NN_UInt32Array {
            data: mm_params_arr.as_mut_ptr(),
            size: 3,
        };
        let mm_inputs = OH_NN_UInt32Array {
            data: mm_inputs_arr.as_mut_ptr(),
            size: 2,
        };
        let mm_outputs = OH_NN_UInt32Array {
            data: mm_outputs_arr.as_mut_ptr(),
            size: 1,
        };
        if OH_NNModel_AddOperation(
            ctx.model,
            OH_NN_OPS_MATMUL,
            &mm_params,
            &mm_inputs,
            &mm_outputs,
        ) != OH_NN_SUCCESS
        {
            return false;
        }

        // BIAS_ADD: params=[], inputs=[20,3], outputs=[21]
        let mut ba_inputs_arr = [20u32, 3];
        let mut ba_outputs_arr = [21u32];
        let ba_params = OH_NN_UInt32Array {
            data: core::ptr::null_mut(),
            size: 0,
        };
        let ba_inputs = OH_NN_UInt32Array {
            data: ba_inputs_arr.as_mut_ptr(),
            size: 2,
        };
        let ba_outputs = OH_NN_UInt32Array {
            data: ba_outputs_arr.as_mut_ptr(),
            size: 1,
        };
        if OH_NNModel_AddOperation(
            ctx.model,
            OH_NN_OPS_BIAS_ADD,
            &ba_params,
            &ba_inputs,
            &ba_outputs,
        ) != OH_NN_SUCCESS
        {
            return false;
        }

        // MAXIMUM: params=[], inputs=[21,14], outputs=[22]
        let mut mx_inputs_arr = [21u32, 14];
        let mut mx_outputs_arr = [22u32];
        let mx_params = OH_NN_UInt32Array {
            data: core::ptr::null_mut(),
            size: 0,
        };
        let mx_inputs = OH_NN_UInt32Array {
            data: mx_inputs_arr.as_mut_ptr(),
            size: 2,
        };
        let mx_outputs = OH_NN_UInt32Array {
            data: mx_outputs_arr.as_mut_ptr(),
            size: 1,
        };
        if OH_NNModel_AddOperation(
            ctx.model,
            OH_NN_OPS_MAXIMUM,
            &mx_params,
            &mx_inputs,
            &mx_outputs,
        ) != OH_NN_SUCCESS
        {
            return false;
        }

        // MINIMUM: params=[], inputs=[22,15], outputs=[23]
        let mut mn_inputs_arr = [22u32, 15];
        let mut mn_outputs_arr = [23u32];
        let mn_params = OH_NN_UInt32Array {
            data: core::ptr::null_mut(),
            size: 0,
        };
        let mn_inputs = OH_NN_UInt32Array {
            data: mn_inputs_arr.as_mut_ptr(),
            size: 2,
        };
        let mn_outputs = OH_NN_UInt32Array {
            data: mn_outputs_arr.as_mut_ptr(),
            size: 1,
        };
        if OH_NNModel_AddOperation(
            ctx.model,
            OH_NN_OPS_MINIMUM,
            &mn_params,
            &mn_inputs,
            &mn_outputs,
        ) != OH_NN_SUCCESS
        {
            return false;
        }

        // CONCAT: params=[13], inputs=[23,16], outputs=[24]
        let mut cc_params_arr = [13u32];
        let mut cc_inputs_arr = [23u32, 16];
        let mut cc_outputs_arr = [24u32];
        let cc_params = OH_NN_UInt32Array {
            data: cc_params_arr.as_mut_ptr(),
            size: 1,
        };
        let cc_inputs = OH_NN_UInt32Array {
            data: cc_inputs_arr.as_mut_ptr(),
            size: 2,
        };
        let cc_outputs = OH_NN_UInt32Array {
            data: cc_outputs_arr.as_mut_ptr(),
            size: 1,
        };
        if OH_NNModel_AddOperation(
            ctx.model,
            OH_NN_OPS_CONCAT,
            &cc_params,
            &cc_inputs,
            &cc_outputs,
        ) != OH_NN_SUCCESS
        {
            return false;
        }

        // SELECT: params=[], inputs=[18,24,0], outputs=[1]
        let mut sl_inputs_arr = [18u32, 24, 0];
        let mut sl_outputs_arr = [1u32];
        let sl_params = OH_NN_UInt32Array {
            data: core::ptr::null_mut(),
            size: 0,
        };
        let sl_inputs = OH_NN_UInt32Array {
            data: sl_inputs_arr.as_mut_ptr(),
            size: 3,
        };
        let sl_outputs = OH_NN_UInt32Array {
            data: sl_outputs_arr.as_mut_ptr(),
            size: 1,
        };
        if OH_NNModel_AddOperation(
            ctx.model,
            OH_NN_OPS_SELECT,
            &sl_params,
            &sl_inputs,
            &sl_outputs,
        ) != OH_NN_SUCCESS
        {
            return false;
        }

        // Specify inputs/outputs
        let mut model_inputs_arr = [0u32];
        let mut model_outputs_arr = [1u32];
        let model_inputs = OH_NN_UInt32Array {
            data: model_inputs_arr.as_mut_ptr(),
            size: 1,
        };
        let model_outputs = OH_NN_UInt32Array {
            data: model_outputs_arr.as_mut_ptr(),
            size: 1,
        };
        if OH_NNModel_SpecifyInputsAndOutputs(ctx.model, &model_inputs, &model_outputs)
            != OH_NN_SUCCESS
        {
            return false;
        }

        // Finish model
        if OH_NNModel_Finish(ctx.model) != OH_NN_SUCCESS {
            return false;
        }

        // Compile
        ctx.compilation = OH_NNCompilation_Construct(ctx.model);
        if ctx.compilation.is_null() {
            return false;
        }
        if !set_accelerator_device(ctx.compilation) {
            return false;
        }
        if OH_NNCompilation_Build(ctx.compilation) != OH_NN_SUCCESS {
            return false;
        }

        // Create executor
        ctx.executor = OH_NNExecutor_Construct(ctx.compilation);
        if ctx.executor.is_null() {
            return false;
        }

        // Create input/output tensors
        ctx.input_tensor = OH_NNTensor_CreateWithSize(0, desc_input, data_size);
        if ctx.input_tensor.is_null() {
            return false;
        }
        ctx.output_tensor = OH_NNTensor_CreateWithSize(0, desc_out, data_size);
        if ctx.output_tensor.is_null() {
            return false;
        }

        // Fill input data
        let input_buf = OH_NNTensor_GetDataBuffer(ctx.input_tensor) as *mut u8;
        if input_buf.is_null() {
            return false;
        }
        core::ptr::copy_nonoverlapping(
            pixels.as_ptr() as *const u8,
            input_buf,
            data_size,
        );

        // Run inference
        let inputs = [ctx.input_tensor];
        let outputs = [ctx.output_tensor];
        if OH_NNExecutor_RunSync(ctx.executor, inputs.as_ptr(), 1, outputs.as_ptr(), 1)
            != OH_NN_SUCCESS
        {
            return false;
        }

        // Read output data
        let output_buf = OH_NNTensor_GetDataBuffer(ctx.output_tensor) as *mut u8;
        if output_buf.is_null() {
            return false;
        }
        core::ptr::copy_nonoverlapping(
            output_buf,
            pixels.as_mut_ptr() as *mut u8,
            data_size,
        );

        true
    }
}
// === NPU mat3_mul ===
//
// NNR computation graph (reproduces scalar `mat3_mul`):
//   tensor 0:  input (src)         — [1, n, 1, 4] NHWC FLOAT32  (model input)
//   tensor 1:  out                 — [1, n, 1, 4] NHWC FLOAT32  (model output)
//   tensor 2:  weights             — [3, 3] ND FLOAT32  (3x3 matrix, row-major)
//   tensor 3:  tx                  — [1] ND INT8     (value: 0 = TransposeX false)
//   tensor 4:  ty                  — [1] ND INT8     (value: 1 = TransposeY true)
//   tensor 5:  activationType      — [1] ND INT8     (value: 0 = FUSED_NONE)
//   tensor 6:  axis                — [1] ND INT32    (value: 3 = channel dim)
//   tensor 7:  rgb_begin           — [1] ND INT32    (value: 0)
//   tensor 8:  rgb_size            — [1] ND INT32    (value: 3)
//   tensor 9:  alpha_begin         — [1] ND INT32    (value: 3)
//   tensor 10: alpha_size          — [1] ND INT32    (value: 1)
//   tensor 11: bshape              — [4] ND INT32    (value: 1, n, 1, 4)
//   tensor 12: concat_axis         — [1] ND INT32    (value: 3)
//   tensor 13: zero                — [1] ND FLOAT32  (value: 0.0)
//   tensor 14: alpha               — [1, n, 1, 1] NHWC FLOAT32  (SLICE output)
//   tensor 15: mask                — [1, n, 1, 1] NHWC BOOL     (GREATER output)
//   tensor 16: mask_broadcast      — [1, n, 1, 4] NHWC BOOL     (BROADCAST_TO output)
//   tensor 17: rgb                 — [1, n, 1, 3] NHWC FLOAT32  (SLICE output)
//   tensor 18: matmul_out          — [1, n, 1, 3] NHWC FLOAT32  (MATMUL output)
//   tensor 19: result              — [1, n, 1, 4] NHWC FLOAT32  (CONCAT output)
//
// Operations:
//   SLICE:       params=[6],       inputs=[0, 9, 10], outputs=[14]  alpha = src[..., 3:4]
//   GREATER:     params=[],        inputs=[14, 13],    outputs=[15]  mask = alpha > 0
//   BROADCAST_TO: params=[11],     inputs=[15],        outputs=[16]  mask_b = broadcast(mask, [1,n,1,4])
//   SLICE:       params=[6],       inputs=[0, 7, 8],   outputs=[17]  rgb = src[..., 0:3]
//   MATMUL:      params=[3, 4, 5], inputs=[17, 2],     outputs=[18]  matmul_out = rgb @ weights^T
//   CONCAT:      params=[12],      inputs=[18, 14],    outputs=[19]  result = concat([matmul_out, alpha], axis=3)
//   SELECT:      params=[],        inputs=[16, 19, 0], outputs=[1]   out = where(mask_b, result, src)
//
// Semantics: out[i] = matrix[i][0]*c[0] + matrix[i][1]*c[1] + matrix[i][2]*c[2] for i in 0..3,
// alpha preserved, where alpha <= 0 → original pixel (skip). No clamp (F32 documents keep superwhites).
// Bit-identical to scalar `mat3_mul`: every op is single IEEE-754 f32 mul/add, same op order.
// MATMUL with TransposeY=true: [1,3] @ [3,3]^T = [1,3] — out[i] = c0*m[i][0] + c1*m[i][1] + c2*m[i][2].
// NpuCtx descs array is 25 (from channel_mixer) — sufficient for 20 tensors, no extension needed.

/// Attempt NPU execution of `mat3_mul`. Returns `true` on success,
/// `false` if NPU is unavailable or any NNR call fails (caller falls back).
pub unsafe fn npu_mat3_mul(pixels: &mut [[f32; 4]], matrix: [[f32; 3]; 3]) -> bool {
    unsafe {
        let n = pixels.len();
        if n == 0 {
            return true;
        }

        let shape: [i32; 4] = [1, n as i32, 1, 4];
        let data_size = n * 4 * 4;

        let mut ctx = NpuCtx::new();

        // Create model
        ctx.model = OH_NNModel_Construct();
        if ctx.model.is_null() {
            return false;
        }

        // Create tensor descs (indices 0..19)
        let desc_input = match create_desc(OH_NN_FLOAT32, &shape, OH_NN_FORMAT_NHWC) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_input);

        let desc_out = match create_desc(OH_NN_FLOAT32, &shape, OH_NN_FORMAT_NHWC) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_out);

        let weights_shape: [i32; 2] = [3, 3];
        let scalar_shape: [i32; 1] = [1];
        let bshape: [i32; 4] = [1, n as i32, 1, 4];
        let alpha_shape: [i32; 4] = [1, n as i32, 1, 1];
        let rgb_shape: [i32; 4] = [1, n as i32, 1, 3];

        let desc_weights = match create_desc(OH_NN_FLOAT32, &weights_shape, OH_NN_FORMAT_ND) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_weights);

        let desc_tx = match create_desc(OH_NN_INT8, &scalar_shape, OH_NN_FORMAT_ND) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_tx);

        let desc_ty = match create_desc(OH_NN_INT8, &scalar_shape, OH_NN_FORMAT_ND) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_ty);

        let desc_act = match create_desc(OH_NN_INT8, &scalar_shape, OH_NN_FORMAT_ND) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_act);

        let desc_axis = match create_desc(OH_NN_INT32, &scalar_shape, OH_NN_FORMAT_ND) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_axis);

        let desc_rgb_begin = match create_desc(OH_NN_INT32, &scalar_shape, OH_NN_FORMAT_ND) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_rgb_begin);

        let desc_rgb_size = match create_desc(OH_NN_INT32, &scalar_shape, OH_NN_FORMAT_ND) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_rgb_size);

        let desc_alpha_begin = match create_desc(OH_NN_INT32, &scalar_shape, OH_NN_FORMAT_ND) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_alpha_begin);

        let desc_alpha_size = match create_desc(OH_NN_INT32, &scalar_shape, OH_NN_FORMAT_ND) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_alpha_size);

        let desc_bshape = match create_desc(OH_NN_INT32, &bshape, OH_NN_FORMAT_ND) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_bshape);

        let desc_concat_axis = match create_desc(OH_NN_INT32, &scalar_shape, OH_NN_FORMAT_ND) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_concat_axis);

        let desc_zero = match create_desc(OH_NN_FLOAT32, &scalar_shape, OH_NN_FORMAT_ND) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_zero);

        let desc_alpha = match create_desc(OH_NN_FLOAT32, &alpha_shape, OH_NN_FORMAT_NHWC) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_alpha);

        let desc_mask = match create_desc(OH_NN_BOOL, &alpha_shape, OH_NN_FORMAT_NHWC) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_mask);

        let desc_mask_b = match create_desc(OH_NN_BOOL, &shape, OH_NN_FORMAT_NHWC) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_mask_b);

        let desc_rgb = match create_desc(OH_NN_FLOAT32, &rgb_shape, OH_NN_FORMAT_NHWC) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_rgb);

        let desc_matmul_out = match create_desc(OH_NN_FLOAT32, &rgb_shape, OH_NN_FORMAT_NHWC) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_matmul_out);

        let desc_result = match create_desc(OH_NN_FLOAT32, &shape, OH_NN_FORMAT_NHWC) {
            Some(d) => d,
            None => return false,
        };
        ctx.add_desc(desc_result);

        // Add all descs to model (indices 0..19)
        for i in 0..ctx.desc_count {
            if OH_NNModel_AddTensorToModel(ctx.model, ctx.descs[i]) != OH_NN_SUCCESS {
                return false;
            }
        }

        // Set constant tensor data
        // weights: 3x3 matrix, row-major [m00,m01,m02,m10,m11,m12,m20,m21,m22]
        let weights_data = [matrix[0][0], matrix[0][1], matrix[0][2],
            matrix[1][0], matrix[1][1], matrix[1][2],
            matrix[2][0], matrix[2][1], matrix[2][2]];
        if OH_NNModel_SetTensorData(ctx.model, 2, weights_data.as_ptr() as *const c_void, 36)
            != OH_NN_SUCCESS
        {
            return false;
        }

        let tx_data = [0i8];
        if OH_NNModel_SetTensorData(ctx.model, 3, tx_data.as_ptr() as *const c_void, 1)
            != OH_NN_SUCCESS
        {
            return false;
        }

        let ty_data = [1i8];
        if OH_NNModel_SetTensorData(ctx.model, 4, ty_data.as_ptr() as *const c_void, 1)
            != OH_NN_SUCCESS
        {
            return false;
        }

        let act_data = [0i8];
        if OH_NNModel_SetTensorData(ctx.model, 5, act_data.as_ptr() as *const c_void, 1)
            != OH_NN_SUCCESS
        {
            return false;
        }

        let axis_data = [3i32];
        if OH_NNModel_SetTensorData(ctx.model, 6, axis_data.as_ptr() as *const c_void, 4)
            != OH_NN_SUCCESS
        {
            return false;
        }

        let rgb_begin_data = [0i32];
        if OH_NNModel_SetTensorData(ctx.model, 7, rgb_begin_data.as_ptr() as *const c_void, 4)
            != OH_NN_SUCCESS
        {
            return false;
        }

        let rgb_size_data = [3i32];
        if OH_NNModel_SetTensorData(ctx.model, 8, rgb_size_data.as_ptr() as *const c_void, 4)
            != OH_NN_SUCCESS
        {
            return false;
        }

        let alpha_begin_data = [3i32];
        if OH_NNModel_SetTensorData(ctx.model, 9, alpha_begin_data.as_ptr() as *const c_void, 4)
            != OH_NN_SUCCESS
        {
            return false;
        }

        let alpha_size_data = [1i32];
        if OH_NNModel_SetTensorData(ctx.model, 10, alpha_size_data.as_ptr() as *const c_void, 4)
            != OH_NN_SUCCESS
        {
            return false;
        }

        let bshape_data = [1i32, n as i32, 1, 4];
        if OH_NNModel_SetTensorData(ctx.model, 11, bshape_data.as_ptr() as *const c_void, 16)
            != OH_NN_SUCCESS
        {
            return false;
        }

        let concat_axis_data = [3i32];
        if OH_NNModel_SetTensorData(ctx.model, 12, concat_axis_data.as_ptr() as *const c_void, 4)
            != OH_NN_SUCCESS
        {
            return false;
        }

        let zero_data = [0.0f32];
        if OH_NNModel_SetTensorData(ctx.model, 13, zero_data.as_ptr() as *const c_void, 4)
            != OH_NN_SUCCESS
        {
            return false;
        }

        // Add operations
        // SLICE (alpha): params=[6], inputs=[0,9,10], outputs=[14]
        let mut s1_params_arr = [6u32];
        let mut s1_inputs_arr = [0u32, 9, 10];
        let mut s1_outputs_arr = [14u32];
        let s1_params = OH_NN_UInt32Array {
            data: s1_params_arr.as_mut_ptr(),
            size: 1,
        };
        let s1_inputs = OH_NN_UInt32Array {
            data: s1_inputs_arr.as_mut_ptr(),
            size: 3,
        };
        let s1_outputs = OH_NN_UInt32Array {
            data: s1_outputs_arr.as_mut_ptr(),
            size: 1,
        };
        if OH_NNModel_AddOperation(
            ctx.model,
            OH_NN_OPS_SLICE,
            &s1_params,
            &s1_inputs,
            &s1_outputs,
        ) != OH_NN_SUCCESS
        {
            return false;
        }

        // GREATER: params=[], inputs=[14,13], outputs=[15]
        let mut gt_inputs_arr = [14u32, 13];
        let mut gt_outputs_arr = [15u32];
        let gt_params = OH_NN_UInt32Array {
            data: core::ptr::null_mut(),
            size: 0,
        };
        let gt_inputs = OH_NN_UInt32Array {
            data: gt_inputs_arr.as_mut_ptr(),
            size: 2,
        };
        let gt_outputs = OH_NN_UInt32Array {
            data: gt_outputs_arr.as_mut_ptr(),
            size: 1,
        };
        if OH_NNModel_AddOperation(
            ctx.model,
            OH_NN_OPS_GREATER,
            &gt_params,
            &gt_inputs,
            &gt_outputs,
        ) != OH_NN_SUCCESS
        {
            return false;
        }

        // BROADCAST_TO: params=[11], inputs=[15], outputs=[16]
        let mut bc_params_arr = [11u32];
        let mut bc_inputs_arr = [15u32];
        let mut bc_outputs_arr = [16u32];
        let bc_params = OH_NN_UInt32Array {
            data: bc_params_arr.as_mut_ptr(),
            size: 1,
        };
        let bc_inputs = OH_NN_UInt32Array {
            data: bc_inputs_arr.as_mut_ptr(),
            size: 1,
        };
        let bc_outputs = OH_NN_UInt32Array {
            data: bc_outputs_arr.as_mut_ptr(),
            size: 1,
        };
        if OH_NNModel_AddOperation(
            ctx.model,
            OH_NN_OPS_BROADCAST_TO,
            &bc_params,
            &bc_inputs,
            &bc_outputs,
        ) != OH_NN_SUCCESS
        {
            return false;
        }

        // SLICE (RGB): params=[6], inputs=[0,7,8], outputs=[17]
        let mut s2_params_arr = [6u32];
        let mut s2_inputs_arr = [0u32, 7, 8];
        let mut s2_outputs_arr = [17u32];
        let s2_params = OH_NN_UInt32Array {
            data: s2_params_arr.as_mut_ptr(),
            size: 1,
        };
        let s2_inputs = OH_NN_UInt32Array {
            data: s2_inputs_arr.as_mut_ptr(),
            size: 3,
        };
        let s2_outputs = OH_NN_UInt32Array {
            data: s2_outputs_arr.as_mut_ptr(),
            size: 1,
        };
        if OH_NNModel_AddOperation(
            ctx.model,
            OH_NN_OPS_SLICE,
            &s2_params,
            &s2_inputs,
            &s2_outputs,
        ) != OH_NN_SUCCESS
        {
            return false;
        }

        // MATMUL: params=[3,4,5], inputs=[17,2], outputs=[18]
        let mut mm_params_arr = [3u32, 4, 5];
        let mut mm_inputs_arr = [17u32, 2];
        let mut mm_outputs_arr = [18u32];
        let mm_params = OH_NN_UInt32Array {
            data: mm_params_arr.as_mut_ptr(),
            size: 3,
        };
        let mm_inputs = OH_NN_UInt32Array {
            data: mm_inputs_arr.as_mut_ptr(),
            size: 2,
        };
        let mm_outputs = OH_NN_UInt32Array {
            data: mm_outputs_arr.as_mut_ptr(),
            size: 1,
        };
        if OH_NNModel_AddOperation(
            ctx.model,
            OH_NN_OPS_MATMUL,
            &mm_params,
            &mm_inputs,
            &mm_outputs,
        ) != OH_NN_SUCCESS
        {
            return false;
        }

        // CONCAT: params=[12], inputs=[18,14], outputs=[19]
        let mut cc_params_arr = [12u32];
        let mut cc_inputs_arr = [18u32, 14];
        let mut cc_outputs_arr = [19u32];
        let cc_params = OH_NN_UInt32Array {
            data: cc_params_arr.as_mut_ptr(),
            size: 1,
        };
        let cc_inputs = OH_NN_UInt32Array {
            data: cc_inputs_arr.as_mut_ptr(),
            size: 2,
        };
        let cc_outputs = OH_NN_UInt32Array {
            data: cc_outputs_arr.as_mut_ptr(),
            size: 1,
        };
        if OH_NNModel_AddOperation(
            ctx.model,
            OH_NN_OPS_CONCAT,
            &cc_params,
            &cc_inputs,
            &cc_outputs,
        ) != OH_NN_SUCCESS
        {
            return false;
        }

        // SELECT: params=[], inputs=[16,19,0], outputs=[1]
        let mut sl_inputs_arr = [16u32, 19, 0];
        let mut sl_outputs_arr = [1u32];
        let sl_params = OH_NN_UInt32Array {
            data: core::ptr::null_mut(),
            size: 0,
        };
        let sl_inputs = OH_NN_UInt32Array {
            data: sl_inputs_arr.as_mut_ptr(),
            size: 3,
        };
        let sl_outputs = OH_NN_UInt32Array {
            data: sl_outputs_arr.as_mut_ptr(),
            size: 1,
        };
        if OH_NNModel_AddOperation(
            ctx.model,
            OH_NN_OPS_SELECT,
            &sl_params,
            &sl_inputs,
            &sl_outputs,
        ) != OH_NN_SUCCESS
        {
            return false;
        }

        // Specify inputs/outputs
        let mut model_inputs_arr = [0u32];
        let mut model_outputs_arr = [1u32];
        let model_inputs = OH_NN_UInt32Array {
            data: model_inputs_arr.as_mut_ptr(),
            size: 1,
        };
        let model_outputs = OH_NN_UInt32Array {
            data: model_outputs_arr.as_mut_ptr(),
            size: 1,
        };
        if OH_NNModel_SpecifyInputsAndOutputs(ctx.model, &model_inputs, &model_outputs)
            != OH_NN_SUCCESS
        {
            return false;
        }

        // Finish model
        if OH_NNModel_Finish(ctx.model) != OH_NN_SUCCESS {
            return false;
        }

        // Compile
        ctx.compilation = OH_NNCompilation_Construct(ctx.model);
        if ctx.compilation.is_null() {
            return false;
        }
        if !set_accelerator_device(ctx.compilation) {
            return false;
        }
        if OH_NNCompilation_Build(ctx.compilation) != OH_NN_SUCCESS {
            return false;
        }

        // Create executor
        ctx.executor = OH_NNExecutor_Construct(ctx.compilation);
        if ctx.executor.is_null() {
            return false;
        }

        // Create input/output tensors
        ctx.input_tensor = OH_NNTensor_CreateWithSize(0, desc_input, data_size);
        if ctx.input_tensor.is_null() {
            return false;
        }
        ctx.output_tensor = OH_NNTensor_CreateWithSize(0, desc_out, data_size);
        if ctx.output_tensor.is_null() {
            return false;
        }

        // Fill input data
        let input_buf = OH_NNTensor_GetDataBuffer(ctx.input_tensor) as *mut u8;
        if input_buf.is_null() {
            return false;
        }
        core::ptr::copy_nonoverlapping(
            pixels.as_ptr() as *const u8,
            input_buf,
            data_size,
        );

        // Run inference
        let inputs = [ctx.input_tensor];
        let outputs = [ctx.output_tensor];
        if OH_NNExecutor_RunSync(ctx.executor, inputs.as_ptr(), 1, outputs.as_ptr(), 1)
            != OH_NN_SUCCESS
        {
            return false;
        }

        // Read output data
        let output_buf = OH_NNTensor_GetDataBuffer(ctx.output_tensor) as *mut u8;
        if output_buf.is_null() {
            return false;
        }
        core::ptr::copy_nonoverlapping(
            output_buf,
            pixels.as_mut_ptr() as *mut u8,
            data_size,
        );

        true
    }
}