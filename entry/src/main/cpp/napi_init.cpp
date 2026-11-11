#include <cstdint>
#include <cstring>
#include <string>
#include <vector>


#include <napi/native_api.h>

extern "C" {
struct PcBuffer {
    uint8_t *ptr;
    size_t len;
    size_t cap;
};

void *pc_session_new();
void pc_session_free(void *session);
void pc_buffer_free(PcBuffer *buffer);

PcBuffer *pc_open(void *session, const uint8_t *name, size_t name_len, const uint8_t *data, size_t data_len);
PcBuffer *pc_document_new(void *session, uint32_t width, uint32_t height, const uint8_t *name, size_t name_len);
PcBuffer *pc_document_turn(void *session, uint64_t id, uint32_t turn);
PcBuffer *pc_history(void *session, uint64_t id);
PcBuffer *pc_undo_steps(void *session, uint64_t id, uint32_t steps);
PcBuffer *pc_redo_steps(void *session, uint64_t id, uint32_t steps);
PcBuffer *pc_close(void *session, uint64_t id);
PcBuffer *pc_info(void *session, uint64_t id);
PcBuffer *pc_preview(void *session, uint64_t id, uint32_t max_side, PcBuffer **out);
PcBuffer *pc_layer_thumbnail(void *session, uint64_t id, uint64_t layer, uint32_t side, PcBuffer **out);
PcBuffer *pc_render(void *session, uint64_t id, int32_t x, int32_t y, int32_t w, int32_t h, PcBuffer **out);
PcBuffer *pc_paint_stroke(void *session, uint64_t id, uint64_t layer, const uint8_t *params, size_t params_len, const uint8_t *points, size_t points_len);
PcBuffer *pc_select(void *session, uint64_t id, const uint8_t *params, size_t params_len);
PcBuffer *pc_selection_info(void *session, uint64_t id);
PcBuffer *pc_filter_apply(void *session, uint64_t id, uint64_t layer, const uint8_t *params, size_t params_len);
PcBuffer *pc_shape_create(void *session, uint64_t id, const uint8_t *params, size_t params_len);
PcBuffer *pc_font_register(const uint8_t *data, size_t data_len);
PcBuffer *pc_text_create(void *session, uint64_t id, const uint8_t *params, size_t params_len);
PcBuffer *pc_layer_edit_get(void *session, uint64_t id, uint64_t layer);
PcBuffer *pc_layer_edit_apply(void *session, uint64_t id, uint64_t layer, const uint8_t *params, size_t params_len);
PcBuffer *pc_layer_visible(void *session, uint64_t id, uint64_t layer, bool visible);
PcBuffer *pc_layer_opacity(void *session, uint64_t id, uint64_t layer, float opacity);
PcBuffer *pc_layer_fill_opacity(void *session, uint64_t id, uint64_t layer, float opacity);
PcBuffer *pc_layer_blend(void *session, uint64_t id, uint64_t layer, const uint8_t *mode, size_t mode_len);
PcBuffer *pc_layer_create(void *session, uint64_t id, uint64_t above, const uint8_t *name, size_t name_len);
PcBuffer *pc_layer_rename(void *session, uint64_t id, uint64_t layer, const uint8_t *name, size_t name_len);
PcBuffer *pc_layer_move(void *session, uint64_t id, uint64_t layer, int32_t delta);
PcBuffer *pc_layer_duplicate(void *session, uint64_t id, uint64_t layer);
PcBuffer *pc_layer_duplicate_many(void *session, uint64_t id, const uint8_t *ids, size_t ids_len);
PcBuffer *pc_layer_merge_down(void *session, uint64_t id, uint64_t layer);
PcBuffer *pc_layer_delete(void *session, uint64_t id, uint64_t layer);
PcBuffer *pc_adjustment_add(void *session, uint64_t id, const uint8_t *kind, size_t kind_len, const uint8_t *params, size_t params_len);
PcBuffer *pc_adjustment_update(void *session, uint64_t id, uint64_t layer, const uint8_t *params, size_t params_len);
PcBuffer *pc_adjustment_get(void *session, uint64_t id, uint64_t layer);
PcBuffer *pc_undo(void *session, uint64_t id);
PcBuffer *pc_redo(void *session, uint64_t id);
PcBuffer *pc_export(void *session, uint64_t id, const uint8_t *format, size_t format_len, uint8_t quality, PcBuffer **out);
PcBuffer *pc_layer_effects_get(void *session, uint64_t id, uint64_t layer);
PcBuffer *pc_layer_effects_apply(void *session, uint64_t id, uint64_t layer, const uint8_t *params, size_t params_len);
PcBuffer *pc_render_pixels(void *session, uint64_t id, int32_t x, int32_t y, int32_t w, int32_t h, PcBuffer **out);
PcBuffer *pc_revision(void *session, uint64_t id);
PcBuffer *pc_layer_mask_get(void *session, uint64_t id, uint64_t layer);
PcBuffer *pc_layer_mask_add(void *session, uint64_t id, uint64_t layer, const uint8_t *fill, size_t fill_len);
PcBuffer *pc_layer_mask_set(void *session, uint64_t id, uint64_t layer, const uint8_t *params, size_t params_len);
PcBuffer *pc_layer_mask_fill_from_selection(void *session, uint64_t id, uint64_t layer, const uint8_t *mode, size_t mode_len);
PcBuffer *pc_layer_mask_apply(void *session, uint64_t id, uint64_t layer);
PcBuffer *pc_layer_mask_remove(void *session, uint64_t id, uint64_t layer);
PcBuffer *pc_layer_group(void *session, uint64_t id, const uint8_t *ids, size_t ids_len, const uint8_t *name, size_t name_len);
PcBuffer *pc_layer_ungroup(void *session, uint64_t id, uint64_t layer);
PcBuffer *pc_layer_expanded(void *session, uint64_t id, uint64_t layer, uint8_t expanded);
PcBuffer *pc_layer_clipped(void *session, uint64_t id, uint64_t layer, uint8_t clipped);
PcBuffer *pc_layer_delete_many(void *session, uint64_t id, const uint8_t *ids, size_t ids_len);
PcBuffer *pc_document_crop(void *session, uint64_t id, int32_t x0, int32_t y0, int32_t x1, int32_t y1);
PcBuffer *pc_document_resize_canvas(void *session, uint64_t id, int32_t width, int32_t height, int32_t anchor);
PcBuffer *pc_guides_get(void *session, uint64_t id);
PcBuffer *pc_guides_set(void *session, uint64_t id, const uint8_t *params, size_t params_len);
PcBuffer *pc_layer_transform(void *session, uint64_t id, uint64_t layer, const uint8_t *params, size_t params_len);
}

