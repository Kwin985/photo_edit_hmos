/*
 * Engine 层（entry/src/main/ets/engine/Engine.ets）无 UI 回归。
 *
 * 只覆盖不依赖 UI 会话状态的纯逻辑：错误分支、纯函数 flattenLayers、以及用
 * newDocument 在内存里建文档后验证 guides 读写与蒙版初始态。
 *
 * 手动运行步骤（命令行可用时）：
 *   1) 构建测试包（工程根）：
 *      hvigorw assembleHap --mode module -p module=entry@ohosTest -p buildMode=debug
 *   2) 安装测试 HAP 后执行：
 *      hdc -t 127.0.0.1:5555 shell aa test -b com.kwin.photocraft -m entry_test \
 *        -s unittest OpenHarmonyTestRunner -s class EngineTest
 *   3) 输出中出现 EngineTest 的用例结果即表示执行；若报 test runner 不存在，
 *      说明工程缺少 entry/src/ohosTest/ets/testrunner/OpenHarmonyTestRunner.ets。
 */
import { describe, it, expect } from '@ohos/hypium';
import { Engine, flattenLayers, visibleRectLeavesTile, LayerInfo } from '../../../main/ets/engine/Engine';

/** 执行动作并返回其抛出的错误信息；未抛错时返回空串。 */
function caughtMessage(action: () => void): string {
  try {
    action();
  } catch (error) {
    return error instanceof Error ? error.message : String(error);
  }
  return '';
}

/** 构造最简图层，供 flattenLayers 的结构化断言使用。 */
function makeLayer(id: number, isGroup: boolean, expanded: boolean, children: LayerInfo[]): LayerInfo {
  return {
    id: id,
    name: 'Layer' + id,
    kind: isGroup ? 'Group' : 'Raster',
    visible: true,
    opacity: 1,
    fillOpacity: 1,
    blend: 'Normal',
    clipped: false,
    depth: 0,
    isGroup: isGroup,
    expanded: expanded,
    children: children
  };
}

export default function engineTest() {
  describe('EngineTest', () => {
    it('infoUnknownDocumentThrows', 0, () => {
      // Engine.ets:627-633 -> pc_info 对未打开文档返回 fail("document N is not open")，故 ArkTS 抛错。
      const message: string = caughtMessage(() => {
        Engine.info(0);
      });
      expect(message.length > 0).assertTrue();
    });

    it('guidesGetUnknownDocumentThrows', 0, () => {
      // Engine.ets:1010-1016 -> pc_guides_get 的 s.entry(id) 失败即 fail，不返回默认结构。
      const message: string = caughtMessage(() => {
        Engine.guidesGet(0);
      });
      expect(message.length > 0).assertTrue();
    });

    it('layerTransformInvalidScaleThrows', 0, () => {
      // 参数校验先于文档查找（native lib.rs:4931-4937），scaleX=0 不在 0.01..100 内。
      const message: string = caughtMessage(() => {
        Engine.layerTransform(0, 1, '{"scaleX":0}');
      });
      expect(message.length > 0).assertTrue();
      expect(message.indexOf('invalid transform') >= 0).assertTrue();
    });

    it('layerTransformMalformedJsonThrows', 0, () => {
      // JSON 解析失败与数值越界共用同一文案（native lib.rs:4919-4922）。
      const message: string = caughtMessage(() => {
        Engine.layerTransform(0, 1, 'not-json');
      });
      expect(message.length > 0).assertTrue();
      expect(message.indexOf('invalid transform') >= 0).assertTrue();
    });

    it('layerMaskGetUnknownDocumentThrows', 0, () => {
      // Engine.ets:895-901 -> pc_layer_mask_get 先查文档（lib.rs:2111-2114），文档不存在直接 fail。
      const message: string = caughtMessage(() => {
        Engine.layerMaskGet(0, 1);
      });
      expect(message.length > 0).assertTrue();
      expect(message.indexOf('document 0 is not open') >= 0).assertTrue();
    });

    it('flattenLayersEmptyStaysEmpty', 0, () => {
      expect(flattenLayers([], 0).length).assertEqual(0);
    });

    it('flattenLayersNestedDepthAndCollapse', 0, () => {
      const leafA: LayerInfo = makeLayer(2, false, true, []);
      const leafB: LayerInfo = makeLayer(3, false, true, []);
      const openGroup: LayerInfo = makeLayer(1, true, true, [leafA, leafB]);

      // 展开的组：组行 + 两个子行，子行深度 = 父深度 + 1（Engine.ets:1046-1068）。
      const openRows: LayerInfo[] = flattenLayers([openGroup], 0);
      expect(openRows.length).assertEqual(3);
      expect(openRows[0].id).assertEqual(1);
      expect(openRows[0].depth).assertEqual(0);
      expect(openRows[1].depth).assertEqual(1);
      expect(openRows[2].depth).assertEqual(1);

      // 折叠的组：保留组行但不展开子层（Engine.ets:1064）。
      const collapsedGroup: LayerInfo = makeLayer(1, true, false, [leafA, leafB]);
      const collapsedRows: LayerInfo[] = flattenLayers([collapsedGroup], 0);
      expect(collapsedRows.length).assertEqual(1);
      expect(collapsedRows[0].id).assertEqual(1);
    });

    it('guidesRoundTripOnNewDocument', 0, () => {
      // newDocument 在 native 内存里建文档，不依赖 UI 会话；结束必须 close 防泄漏。
      const created = Engine.newDocument(64, 48, 'guides-test');
      const docId: number = created.id;
      try {
        const initial = Engine.guidesGet(docId);
        expect(initial.origin.length).assertEqual(2);

        const updated = Engine.guidesSet(docId, '{"horizontal":[20,10],"origin":[5,6]}');
        expect(updated.horizontal.length).assertEqual(2);
        // native 对参考线做排序去重（lib.rs 的 sorted_unique）。
        expect(updated.horizontal[0]).assertEqual(10);
        expect(updated.horizontal[1]).assertEqual(20);
        expect(updated.origin[0]).assertEqual(5);
        expect(updated.origin[1]).assertEqual(6);

        const readBack = Engine.guidesGet(docId);
        expect(readBack.horizontal.length).assertEqual(2);
        expect(readBack.horizontal[1]).assertEqual(20);
        expect(readBack.origin[0]).assertEqual(5);
      } finally {
        Engine.close(docId);
      }
    });

    it('layerMaskInitialStateOnNewDocument', 0, () => {
      // 新建文档默认只有 Background 层；任何存在图层都可读蒙版，无蒙版时返回缺省值
      // （Engine.ets:544-552 的 normalizeLayerMask）。
      const created = Engine.newDocument(64, 48, 'mask-test');
      const docId: number = created.id;
      try {
        const info = Engine.info(docId);
        expect(info.layers.length).assertEqual(1);
        const layerId: number = info.layers[0].id;

        const mask = Engine.layerMaskGet(docId, layerId);
        expect(mask.present).assertFalse();
        expect(mask.enabled).assertTrue();
        expect(mask.linked).assertTrue();
        expect(mask.density).assertEqual(1);
        expect(mask.feather).assertEqual(0);
      } finally {
        Engine.close(docId);
      }
    });

    it('guidesUndoRedoChainStepsIndividually', 0, () => {
      // 多条参考线撤销链逐项验（B5）：每次 guidesSet 占一个 checkpoint（lib.rs:4699），
      // undo 必须逐项回退、redo 逐项恢复；同值是空操作（lib.rs:4695），故每次值不同。
      const created = Engine.newDocument(64, 48, 'guides-undo-chain');
      const docId: number = created.id;
      try {
        // 三次全量替换 → 三个独立 checkpoint。
        Engine.guidesSet(docId, '{"horizontal":[10]}');
        Engine.guidesSet(docId, '{"horizontal":[10,20]}');
        Engine.guidesSet(docId, '{"horizontal":[10,20,30]}');
        expect(Engine.guidesGet(docId).horizontal.length).assertEqual(3);

        // undo 逐项回退：30 → 20 → 10 → 空。
        Engine.undo(docId);
        expect(Engine.guidesGet(docId).horizontal.length).assertEqual(2);
        Engine.undo(docId);
        expect(Engine.guidesGet(docId).horizontal.length).assertEqual(1);
        Engine.undo(docId);
        expect(Engine.guidesGet(docId).horizontal.length).assertEqual(0);

        // redo 逐项恢复：空 → 10 → 20 → 30。
        Engine.redo(docId);
        expect(Engine.guidesGet(docId).horizontal.length).assertEqual(1);
        Engine.redo(docId);
        expect(Engine.guidesGet(docId).horizontal.length).assertEqual(2);
        Engine.redo(docId);
        expect(Engine.guidesGet(docId).horizontal.length).assertEqual(3);
      } finally {
        Engine.close(docId);
      }
    });

    it('layerTransformScaleBoundary10', 0, () => {
      // B7：UI 缩放滑块下限 10% → 引擎 scaleX=0.1，在 0.01..100 内应成功（lib.rs:5098）。
      const created = Engine.newDocument(64, 48, 'transform-scale-10');
      const docId: number = created.id;
      try {
        const layerId: number = Engine.info(docId).layers[0].id;
        const result = Engine.layerTransform(docId, layerId, '{"scaleX":0.1}');
        expect(result.bounds.length).assertEqual(4);
        expect(result.scaleX).assertEqual(0.1);
      } finally {
        Engine.close(docId);
      }
    });

    it('layerTransformScaleBoundary400', 0, () => {
      // B7：UI 缩放滑块上限 400% → 引擎 scaleX=4.0，在 0.01..100 内应成功（lib.rs:5098）。
      const created = Engine.newDocument(64, 48, 'transform-scale-400');
      const docId: number = created.id;
      try {
        const layerId: number = Engine.info(docId).layers[0].id;
        const result = Engine.layerTransform(docId, layerId, '{"scaleX":4.0}');
        expect(result.bounds.length).assertEqual(4);
        expect(result.scaleX).assertEqual(4.0);
      } finally {
        Engine.close(docId);
      }
    });

    it('layerTransformScaleBelowMinThrows', 0, () => {
      // B7：scaleX=0.009 < 0.01 下限，应抛 "invalid transform"（lib.rs:5098-5104）。
      const created = Engine.newDocument(64, 48, 'transform-scale-below-min');
      const docId: number = created.id;
      try {
        const layerId: number = Engine.info(docId).layers[0].id;
        const message: string = caughtMessage(() => {
          Engine.layerTransform(docId, layerId, '{"scaleX":0.009}');
        });
        expect(message.indexOf('invalid transform') >= 0).assertTrue();
      } finally {
        Engine.close(docId);
      }
    });

    it('layerTransformAngleBoundary180', 0, () => {
      // B7：UI 角度滑块上限 180°，在 -3600..3600 内应成功（lib.rs:5100）。
      const created = Engine.newDocument(64, 48, 'transform-angle-180');
      const docId: number = created.id;
      try {
        const layerId: number = Engine.info(docId).layers[0].id;
        const result = Engine.layerTransform(docId, layerId, '{"angle":180}');
        expect(result.bounds.length).assertEqual(4);
        expect(result.angle).assertEqual(180);
      } finally {
        Engine.close(docId);
      }
    });

    it('layerTransformAngleBoundary200', 0, () => {
      // B7：angle=200° 超出 UI 滑块范围但在引擎 -3600..3600 内，应成功（lib.rs:5100）。
      const created = Engine.newDocument(64, 48, 'transform-angle-200');
      const docId: number = created.id;
      try {
        const layerId: number = Engine.info(docId).layers[0].id;
        const result = Engine.layerTransform(docId, layerId, '{"angle":200}');
        expect(result.bounds.length).assertEqual(4);
        expect(result.angle).assertEqual(200);
      } finally {
        Engine.close(docId);
      }
    });

    it('layerTransformAngleBoundaryNeg180', 0, () => {
      // B7：UI 角度滑块下限 -180°，应成功。
      const created = Engine.newDocument(64, 48, 'transform-angle-neg180');
      const docId: number = created.id;
      try {
        const layerId: number = Engine.info(docId).layers[0].id;
        const result = Engine.layerTransform(docId, layerId, '{"angle":-180}');
        expect(result.bounds.length).assertEqual(4);
        expect(result.angle).assertEqual(-180);
      } finally {
        Engine.close(docId);
      }
    });

    it('layerTransformAngleBoundaryNeg200', 0, () => {
      // angle=-200° 超出 UI 滑块范围但在引擎范围内，应成功。
      const created = Engine.newDocument(64, 48, 'transform-angle-neg200');
      const docId: number = created.id;
      try {
        const layerId: number = Engine.info(docId).layers[0].id;
        const result = Engine.layerTransform(docId, layerId, '{"angle":-200}');
        expect(result.bounds.length).assertEqual(4);
        expect(result.angle).assertEqual(-200);
      } finally {
        Engine.close(docId);
      }
    });

    // ──────────────────────────────────────────────────────────────────────
    // A1 矢量绘图（1.0.2）：vectorPathCreate/Edit/Rasterize/ToSelection
    // ──────────────────────────────────────────────────────────────────────

    it('vectorPathCreateOnNewDocument', 0, () => {
      const created = Engine.newDocument(64, 48, 'vp-create');
      const docId: number = created.id;
      try {
        const out = Engine.vectorPathCreate(docId, '{}');
        expect(out.layerId > 0).assertTrue();
        expect(out.info.layers.length >= 1).assertTrue();
      } finally {
        Engine.close(docId);
      }
    });

    it('vectorPathEditAddAnchor', 0, () => {
      const created = Engine.newDocument(64, 48, 'vp-add-anchor');
      const docId: number = created.id;
      try {
        const out = Engine.vectorPathCreate(docId, '{}');
        const info = Engine.vectorPathEdit(docId,
          '{"op":"addAnchor","layerId":' + out.layerId + ',"subpathIndex":0,"knotIndex":0,"anchor":[10,20]}');
        expect(info.layers.length >= 1).assertTrue();
      } finally {
        Engine.close(docId);
      }
    });

    it('vectorPathEditUnsupportedOpThrows', 0, () => {
      const created = Engine.newDocument(64, 48, 'vp-bad-op');
      const docId: number = created.id;
      try {
        const out = Engine.vectorPathCreate(docId, '{}');
        const message: string = caughtMessage(() => {
          Engine.vectorPathEdit(docId, '{"op":"bogus","layerId":' + out.layerId + '}');
        });
        expect(message.indexOf('unsupported vector path op') >= 0).assertTrue();
      } finally {
        Engine.close(docId);
      }
    });

    it('vectorPathEditNonShapeLayerThrows', 0, () => {
      // Background 层 kind=Pixel，vectorPathEdit 应报「not a vector/shape layer」。
      const created = Engine.newDocument(64, 48, 'vp-non-shape');
      const docId: number = created.id;
      try {
        const layerId: number = Engine.info(docId).layers[0].id;
        const message: string = caughtMessage(() => {
          Engine.vectorPathEdit(docId,
            '{"op":"addAnchor","layerId":' + layerId + ',"subpathIndex":0,"knotIndex":0,"anchor":[1,2]}');
        });
        expect(message.indexOf('not a vector/shape layer') >= 0).assertTrue();
      } finally {
        Engine.close(docId);
      }
    });

    it('vectorPathRasterizeOnCreatedPath', 0, () => {
      const created = Engine.newDocument(64, 48, 'vp-rasterize');
      const docId: number = created.id;
      try {
        const out = Engine.vectorPathCreate(docId, '{}');
        const info = Engine.vectorPathRasterize(docId, '{"layerId":' + out.layerId + '}');
        expect(info.layers.length >= 1).assertTrue();
      } finally {
        Engine.close(docId);
      }
    });

    it('vectorPathToSelectionOnClosedPath', 0, () => {
      // 闭合三角形路径转选区，填充面积非零 → active=true。
      const created = Engine.newDocument(64, 48, 'vp-to-sel');
      const docId: number = created.id;
      try {
        const out = Engine.vectorPathCreate(docId, '{}');
        const layerId: number = out.layerId;
        Engine.vectorPathEdit(docId,
          '{"op":"addAnchor","layerId":' + layerId + ',"subpathIndex":0,"knotIndex":0,"anchor":[10,10]}');
        Engine.vectorPathEdit(docId,
          '{"op":"addAnchor","layerId":' + layerId + ',"subpathIndex":0,"knotIndex":1,"anchor":[50,10]}');
        Engine.vectorPathEdit(docId,
          '{"op":"addAnchor","layerId":' + layerId + ',"subpathIndex":0,"knotIndex":2,"anchor":[30,50]}');
        Engine.vectorPathEdit(docId,
          '{"op":"setClosed","layerId":' + layerId + ',"subpathIndex":0,"closed":true}');
        const sel = Engine.vectorPathToSelection(docId, layerId, 'replace', 0);
        expect(sel.active).assertTrue();
      } finally {
        Engine.close(docId);
      }
    });

    it('vectorPathToSelectionInvalidModeThrows', 0, () => {
      const created = Engine.newDocument(64, 48, 'vp-bad-mode');
      const docId: number = created.id;
      try {
        const out = Engine.vectorPathCreate(docId, '{}');
        const message: string = caughtMessage(() => {
          Engine.vectorPathToSelection(docId, out.layerId, 'bogus', 0);
        });
        expect(message.indexOf('mode must be') >= 0).assertTrue();
      } finally {
        Engine.close(docId);
      }
    });

    it('vectorPathCreateOnClosedDocumentThrows', 0, () => {
      const created = Engine.newDocument(64, 48, 'vp-closed');
      const docId: number = created.id;
      Engine.close(docId);
      const message: string = caughtMessage(() => {
        Engine.vectorPathCreate(docId, '{}');
      });
      expect(message.length > 0).assertTrue();
    });

    // ──────────────────────────────────────────────────────────────────────
    // D2 智能抠图（1.0.2）：cutoutRemoveBackground 错误分支
    // ──────────────────────────────────────────────────────────────────────

    it('cutoutRejectsNonRasterLayer', 0, () => {
      // 矢量层调抠图报「仅支持像素图层」。
      const created = Engine.newDocument(64, 48, 'cutout-shape');
      const docId: number = created.id;
      try {
        const out = Engine.vectorPathCreate(docId, '{}');
        const message: string = caughtMessage(() => {
          Engine.cutoutRemoveBackground(docId, out.layerId, true);
        });
        expect(message.indexOf('像素图层') >= 0).assertTrue();
      } finally {
        Engine.close(docId);
      }
    });

    it('cutoutUnknownDocumentThrows', 0, () => {
      const created = Engine.newDocument(64, 48, 'cutout-closed');
      const docId: number = created.id;
      Engine.close(docId);
      const message: string = caughtMessage(() => {
        Engine.cutoutRemoveBackground(docId, 1, true);
      });
      expect(message.length > 0).assertTrue();
    });

    // ──────────────────────────────────────────────────────────────────────
    // G1 PSD 导出验证（1.1.0）：证明 PSD 导出链路通（签名检查）
    // ──────────────────────────────────────────────────────────────────────

    it('exportImagePsdSignature', 0, () => {
      // PSD 文件以 '8BPS' 签名开头；验证 pc_export 的 psd 分支产出合法 PSD 字节。
      const created = Engine.newDocument(64, 48, 'export-psd');
      const docId: number = created.id;
      try {
        const bytes = Engine.exportImage(docId, 'psd', 90);
        expect(bytes.byteLength > 0).assertTrue();
        const view = new Uint8Array(bytes);
        expect(view[0]).assertEqual(0x38); // '8'
        expect(view[1]).assertEqual(0x42); // 'B'
        expect(view[2]).assertEqual(0x50); // 'P'
        expect(view[3]).assertEqual(0x53); // 'S'
      } finally {
        Engine.close(docId);
      }
    });

    it('exportImagePngNonEmpty', 0, () => {
      const created = Engine.newDocument(64, 48, 'export-png');
      const docId: number = created.id;
      try {
        const bytes = Engine.exportImage(docId, 'png', 90);
        expect(bytes.byteLength > 0).assertTrue();
      } finally {
        Engine.close(docId);
      }
    });

    // ──────────────────────────────────────────────────────────────────────
    // G2 复制可见验证（1.1.0）：pc_copy_visible 链路
    // ──────────────────────────────────────────────────────────────────────

    it('copyVisibleCreatesNewLayer', 0, () => {
      // 新建文档默认 1 个可见 Background 层；copyVisible 合成可见内容到新图层。
      const created = Engine.newDocument(64, 48, 'copy-visible');
      const docId: number = created.id;
      try {
        const before = Engine.info(docId);
        expect(before.layers.length).assertEqual(1);
        const result = Engine.copyVisible(docId);
        expect(result.layerId > 0).assertTrue();
        expect(result.info.layers.length >= 2).assertTrue();
      } finally {
        Engine.close(docId);
      }
    });

    // ──────────────────────────────────────────────────────────────────────
    // PAN-PREFETCH@1：平移越界判定纯函数（hypium 无需 UI driver）
    // ──────────────────────────────────────────────────────────────────────

    it('visibleRectInsideTileDoesNotLeave', 0, () => {
      // 可见区完全落在 tile 内，含边界相等：不应触发提前重渲染。
      expect(visibleRectLeavesTile(100, 200, 500, 600, 100, 200, 500, 600)).assertFalse();
      expect(visibleRectLeavesTile(150, 250, 450, 550, 100, 200, 500, 600)).assertFalse();
    });

    it('visibleRectOutsideTileLeavesOnEveryEdge', 0, () => {
      // 四条边分别越出，任一越界即返回 true。
      expect(visibleRectLeavesTile(99, 200, 500, 600, 100, 200, 500, 600)).assertTrue();
      expect(visibleRectLeavesTile(100, 199, 500, 600, 100, 200, 500, 600)).assertTrue();
      // tile 范围 x∈[100,600]、y∈[200,800]，右/下越界需超过该上界。
      expect(visibleRectLeavesTile(100, 200, 601, 600, 100, 200, 500, 600)).assertTrue();
      expect(visibleRectLeavesTile(100, 200, 500, 801, 100, 200, 500, 600)).assertTrue();
    });

    it('visibleRectPartiallyOverlappingTileLeaves', 0, () => {
      // 部分重叠仍视为越界（露白区的根因）。
      expect(visibleRectLeavesTile(50, 250, 400, 550, 100, 200, 500, 600)).assertTrue();
    });

    it('copyVisibleOnClosedDocumentThrows', 0, () => {
      const created = Engine.newDocument(64, 48, 'copy-visible-closed');
      const docId: number = created.id;
      Engine.close(docId);
      const message: string = caughtMessage(() => {
        Engine.copyVisible(docId);
      });
      expect(message.length > 0).assertTrue();
    });
  });
}