namespace {

void *g_session = nullptr;

void *Session() {
    if (g_session == nullptr) {
        g_session = pc_session_new();
    }
    return g_session;
}

std::string TakeString(PcBuffer *buffer) {
    if (buffer == nullptr) {
        return "{\"ok\":false,\"error\":\"engine returned no result\"}";
    }
    std::string text(reinterpret_cast<const char *>(buffer->ptr), buffer->len);
    pc_buffer_free(buffer);
    return text;
}

bool IsOk(const std::string &json) {
    return json.find("\"ok\":false") == std::string::npos;
}

std::string ErrorOf(const std::string &json) {
    const std::string key = "\"error\":\"";
    size_t at = json.find(key);
    if (at == std::string::npos) {
        return json;
    }
    at += key.size();
    size_t end = at;
    while (end < json.size() && json[end] != '"') {
        if (json[end] == '\\' && end + 1 < json.size()) {
            end += 2;
            continue;
        }
        end++;
    }
    return json.substr(at, end - at);
}

napi_value Throw(napi_env env, const std::string &message) {
    napi_throw_error(env, nullptr, message.c_str());
    return nullptr;
}

bool GetString(napi_env env, napi_value value, std::string &out) {
    size_t len = 0;
    if (napi_get_value_string_utf8(env, value, nullptr, 0, &len) != napi_ok) {
        return false;
    }
    out.resize(len);
    if (len == 0) {
        return true;
    }
    return napi_get_value_string_utf8(env, value, out.data(), len + 1, &len) == napi_ok;
}

bool GetBytes(napi_env env, napi_value value, const uint8_t *&data, size_t &len) {
    bool is_buffer = false;
    if (napi_is_buffer(env, value, &is_buffer) == napi_ok && is_buffer) {
        void *ptr = nullptr;
        if (napi_get_buffer_info(env, value, &ptr, &len) == napi_ok) {
            data = static_cast<const uint8_t *>(ptr);
            return true;
        }
    }
    napi_typedarray_type type;
    size_t length = 0;
    void *ptr = nullptr;
    napi_value arraybuffer = nullptr;
    size_t offset = 0;
    if (napi_get_typedarray_info(env, value, &type, &length, &ptr, &arraybuffer, &offset) == napi_ok) {
        data = static_cast<const uint8_t *>(ptr);
        len = length;
        return true;
    }
    if (napi_get_arraybuffer_info(env, value, &ptr, &len) == napi_ok) {
        data = static_cast<const uint8_t *>(ptr);
        return true;
    }
    return false;
}

napi_value MakeArrayBuffer(napi_env env, PcBuffer *buffer) {
    if (buffer == nullptr) {
        return Throw(env, "engine returned no bytes");
    }
    napi_value result = nullptr;
    void *out = nullptr;
    napi_status status = napi_create_arraybuffer(env, buffer->len, &out, &result);
    if (status != napi_ok || result == nullptr) {
        pc_buffer_free(buffer);
        return Throw(env, "could not allocate a result buffer");
    }
    if (buffer->len > 0) {
        std::memcpy(out, buffer->ptr, buffer->len);
    }
    pc_buffer_free(buffer);
    return result;
}

uint64_t GetU64(napi_env env, napi_value value) {
    double number = 0;
    if (napi_get_value_double(env, value, &number) == napi_ok) {
        return static_cast<uint64_t>(number);
    }
    int64_t big = 0;
    if (napi_get_value_int64(env, value, &big) == napi_ok) {
        return static_cast<uint64_t>(big);
    }
    return 0;
}

napi_value OpenImage(napi_env env, napi_callback_info info) {
    size_t argc = 2;
    napi_value argv[2] = {nullptr, nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    if (argc < 2) {
        return Throw(env, "openImage(name, data) needs two arguments");
    }
    std::string name;
    if (!GetString(env, argv[0], name)) {
        return Throw(env, "openImage: invalid name");
    }
    const uint8_t *data = nullptr;
    size_t len = 0;
    if (!GetBytes(env, argv[1], data, len)) {
        return Throw(env, "openImage: data must be an ArrayBuffer");
    }
    std::string result = TakeString(pc_open(Session(), reinterpret_cast<const uint8_t *>(name.data()), name.size(), data, len));
    if (!IsOk(result)) {
        return Throw(env, ErrorOf(result));
    }
    napi_value out = nullptr;
    napi_create_string_utf8(env, result.c_str(), result.size(), &out);
    return out;
}

napi_value NewDocument(napi_env env, napi_callback_info info) {
    size_t argc = 3;
    napi_value argv[3] = {nullptr, nullptr, nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    double width = 0;
    double height = 0;
    napi_get_value_double(env, argv[0], &width);
    napi_get_value_double(env, argv[1], &height);
    std::string name;
    if (!GetString(env, argv[2], name)) {
        return Throw(env, "newDocument: name must be a string");
    }
    std::string result = TakeString(pc_document_new(Session(), static_cast<uint32_t>(width), static_cast<uint32_t>(height),
                                                    reinterpret_cast<const uint8_t *>(name.data()), name.size()));
    if (!IsOk(result)) {
        return Throw(env, ErrorOf(result));
    }
    napi_value out = nullptr;
    napi_create_string_utf8(env, result.c_str(), result.size(), &out);
    return out;
}

napi_value DocumentTurn(napi_env env, napi_callback_info info) {
    size_t argc = 2;
    napi_value argv[2] = {nullptr, nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    double turn = 0;
    napi_get_value_double(env, argv[1], &turn);
    std::string result = TakeString(pc_document_turn(Session(), GetU64(env, argv[0]), static_cast<uint32_t>(turn)));
    if (!IsOk(result)) {
        return Throw(env, ErrorOf(result));
    }
    napi_value out = nullptr;
    napi_create_string_utf8(env, result.c_str(), result.size(), &out);
    return out;
}

napi_value History(napi_env env, napi_callback_info info) {
    size_t argc = 1;
    napi_value argv[1] = {nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    std::string result = TakeString(pc_history(Session(), GetU64(env, argv[0])));
    if (!IsOk(result)) {
        return Throw(env, ErrorOf(result));
    }
    napi_value out = nullptr;
    napi_create_string_utf8(env, result.c_str(), result.size(), &out);
    return out;
}

napi_value UndoSteps(napi_env env, napi_callback_info info) {
    size_t argc = 2;
    napi_value argv[2] = {nullptr, nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    double steps = 0;
    napi_get_value_double(env, argv[1], &steps);
    std::string result = TakeString(pc_undo_steps(Session(), GetU64(env, argv[0]), static_cast<uint32_t>(steps)));
    if (!IsOk(result)) {
        return Throw(env, ErrorOf(result));
    }
    napi_value out = nullptr;
    napi_create_string_utf8(env, result.c_str(), result.size(), &out);
    return out;
}

napi_value RedoSteps(napi_env env, napi_callback_info info) {
    size_t argc = 2;
    napi_value argv[2] = {nullptr, nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    double steps = 0;
    napi_get_value_double(env, argv[1], &steps);
    std::string result = TakeString(pc_redo_steps(Session(), GetU64(env, argv[0]), static_cast<uint32_t>(steps)));
    if (!IsOk(result)) {
        return Throw(env, ErrorOf(result));
    }
    napi_value out = nullptr;
    napi_create_string_utf8(env, result.c_str(), result.size(), &out);
    return out;
}

napi_value Close(napi_env env, napi_callback_info info) {
    size_t argc = 1;
    napi_value argv[1] = {nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    std::string result = TakeString(pc_close(Session(), GetU64(env, argv[0])));
    if (!IsOk(result)) {
        return Throw(env, ErrorOf(result));
    }
    return nullptr;
}

napi_value Info(napi_env env, napi_callback_info info) {
    size_t argc = 1;
    napi_value argv[1] = {nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    std::string result = TakeString(pc_info(Session(), GetU64(env, argv[0])));
    if (!IsOk(result)) {
        return Throw(env, ErrorOf(result));
    }
    napi_value out = nullptr;
    napi_create_string_utf8(env, result.c_str(), result.size(), &out);
    return out;
}

napi_value Preview(napi_env env, napi_callback_info info) {
    size_t argc = 2;
    napi_value argv[2] = {nullptr, nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    int32_t side = 1024;
    napi_get_value_int32(env, argv[1], &side);
    PcBuffer *out = nullptr;
    std::string result = TakeString(pc_preview(Session(), GetU64(env, argv[0]), static_cast<uint32_t>(side), &out));
    if (!IsOk(result)) {
        if (out != nullptr) {
            pc_buffer_free(out);
        }
        return Throw(env, ErrorOf(result));
    }
    return MakeArrayBuffer(env, out);
}

napi_value LayerThumbnail(napi_env env, napi_callback_info info) {
    size_t argc = 3;
    napi_value argv[3] = {nullptr, nullptr, nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    int32_t side = 48;
    napi_get_value_int32(env, argv[2], &side);
    PcBuffer *out = nullptr;
    std::string result = TakeString(pc_layer_thumbnail(Session(), GetU64(env, argv[0]), GetU64(env, argv[1]), static_cast<uint32_t>(side), &out));
    if (!IsOk(result)) {
        if (out != nullptr) {
            pc_buffer_free(out);
        }
        return Throw(env, ErrorOf(result));
    }
    return MakeArrayBuffer(env, out);
}

napi_value Render(napi_env env, napi_callback_info info) {
    size_t argc = 5;
    napi_value argv[5] = {nullptr, nullptr, nullptr, nullptr, nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    int32_t x = 0;
    int32_t y = 0;
    int32_t w = 0;
    int32_t h = 0;
    napi_get_value_int32(env, argv[1], &x);
    napi_get_value_int32(env, argv[2], &y);
    napi_get_value_int32(env, argv[3], &w);
    napi_get_value_int32(env, argv[4], &h);
    PcBuffer *out = nullptr;
    std::string result = TakeString(pc_render(Session(), GetU64(env, argv[0]), x, y, w, h, &out));
    if (!IsOk(result)) {
        if (out != nullptr) {
            pc_buffer_free(out);
        }
        return Throw(env, ErrorOf(result));
    }
    return MakeArrayBuffer(env, out);
}

napi_value Select(napi_env env, napi_callback_info info) {
    size_t argc = 2;
    napi_value argv[2] = {nullptr, nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    if (argc < 2) {
        return Throw(env, "select(documentId, paramsJson) needs two arguments");
    }
    std::string params;
    if (!GetString(env, argv[1], params)) {
        return Throw(env, "select: paramsJson must be a string");
    }
    std::string result = TakeString(pc_select(Session(), GetU64(env, argv[0]), reinterpret_cast<const uint8_t *>(params.data()), params.size()));
    if (!IsOk(result)) {
        return Throw(env, ErrorOf(result));
    }
    napi_value out = nullptr;
    napi_create_string_utf8(env, result.c_str(), result.size(), &out);
    return out;
}

napi_value SelectionInfo(napi_env env, napi_callback_info info) {
    size_t argc = 1;
    napi_value argv[1] = {nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    std::string result = TakeString(pc_selection_info(Session(), GetU64(env, argv[0])));
    if (!IsOk(result)) {
        return Throw(env, ErrorOf(result));
    }
    napi_value out = nullptr;
    napi_create_string_utf8(env, result.c_str(), result.size(), &out);
    return out;
}

napi_value FilterApply(napi_env env, napi_callback_info info) {
    size_t argc = 3;
    napi_value argv[3] = {nullptr, nullptr, nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    if (argc < 3) {
        return Throw(env, "filterApply(documentId, layerId, paramsJson) needs three arguments");
    }
    std::string params;
    if (!GetString(env, argv[2], params)) {
        return Throw(env, "filterApply: paramsJson must be a string");
    }
    std::string result = TakeString(pc_filter_apply(Session(), GetU64(env, argv[0]), GetU64(env, argv[1]),
                                                    reinterpret_cast<const uint8_t *>(params.data()), params.size()));
    if (!IsOk(result)) {
        return Throw(env, ErrorOf(result));
    }
    napi_value out = nullptr;
    napi_create_string_utf8(env, result.c_str(), result.size(), &out);
    return out;
}

napi_value ShapeCreate(napi_env env, napi_callback_info info) {
    size_t argc = 2;
    napi_value argv[2] = {nullptr, nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    if (argc < 2) {
        return Throw(env, "shapeCreate(documentId, paramsJson) needs two arguments");
    }
    std::string params;
    if (!GetString(env, argv[1], params)) {
        return Throw(env, "shapeCreate: paramsJson must be a string");
    }
    std::string result = TakeString(pc_shape_create(Session(), GetU64(env, argv[0]), reinterpret_cast<const uint8_t *>(params.data()), params.size()));
    if (!IsOk(result)) {
        return Throw(env, ErrorOf(result));
    }
    napi_value out = nullptr;
    napi_create_string_utf8(env, result.c_str(), result.size(), &out);
    return out;
}

napi_value PaintStroke(napi_env env, napi_callback_info info) {
    size_t argc = 4;
    napi_value argv[4] = {nullptr, nullptr, nullptr, nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    if (argc < 4) {
        return Throw(env, "paintStroke(documentId, layerId, paramsJson, pointsJson) needs four arguments");
    }
    std::string params;
    std::string points;
    if (!GetString(env, argv[2], params) || !GetString(env, argv[3], points)) {
        return Throw(env, "paintStroke: paramsJson and pointsJson must be strings");
    }
    std::string result = TakeString(pc_paint_stroke(Session(), GetU64(env, argv[0]), GetU64(env, argv[1]),
                                                    reinterpret_cast<const uint8_t *>(params.data()), params.size(),
                                                    reinterpret_cast<const uint8_t *>(points.data()), points.size()));
    if (!IsOk(result)) {
        return Throw(env, ErrorOf(result));
    }
    napi_value out = nullptr;
    napi_create_string_utf8(env, result.c_str(), result.size(), &out);
    return out;
}

napi_value FontRegister(napi_env env, napi_callback_info info) {
    size_t argc = 1;
    napi_value argv[1] = {nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    const uint8_t *data = nullptr;
    size_t len = 0;
    if (!GetBytes(env, argv[0], data, len)) {
        return Throw(env, "fontRegister: data must be an ArrayBuffer or TypedArray");
    }
    std::string result = TakeString(pc_font_register(data, len));
    if (!IsOk(result)) {
        return Throw(env, ErrorOf(result));
    }
    napi_value out = nullptr;
    napi_create_string_utf8(env, result.c_str(), result.size(), &out);
    return out;
}

napi_value TextCreate(napi_env env, napi_callback_info info) {
    size_t argc = 2;
    napi_value argv[2] = {nullptr, nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    if (argc < 2) {
        return Throw(env, "textCreate(documentId, paramsJson) needs two arguments");
    }
    std::string params;
    if (!GetString(env, argv[1], params)) {
        return Throw(env, "textCreate: paramsJson must be a string");
    }
    std::string result = TakeString(pc_text_create(Session(), GetU64(env, argv[0]), reinterpret_cast<const uint8_t *>(params.data()), params.size()));
    if (!IsOk(result)) {
        return Throw(env, ErrorOf(result));
    }
    napi_value out = nullptr;
    napi_create_string_utf8(env, result.c_str(), result.size(), &out);
    return out;
}

napi_value LayerEditGet(napi_env env, napi_callback_info info) {
    size_t argc = 2;
    napi_value argv[2] = {nullptr, nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    if (argc < 2) {
        return Throw(env, "layerEditGet(documentId, layerId) needs two arguments");
    }
    std::string result = TakeString(pc_layer_edit_get(Session(), GetU64(env, argv[0]), GetU64(env, argv[1])));
    if (!IsOk(result)) {
        return Throw(env, ErrorOf(result));
    }
    napi_value out = nullptr;
    napi_create_string_utf8(env, result.c_str(), result.size(), &out);
    return out;
}

napi_value LayerEditApply(napi_env env, napi_callback_info info) {
    size_t argc = 3;
    napi_value argv[3] = {nullptr, nullptr, nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    if (argc < 3) {
        return Throw(env, "layerEditApply(documentId, layerId, paramsJson) needs three arguments");
    }
    std::string params;
    if (!GetString(env, argv[2], params)) {
        return Throw(env, "layerEditApply: paramsJson must be a string");
    }
    std::string result = TakeString(pc_layer_edit_apply(Session(), GetU64(env, argv[0]), GetU64(env, argv[1]),
                                                        reinterpret_cast<const uint8_t *>(params.data()), params.size()));
    if (!IsOk(result)) {
        return Throw(env, ErrorOf(result));
    }
    napi_value out = nullptr;
    napi_create_string_utf8(env, result.c_str(), result.size(), &out);
    return out;
}

napi_value LayerVisible(napi_env env, napi_callback_info info) {
    size_t argc = 3;
    napi_value argv[3] = {nullptr, nullptr, nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    bool visible = true;
    napi_get_value_bool(env, argv[2], &visible);
    std::string result = TakeString(pc_layer_visible(Session(), GetU64(env, argv[0]), GetU64(env, argv[1]), visible));
    if (!IsOk(result)) {
        return Throw(env, ErrorOf(result));
    }
    return nullptr;
}

napi_value LayerOpacity(napi_env env, napi_callback_info info) {
    size_t argc = 3;
    napi_value argv[3] = {nullptr, nullptr, nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    double opacity = 1.0;
    napi_get_value_double(env, argv[2], &opacity);
    std::string result = TakeString(pc_layer_opacity(Session(), GetU64(env, argv[0]), GetU64(env, argv[1]), static_cast<float>(opacity)));
    if (!IsOk(result)) {
        return Throw(env, ErrorOf(result));
    }
    return nullptr;
}

napi_value LayerFillOpacity(napi_env env, napi_callback_info info) {
    size_t argc = 3;
    napi_value argv[3] = {nullptr, nullptr, nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    double opacity = 1.0;
    napi_get_value_double(env, argv[2], &opacity);
    std::string result = TakeString(pc_layer_fill_opacity(Session(), GetU64(env, argv[0]), GetU64(env, argv[1]), static_cast<float>(opacity)));
    if (!IsOk(result)) {
        return Throw(env, ErrorOf(result));
    }
    return nullptr;
}

napi_value LayerBlend(napi_env env, napi_callback_info info) {
    size_t argc = 3;
    napi_value argv[3] = {nullptr, nullptr, nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    std::string mode;
    if (!GetString(env, argv[2], mode)) {
        return Throw(env, "setLayerBlend: mode must be a string");
    }
    std::string result = TakeString(pc_layer_blend(Session(), GetU64(env, argv[0]), GetU64(env, argv[1]),
                                                   reinterpret_cast<const uint8_t *>(mode.data()), mode.size()));
    if (!IsOk(result)) {
        return Throw(env, ErrorOf(result));
    }
    return nullptr;
}

napi_value LayerCreate(napi_env env, napi_callback_info info) {
    size_t argc = 3;
    napi_value argv[3] = {nullptr, nullptr, nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    std::string name;
    if (!GetString(env, argv[2], name)) {
        return Throw(env, "createLayer: name must be a string");
    }
    std::string result = TakeString(pc_layer_create(Session(), GetU64(env, argv[0]), GetU64(env, argv[1]),
                                                    reinterpret_cast<const uint8_t *>(name.data()), name.size()));
    if (!IsOk(result)) {
        return Throw(env, ErrorOf(result));
    }
    napi_value out = nullptr;
    napi_create_string_utf8(env, result.c_str(), result.size(), &out);
    return out;
}

napi_value LayerRename(napi_env env, napi_callback_info info) {
    size_t argc = 3;
    napi_value argv[3] = {nullptr, nullptr, nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    std::string name;
    if (!GetString(env, argv[2], name)) {
        return Throw(env, "renameLayer: name must be a string");
    }
    std::string result = TakeString(pc_layer_rename(Session(), GetU64(env, argv[0]), GetU64(env, argv[1]),
                                                    reinterpret_cast<const uint8_t *>(name.data()), name.size()));
    if (!IsOk(result)) {
        return Throw(env, ErrorOf(result));
    }
    return nullptr;
}

napi_value LayerMove(napi_env env, napi_callback_info info) {
    size_t argc = 3;
    napi_value argv[3] = {nullptr, nullptr, nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    double delta = 0;
    napi_get_value_double(env, argv[2], &delta);
    std::string result = TakeString(pc_layer_move(Session(), GetU64(env, argv[0]), GetU64(env, argv[1]), static_cast<int32_t>(delta)));
    if (!IsOk(result)) {
        return Throw(env, ErrorOf(result));
    }
    napi_value out = nullptr;
    napi_create_string_utf8(env, result.c_str(), result.size(), &out);
    return out;
}

napi_value LayerDuplicate(napi_env env, napi_callback_info info) {
    size_t argc = 2;
    napi_value argv[2] = {nullptr, nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    std::string result = TakeString(pc_layer_duplicate(Session(), GetU64(env, argv[0]), GetU64(env, argv[1])));
    if (!IsOk(result)) {
        return Throw(env, ErrorOf(result));
    }
    napi_value out = nullptr;
    napi_create_string_utf8(env, result.c_str(), result.size(), &out);
    return out;
}

napi_value LayerDuplicateMany(napi_env env, napi_callback_info info) {
    size_t argc = 2;
    napi_value argv[2] = {nullptr, nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    std::string ids;
    if (!GetString(env, argv[1], ids)) {
        return Throw(env, "duplicateLayerMany: idsJson must be a string");
    }
    std::string result = TakeString(pc_layer_duplicate_many(Session(), GetU64(env, argv[0]),
                                                            reinterpret_cast<const uint8_t *>(ids.data()), ids.size()));
    if (!IsOk(result)) {
        return Throw(env, ErrorOf(result));
    }
    napi_value out = nullptr;
    napi_create_string_utf8(env, result.c_str(), result.size(), &out);
    return out;
}

napi_value LayerMerge(napi_env env, napi_callback_info info) {
    size_t argc = 2;
    napi_value argv[2] = {nullptr, nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    std::string result = TakeString(pc_layer_merge_down(Session(), GetU64(env, argv[0]), GetU64(env, argv[1])));
    if (!IsOk(result)) {
        return Throw(env, ErrorOf(result));
    }
    napi_value out = nullptr;
    napi_create_string_utf8(env, result.c_str(), result.size(), &out);
    return out;
}

napi_value LayerDelete(napi_env env, napi_callback_info info) {
    size_t argc = 2;
    napi_value argv[2] = {nullptr, nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    std::string result = TakeString(pc_layer_delete(Session(), GetU64(env, argv[0]), GetU64(env, argv[1])));
    if (!IsOk(result)) {
        return Throw(env, ErrorOf(result));
    }
    return nullptr;
}

napi_value LayerDeleteMany(napi_env env, napi_callback_info info) {
    size_t argc = 2;
    napi_value argv[2] = {nullptr, nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    if (argc < 2) {
        return Throw(env, "layerDeleteMany(documentId, idsJson) needs two arguments");
    }
    std::string ids;
    if (!GetString(env, argv[1], ids)) {
        return Throw(env, "layerDeleteMany: idsJson must be a string");
    }
    std::string result = TakeString(pc_layer_delete_many(Session(), GetU64(env, argv[0]),
                                                        reinterpret_cast<const uint8_t *>(ids.data()), ids.size()));
    if (!IsOk(result)) {
        return Throw(env, ErrorOf(result));
    }
    napi_value out = nullptr;
    napi_create_string_utf8(env, result.c_str(), result.size(), &out);
    return out;
}

napi_value AdjustmentAdd(napi_env env, napi_callback_info info) {
    size_t argc = 3;
    napi_value argv[3] = {nullptr, nullptr, nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    std::string kind;
    std::string params;
    if (!GetString(env, argv[1], kind) || !GetString(env, argv[2], params)) {
        return Throw(env, "addAdjustment(documentId, kind, paramsJson)");
    }
    std::string result = TakeString(pc_adjustment_add(Session(), GetU64(env, argv[0]), reinterpret_cast<const uint8_t *>(kind.data()), kind.size(),
                                                      reinterpret_cast<const uint8_t *>(params.data()), params.size()));
    if (!IsOk(result)) {
        return Throw(env, ErrorOf(result));
    }
    napi_value out = nullptr;
    napi_create_string_utf8(env, result.c_str(), result.size(), &out);
    return out;
}

napi_value AdjustmentUpdate(napi_env env, napi_callback_info info) {
    size_t argc = 3;
    napi_value argv[3] = {nullptr, nullptr, nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    std::string params;
    if (!GetString(env, argv[2], params)) {
        return Throw(env, "updateAdjustment(documentId, layerId, paramsJson)");
    }
    std::string result = TakeString(pc_adjustment_update(Session(), GetU64(env, argv[0]), GetU64(env, argv[1]),
                                                         reinterpret_cast<const uint8_t *>(params.data()), params.size()));
    if (!IsOk(result)) {
        return Throw(env, ErrorOf(result));
    }
    return nullptr;
}

napi_value AdjustmentGet(napi_env env, napi_callback_info info) {
    size_t argc = 2;
    napi_value argv[2] = {nullptr, nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    std::string result = TakeString(pc_adjustment_get(Session(), GetU64(env, argv[0]), GetU64(env, argv[1])));
    if (!IsOk(result)) {
        return Throw(env, ErrorOf(result));
    }
    napi_value out = nullptr;
    napi_create_string_utf8(env, result.c_str(), result.size(), &out);
    return out;
}

napi_value Undo(napi_env env, napi_callback_info info) {
    size_t argc = 1;
    napi_value argv[1] = {nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    std::string result = TakeString(pc_undo(Session(), GetU64(env, argv[0])));
    if (!IsOk(result)) {
        return Throw(env, ErrorOf(result));
    }
    napi_value out = nullptr;
    napi_create_string_utf8(env, result.c_str(), result.size(), &out);
    return out;
}

napi_value Redo(napi_env env, napi_callback_info info) {
    size_t argc = 1;
    napi_value argv[1] = {nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    std::string result = TakeString(pc_redo(Session(), GetU64(env, argv[0])));
    if (!IsOk(result)) {
        return Throw(env, ErrorOf(result));
    }
    napi_value out = nullptr;
    napi_create_string_utf8(env, result.c_str(), result.size(), &out);
    return out;
}

napi_value ExportImage(napi_env env, napi_callback_info info) {
    size_t argc = 3;
    napi_value argv[3] = {nullptr, nullptr, nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    std::string format = "png";
    int32_t quality = 90;
    GetString(env, argv[1], format);
    napi_get_value_int32(env, argv[2], &quality);
    PcBuffer *out = nullptr;
    std::string result = TakeString(pc_export(Session(), GetU64(env, argv[0]), reinterpret_cast<const uint8_t *>(format.data()), format.size(),
                                              static_cast<uint8_t>(quality), &out));
    if (!IsOk(result)) {
        if (out != nullptr) {
            pc_buffer_free(out);
        }
        return Throw(env, ErrorOf(result));
    }
    return MakeArrayBuffer(env, out);
}

napi_value LayerEffects(napi_env env, napi_callback_info info) {
    size_t argc = 2;
    napi_value argv[2] = {nullptr, nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    if (argc < 2) {
        return Throw(env, "layerEffects(documentId, layerId) needs two arguments");
    }
    std::string result = TakeString(pc_layer_effects_get(Session(), GetU64(env, argv[0]), GetU64(env, argv[1])));
    if (!IsOk(result)) {
        return Throw(env, ErrorOf(result));
    }
    napi_value out = nullptr;
    napi_create_string_utf8(env, result.c_str(), result.size(), &out);
    return out;
}

napi_value UpdateLayerEffect(napi_env env, napi_callback_info info) {
    size_t argc = 3;
    napi_value argv[3] = {nullptr, nullptr, nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    if (argc < 3) {
        return Throw(env, "updateLayerEffect(documentId, layerId, paramsJson) needs three arguments");
    }
    std::string params;
    if (!GetString(env, argv[2], params)) {
        return Throw(env, "updateLayerEffect: paramsJson must be a string");
    }
    std::string result = TakeString(pc_layer_effects_apply(Session(), GetU64(env, argv[0]), GetU64(env, argv[1]),
                                                          reinterpret_cast<const uint8_t *>(params.data()), params.size()));
    if (!IsOk(result)) {
        return Throw(env, ErrorOf(result));
    }
    napi_value out = nullptr;
    napi_create_string_utf8(env, result.c_str(), result.size(), &out);
    return out;
}

napi_value RenderPixels(napi_env env, napi_callback_info info) {
    size_t argc = 5;
    napi_value argv[5] = {nullptr, nullptr, nullptr, nullptr, nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    if (argc < 5) {
        return Throw(env, "renderPixels(documentId, x, y, w, h) needs five arguments");
    }
    int32_t x = 0;
    int32_t y = 0;
    int32_t w = 0;
    int32_t h = 0;
    napi_get_value_int32(env, argv[1], &x);
    napi_get_value_int32(env, argv[2], &y);
    napi_get_value_int32(env, argv[3], &w);
    napi_get_value_int32(env, argv[4], &h);
    PcBuffer *out = nullptr;
    std::string result = TakeString(pc_render_pixels(Session(), GetU64(env, argv[0]), x, y, w, h, &out));
    if (!IsOk(result)) {
        if (out != nullptr) {
            pc_buffer_free(out);
        }
        return Throw(env, ErrorOf(result));
    }
    return MakeArrayBuffer(env, out);
}

napi_value Revision(napi_env env, napi_callback_info info) {
    size_t argc = 1;
    napi_value argv[1] = {nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    std::string result = TakeString(pc_revision(Session(), GetU64(env, argv[0])));
    if (!IsOk(result)) {
        return Throw(env, ErrorOf(result));
    }
    napi_value out = nullptr;
    napi_create_string_utf8(env, result.c_str(), result.size(), &out);
    return out;
}

napi_value LayerMaskGet(napi_env env, napi_callback_info info) {
    size_t argc = 2;
    napi_value argv[2] = {nullptr, nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    if (argc < 2) {
        return Throw(env, "layerMaskGet(documentId, layerId) needs two arguments");
    }
    std::string result = TakeString(pc_layer_mask_get(Session(), GetU64(env, argv[0]), GetU64(env, argv[1])));
    if (!IsOk(result)) {
        return Throw(env, ErrorOf(result));
    }
    napi_value out = nullptr;
    napi_create_string_utf8(env, result.c_str(), result.size(), &out);
    return out;
}

napi_value LayerMaskAdd(napi_env env, napi_callback_info info) {
    size_t argc = 3;
    napi_value argv[3] = {nullptr, nullptr, nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    if (argc < 3) {
        return Throw(env, "layerMaskAdd(documentId, layerId, fill) needs three arguments");
    }
    std::string fill;
    if (!GetString(env, argv[2], fill)) {
        return Throw(env, "layerMaskAdd: fill must be a string");
    }
    std::string result = TakeString(pc_layer_mask_add(Session(), GetU64(env, argv[0]), GetU64(env, argv[1]),
                                                      reinterpret_cast<const uint8_t *>(fill.data()), fill.size()));
    if (!IsOk(result)) {
        return Throw(env, ErrorOf(result));
    }
    napi_value out = nullptr;
    napi_create_string_utf8(env, result.c_str(), result.size(), &out);
    return out;
}

napi_value LayerMaskSet(napi_env env, napi_callback_info info) {
    size_t argc = 3;
    napi_value argv[3] = {nullptr, nullptr, nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    if (argc < 3) {
        return Throw(env, "layerMaskSet(documentId, layerId, paramsJson) needs three arguments");
    }
    std::string params;
    if (!GetString(env, argv[2], params)) {
        return Throw(env, "layerMaskSet: paramsJson must be a string");
    }
    std::string result = TakeString(pc_layer_mask_set(Session(), GetU64(env, argv[0]), GetU64(env, argv[1]),
                                                      reinterpret_cast<const uint8_t *>(params.data()), params.size()));
    if (!IsOk(result)) {
        return Throw(env, ErrorOf(result));
    }
    napi_value out = nullptr;
    napi_create_string_utf8(env, result.c_str(), result.size(), &out);
    return out;
}

napi_value LayerMaskFillFromSelection(napi_env env, napi_callback_info info) {
    size_t argc = 3;
    napi_value argv[3] = {nullptr, nullptr, nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    if (argc < 3) {
        return Throw(env, "layerMaskFillFromSelection(documentId, layerId, mode) needs three arguments");
    }
    std::string mode;
    if (!GetString(env, argv[2], mode)) {
        return Throw(env, "layerMaskFillFromSelection: mode must be a string");
    }
    std::string result = TakeString(pc_layer_mask_fill_from_selection(Session(), GetU64(env, argv[0]), GetU64(env, argv[1]),
                                                                     reinterpret_cast<const uint8_t *>(mode.data()), mode.size()));
    if (!IsOk(result)) {
        return Throw(env, ErrorOf(result));
    }
    napi_value out = nullptr;
    napi_create_string_utf8(env, result.c_str(), result.size(), &out);
    return out;
}

napi_value LayerMaskApply(napi_env env, napi_callback_info info) {
    size_t argc = 2;
    napi_value argv[2] = {nullptr, nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    if (argc < 2) {
        return Throw(env, "layerMaskApply(documentId, layerId) needs two arguments");
    }
    std::string result = TakeString(pc_layer_mask_apply(Session(), GetU64(env, argv[0]), GetU64(env, argv[1])));
    if (!IsOk(result)) {
        return Throw(env, ErrorOf(result));
    }
    napi_value out = nullptr;
    napi_create_string_utf8(env, result.c_str(), result.size(), &out);
    return out;
}

napi_value LayerMaskRemove(napi_env env, napi_callback_info info) {
    size_t argc = 2;
    napi_value argv[2] = {nullptr, nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    if (argc < 2) {
        return Throw(env, "layerMaskRemove(documentId, layerId) needs two arguments");
    }
    std::string result = TakeString(pc_layer_mask_remove(Session(), GetU64(env, argv[0]), GetU64(env, argv[1])));
    if (!IsOk(result)) {
        return Throw(env, ErrorOf(result));
    }
    napi_value out = nullptr;
    napi_create_string_utf8(env, result.c_str(), result.size(), &out);
    return out;
}

napi_value LayerGroup(napi_env env, napi_callback_info info) {
    size_t argc = 3;
    napi_value argv[3] = {nullptr, nullptr, nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    if (argc < 3) {
        return Throw(env, "layerGroup(documentId, idsJson, name) needs three arguments");
    }
    std::string ids;
    std::string name;
    if (!GetString(env, argv[1], ids) || !GetString(env, argv[2], name)) {
        return Throw(env, "layerGroup: idsJson and name must be strings");
    }
    std::string result = TakeString(pc_layer_group(Session(), GetU64(env, argv[0]),
                                                   reinterpret_cast<const uint8_t *>(ids.data()), ids.size(),
                                                   reinterpret_cast<const uint8_t *>(name.data()), name.size()));
    if (!IsOk(result)) {
        return Throw(env, ErrorOf(result));
    }
    napi_value out = nullptr;
    napi_create_string_utf8(env, result.c_str(), result.size(), &out);
    return out;
}

napi_value LayerUngroup(napi_env env, napi_callback_info info) {
    size_t argc = 2;
    napi_value argv[2] = {nullptr, nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    if (argc < 2) {
        return Throw(env, "layerUngroup(documentId, layerId) needs two arguments");
    }
    std::string result = TakeString(pc_layer_ungroup(Session(), GetU64(env, argv[0]), GetU64(env, argv[1])));
    if (!IsOk(result)) {
        return Throw(env, ErrorOf(result));
    }
    napi_value out = nullptr;
    napi_create_string_utf8(env, result.c_str(), result.size(), &out);
    return out;
}

napi_value LayerExpanded(napi_env env, napi_callback_info info) {
    size_t argc = 3;
    napi_value argv[3] = {nullptr, nullptr, nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    if (argc < 3) {
        return Throw(env, "layerExpanded(documentId, layerId, expanded) needs three arguments");
    }
    // 同时接受 bool 与 number（ArkTS 侧可能传 0/1），两者都映射到 C ABI 的 u8。
    napi_valuetype type = napi_undefined;
    bool expanded = true;
    if (napi_typeof(env, argv[2], &type) == napi_ok && type == napi_number) {
        double number = 1.0;
        napi_get_value_double(env, argv[2], &number);
        expanded = number != 0.0;
    } else {
        napi_get_value_bool(env, argv[2], &expanded);
    }
    std::string result = TakeString(pc_layer_expanded(Session(), GetU64(env, argv[0]), GetU64(env, argv[1]), expanded ? 1 : 0));
    if (!IsOk(result)) {
        return Throw(env, ErrorOf(result));
    }
    napi_value out = nullptr;
    napi_create_string_utf8(env, result.c_str(), result.size(), &out);
    return out;
}

napi_value LayerClipped(napi_env env, napi_callback_info info) {
    size_t argc = 3;
    napi_value argv[3] = {nullptr, nullptr, nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    if (argc < 3) {
        return Throw(env, "layerClipped(documentId, layerId, clipped) needs three arguments");
    }
    // 与 layerExpanded 一致：同时接受 bool 与 number（ArkTS 侧可能传 0/1）。
    napi_valuetype type = napi_undefined;
    bool clipped = true;
    if (napi_typeof(env, argv[2], &type) == napi_ok && type == napi_number) {
        double number = 1.0;
        napi_get_value_double(env, argv[2], &number);
        clipped = number != 0.0;
    } else {
        napi_get_value_bool(env, argv[2], &clipped);
    }
    std::string result = TakeString(pc_layer_clipped(Session(), GetU64(env, argv[0]), GetU64(env, argv[1]), clipped ? 1 : 0));
    if (!IsOk(result)) {
        return Throw(env, ErrorOf(result));
    }
    napi_value out = nullptr;
    napi_create_string_utf8(env, result.c_str(), result.size(), &out);
    return out;
}

// 裁剪到文档坐标矩形（x0,y0,x1,y1，x1/y1 独占）。返回 DocumentInfo 的 JSON 字符串。
napi_value DocumentCrop(napi_env env, napi_callback_info info) {
    size_t argc = 5;
    napi_value argv[5] = {nullptr, nullptr, nullptr, nullptr, nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    if (argc < 5) {
        return Throw(env, "documentCrop(documentId, x0, y0, x1, y1) needs five arguments");
    }
    int32_t x0 = 0;
    int32_t y0 = 0;
    int32_t x1 = 0;
    int32_t y1 = 0;
    napi_get_value_int32(env, argv[1], &x0);
    napi_get_value_int32(env, argv[2], &y0);
    napi_get_value_int32(env, argv[3], &x1);
    napi_get_value_int32(env, argv[4], &y1);
    std::string result = TakeString(pc_document_crop(Session(), GetU64(env, argv[0]), x0, y0, x1, y1));
    if (!IsOk(result)) {
        return Throw(env, ErrorOf(result));
    }
    napi_value out = nullptr;
    napi_create_string_utf8(env, result.c_str(), result.size(), &out);
    return out;
}

// 画布尺寸调整；anchor ∈ 0..8（row*3+col，4 = 中心）。返回 DocumentInfo 的 JSON 字符串。
napi_value DocumentResizeCanvas(napi_env env, napi_callback_info info) {
    size_t argc = 4;
    napi_value argv[4] = {nullptr, nullptr, nullptr, nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    if (argc < 4) {
        return Throw(env, "documentResizeCanvas(documentId, width, height, anchor) needs four arguments");
    }
    int32_t width = 0;
    int32_t height = 0;
    int32_t anchor = 0;
    napi_get_value_int32(env, argv[1], &width);
    napi_get_value_int32(env, argv[2], &height);
    napi_get_value_int32(env, argv[3], &anchor);
    std::string result = TakeString(pc_document_resize_canvas(Session(), GetU64(env, argv[0]), width, height, anchor));
    if (!IsOk(result)) {
        return Throw(env, ErrorOf(result));
    }
    napi_value out = nullptr;
    napi_create_string_utf8(env, result.c_str(), result.size(), &out);
    return out;
}

// 读取参考线与标尺原点（只读）。返回 {"horizontal":[..],"vertical":[..],"origin":[x,y]}。
napi_value GuidesGet(napi_env env, napi_callback_info info) {
    size_t argc = 1;
    napi_value argv[1] = {nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    if (argc < 1) {
        return Throw(env, "guidesGet(documentId) needs one argument");
    }
    std::string result = TakeString(pc_guides_get(Session(), GetU64(env, argv[0])));
    if (!IsOk(result)) {
        return Throw(env, ErrorOf(result));
    }
    napi_value out = nullptr;
    napi_create_string_utf8(env, result.c_str(), result.size(), &out);
    return out;
}

// 全量替换参考线与标尺原点；三个键均可选，缺省键表示不改该轴。
napi_value GuidesSet(napi_env env, napi_callback_info info) {
    size_t argc = 2;
    napi_value argv[2] = {nullptr, nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    if (argc < 2) {
        return Throw(env, "guidesSet(documentId, paramsJson) needs two arguments");
    }
    std::string params;
    if (!GetString(env, argv[1], params)) {
        return Throw(env, "guidesSet: paramsJson must be a string");
    }
    std::string result = TakeString(pc_guides_set(Session(), GetU64(env, argv[0]),
                                                  reinterpret_cast<const uint8_t *>(params.data()), params.size()));
    if (!IsOk(result)) {
        return Throw(env, ErrorOf(result));
    }
    napi_value out = nullptr;
    napi_create_string_utf8(env, result.c_str(), result.size(), &out);
    return out;
}

// 图层自由变换：参数相对当前状态（scaleX/scaleY 倍率、angle 度数增量、dx/dy 像素增量）。
napi_value LayerTransform(napi_env env, napi_callback_info info) {
    size_t argc = 3;
    napi_value argv[3] = {nullptr, nullptr, nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    if (argc < 3) {
        return Throw(env, "layerTransform(documentId, layerId, paramsJson) needs three arguments");
    }
    std::string params;
    if (!GetString(env, argv[2], params)) {
        return Throw(env, "layerTransform: paramsJson must be a string");
    }
    std::string result = TakeString(pc_layer_transform(Session(), GetU64(env, argv[0]), GetU64(env, argv[1]),
                                                      reinterpret_cast<const uint8_t *>(params.data()), params.size()));
    if (!IsOk(result)) {
        return Throw(env, ErrorOf(result));
    }
    napi_value out = nullptr;
    napi_create_string_utf8(env, result.c_str(), result.size(), &out);
    return out;
}

napi_value Init(napi_env env, napi_value exports) {
    napi_property_descriptor desc[] = {
        {"openImage", nullptr, OpenImage, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"newDocument", nullptr, NewDocument, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"documentTurn", nullptr, DocumentTurn, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"history", nullptr, History, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"undoSteps", nullptr, UndoSteps, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"redoSteps", nullptr, RedoSteps, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"close", nullptr, Close, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"info", nullptr, Info, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"preview", nullptr, Preview, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"layerThumbnail", nullptr, LayerThumbnail, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"render", nullptr, Render, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"paintStroke", nullptr, PaintStroke, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"select", nullptr, Select, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"selectionInfo", nullptr, SelectionInfo, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"filterApply", nullptr, FilterApply, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"shapeCreate", nullptr, ShapeCreate, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"fontRegister", nullptr, FontRegister, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"textCreate", nullptr, TextCreate, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"layerEditGet", nullptr, LayerEditGet, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"layerEditApply", nullptr, LayerEditApply, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"setLayerVisible", nullptr, LayerVisible, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"setLayerOpacity", nullptr, LayerOpacity, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"setLayerFillOpacity", nullptr, LayerFillOpacity, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"setLayerBlend", nullptr, LayerBlend, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"createLayer", nullptr, LayerCreate, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"renameLayer", nullptr, LayerRename, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"moveLayer", nullptr, LayerMove, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"duplicateLayer", nullptr, LayerDuplicate, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"duplicateLayerMany", nullptr, LayerDuplicateMany, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"mergeLayer", nullptr, LayerMerge, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"deleteLayer", nullptr, LayerDelete, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"layerDeleteMany", nullptr, LayerDeleteMany, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"addAdjustment", nullptr, AdjustmentAdd, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"updateAdjustment", nullptr, AdjustmentUpdate, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"getAdjustment", nullptr, AdjustmentGet, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"undo", nullptr, Undo, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"redo", nullptr, Redo, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"exportImage", nullptr, ExportImage, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"layerEffects", nullptr, LayerEffects, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"updateLayerEffect", nullptr, UpdateLayerEffect, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"renderPixels", nullptr, RenderPixels, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"revision", nullptr, Revision, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"layerMaskGet", nullptr, LayerMaskGet, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"layerMaskAdd", nullptr, LayerMaskAdd, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"layerMaskSet", nullptr, LayerMaskSet, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"layerMaskFillFromSelection", nullptr, LayerMaskFillFromSelection, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"layerMaskApply", nullptr, LayerMaskApply, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"layerMaskRemove", nullptr, LayerMaskRemove, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"layerGroup", nullptr, LayerGroup, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"layerUngroup", nullptr, LayerUngroup, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"layerExpanded", nullptr, LayerExpanded, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"layerClipped", nullptr, LayerClipped, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"documentCrop", nullptr, DocumentCrop, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"documentResizeCanvas", nullptr, DocumentResizeCanvas, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"guidesGet", nullptr, GuidesGet, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"guidesSet", nullptr, GuidesSet, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"layerTransform", nullptr, LayerTransform, nullptr, nullptr, nullptr, napi_default, nullptr},
    };
    napi_define_properties(env, exports, sizeof(desc) / sizeof(desc[0]), desc);
    return exports;
}

napi_module g_module = {
    .nm_version = 1,
    .nm_flags = 0,
    .nm_filename = nullptr,
    .nm_register_func = Init,
    .nm_modname = "entry",
    .nm_priv = nullptr,
    .reserved = {0},
};

}  // namespace

extern "C" __attribute__((constructor)) void RegisterModule() {
    napi_module_register(&g_module);
}