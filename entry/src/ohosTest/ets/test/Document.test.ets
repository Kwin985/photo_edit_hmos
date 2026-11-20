/*
 * 文档 / 图层 / 历史 三块的无 UI 回归。
 *
 * 全部用例都在 native 内存文档上执行（`Engine.newDocument`），不依赖相册、文件选择器或
 * 画布会话；每个用例在 `finally` 里 close，避免会话内文档泄漏。
 */
import { describe, it, expect } from '@ohos/hypium';
import { Engine, DocumentInfo, LayerInfo } from '../../../main/ets/engine/Engine';

/** 执行动作并返回其抛出的错误信息；未抛错时返回空串。 */
function messageOf(action: () => void): string {
  try {
    action();
  } catch (error) {
    return error instanceof Error ? error.message : String(error);
  }
  return '';
}

/** 建一个内存文档并在 finally 中关闭，保证每个用例互不影响。 */
function withDocument(width: number, height: number, name: string, body: (id: number) => void): void {
  const created = Engine.newDocument(width, height, name);
  try {
    body(created.id);
  } finally {
    Engine.close(created.id);
  }
}

/** 在图层树中按 id 递归查找。 */
function findInLayers(layers: LayerInfo[], id: number): LayerInfo | null {
  for (const layer of layers) {
    if (layer.id === id) {
      return layer;
    }
    const hit: LayerInfo | null = findInLayers(layer.children, id);
    if (hit !== null) {
      return hit;
    }
  }
  return null;
}

function findLayer(info: DocumentInfo, id: number): LayerInfo | null {
  return findInLayers(info.layers, id);
}

/** 建三层（Background + A + B），返回自顶向下的 id 列表。 */
function threeLayerIds(id: number): number[] {
  const a: number = Engine.createLayer(id, 0, 'A').layerId;
  const b: number = Engine.createLayer(id, 0, 'B').layerId;
  return [b, a];
}

export default function documentTest() {
  describe('DocumentTest', () => {
    it('newDocumentRejectsZeroSize', 0, () => {
      // lib.rs:pc_document_new 先拦非正尺寸。
      const message = messageOf(() => {
        Engine.newDocument(0, 10, 'zero');
      });
      expect(message.indexOf('document size must be positive') >= 0).assertTrue();
    });

    it('newDocumentRejectsSideOverLimit', 0, () => {
      // MAX_CANVAS_SIDE = 16384。
      const message = messageOf(() => {
        Engine.newDocument(16385, 10, 'wide');
      });
      expect(message.indexOf('document side exceeds 16384') >= 0).assertTrue();
    });

    it('newDocumentCreatesWhiteBackground', 0, () => {
      withDocument(40, 30, 'bg', (id: number) => {
        const info: DocumentInfo = Engine.info(id);
        expect(info.width).assertEqual(40);
        expect(info.height).assertEqual(30);
        expect(info.mode).assertEqual('RGB');
        expect(info.depth).assertEqual('8 位/通道');
        expect(info.layers.length).assertEqual(1);
        expect(info.layers[0].kind).assertEqual('Pixel');

        const pixel = new Uint8Array(Engine.renderPixels(id, 0, 0, 1, 1));
        expect(pixel.length).assertEqual(4);
        expect(pixel[0]).assertEqual(255);
        expect(pixel[1]).assertEqual(255);
        expect(pixel[2]).assertEqual(255);
        expect(pixel[3]).assertEqual(255);
      });
    });

    it('newDocumentBlankNameFallsBackToUntitled', 0, () => {
      withDocument(10, 10, '   ', (id: number) => {
        expect(Engine.info(id).name).assertEqual('Untitled');
      });
    });

    it('documentTurnCw90SwapsAxes', 0, () => {
      withDocument(100, 60, 'turn', (id: number) => {
        const turned: DocumentInfo = Engine.documentTurn(id, 3);
        expect(turned.width).assertEqual(60);
        expect(turned.height).assertEqual(100);
        expect(Engine.info(id).width).assertEqual(60);
        expect(Engine.info(id).height).assertEqual(100);
      });
    });

    it('documentTurnRejectsOutOfRangeCode', 0, () => {
      withDocument(20, 20, 'turn-bad', (id: number) => {
        const message = messageOf(() => {
          Engine.documentTurn(id, 5);
        });
        expect(message.indexOf('invalid turn: expected 0..=4') >= 0).assertTrue();
        // 失败零副作用：尺寸仍是原值。
        expect(Engine.info(id).width).assertEqual(20);
      });
    });

    it('documentTurnUndoRestoresAxes', 0, () => {
      withDocument(100, 60, 'turn-undo', (id: number) => {
        Engine.documentTurn(id, 3);
        const restored: DocumentInfo = Engine.undo(id);
        expect(restored.width).assertEqual(100);
        expect(restored.height).assertEqual(60);
      });
    });

    it('documentCropShrinksAndUndoRestores', 0, () => {
      withDocument(100, 80, 'crop', (id: number) => {
        const cropped: DocumentInfo = Engine.documentCrop(id, 10, 20, 60, 70);
        expect(cropped.width).assertEqual(50);
        expect(cropped.height).assertEqual(50);
        expect(cropped.canUndo).assertTrue();

        const restored: DocumentInfo = Engine.undo(id);
        expect(restored.width).assertEqual(100);
        expect(restored.height).assertEqual(80);
      });
    });

    it('documentCropOutsideCanvasThrows', 0, () => {
      withDocument(100, 80, 'crop-bad', (id: number) => {
        const before: number = Engine.revision(id);
        const message = messageOf(() => {
          Engine.documentCrop(id, 200, 200, 300, 300);
        });
        expect(message.indexOf('crop rectangle is outside the document') >= 0).assertTrue();
        expect(Engine.revision(id)).assertEqual(before);
      });
    });

    it('documentCropSameRectKeepsRevision', 0, () => {
      withDocument(100, 80, 'crop-same', (id: number) => {
        const before: number = Engine.revision(id);
        const same: DocumentInfo = Engine.documentCrop(id, 0, 0, 100, 80);
        expect(same.width).assertEqual(100);
        expect(Engine.revision(id)).assertEqual(before);
        expect(same.canUndo).assertFalse();
      });
    });

    it('documentResizeCanvasCenterAnchor', 0, () => {
      withDocument(100, 80, 'resize', (id: number) => {
        const resized: DocumentInfo = Engine.documentResizeCanvas(id, 120, 100, 4);
        expect(resized.width).assertEqual(120);
        expect(resized.height).assertEqual(100);
      });
    });

    it('documentResizeCanvasRejectsInvalidAnchor', 0, () => {
      withDocument(100, 80, 'resize-bad', (id: number) => {
        const message = messageOf(() => {
          Engine.documentResizeCanvas(id, 120, 100, 9);
        });
        expect(message.indexOf('invalid anchor') >= 0).assertTrue();
        expect(Engine.info(id).width).assertEqual(100);
      });
    });

    it('documentResizeCanvasRejectsZeroSize', 0, () => {
      withDocument(100, 80, 'resize-zero', (id: number) => {
        const message = messageOf(() => {
          Engine.documentResizeCanvas(id, 0, 100, 4);
        });
        expect(message.indexOf('invalid canvas size') >= 0).assertTrue();
      });
    });
  });

  describe('LayerTest', () => {
    it('createLayerAddsPixelLayerOnTop', 0, () => {
      withDocument(60, 40, 'layer-create', (id: number) => {
        const outcome = Engine.createLayer(id, 0, 'Top');
        expect(outcome.layerId > 0).assertTrue();
        expect(outcome.info.layers.length).assertEqual(2);
        expect(outcome.info.layers[0].name).assertEqual('Top');
        expect(outcome.info.layers[0].kind).assertEqual('Pixel');
        expect(outcome.info.layers[0].visible).assertTrue();
      });
    });

    it('createLayerBlankNameAutoFills', 0, () => {
      withDocument(60, 40, 'layer-auto', (id: number) => {
        const outcome = Engine.createLayer(id, 0, '');
        expect(outcome.info.layers[0].name.length > 0).assertTrue();
        expect(outcome.info.layers.length).assertEqual(2);
      });
    });

    it('createLayerUnknownAboveThrows', 0, () => {
      withDocument(60, 40, 'layer-bad', (id: number) => {
        const before: number = Engine.revision(id);
        const message = messageOf(() => {
          Engine.createLayer(id, 999999, 'Nope');
        });
        expect(message.indexOf('layer not found: 999999') >= 0).assertTrue();
        expect(Engine.info(id).layers.length).assertEqual(1);
        expect(Engine.revision(id)).assertEqual(before);
      });
    });

    it('renameLayerRoundTrip', 0, () => {
      withDocument(60, 40, 'rename', (id: number) => {
        const backgroundId: number = Engine.info(id).layers[0].id;
        Engine.renameLayer(id, backgroundId, 'Background 2');
        const layer: LayerInfo | null = findLayer(Engine.info(id), backgroundId);
        expect(layer !== null).assertTrue();
        expect((layer as LayerInfo).name).assertEqual('Background 2');
      });
    });

    it('renameLayerEmptyThrows', 0, () => {
      withDocument(60, 40, 'rename-empty', (id: number) => {
        const backgroundId: number = Engine.info(id).layers[0].id;
        const message = messageOf(() => {
          Engine.renameLayer(id, backgroundId, '   ');
        });
        expect(message.indexOf('layer name is empty') >= 0).assertTrue();
      });
    });

    it('layerVisibleOpacityBlendRoundTrip', 0, () => {
      withDocument(60, 40, 'props', (id: number) => {
        const backgroundId: number = Engine.info(id).layers[0].id;
        Engine.setLayerVisible(id, backgroundId, false);
        Engine.setLayerOpacity(id, backgroundId, 0.5);
        Engine.setLayerBlend(id, backgroundId, 'Multiply');
        const layer = findLayer(Engine.info(id), backgroundId) as LayerInfo;
        expect(layer.visible).assertFalse();
        expect(layer.opacity).assertEqual(0.5);
        expect(layer.blend).assertEqual('Multiply');
      });
    });

    it('layerFillOpacityRoundTrip', 0, () => {
      withDocument(60, 40, 'fill-opacity', (id: number) => {
        const backgroundId: number = Engine.info(id).layers[0].id;
        Engine.setLayerFillOpacity(id, backgroundId, 0.25);
        const layer = findLayer(Engine.info(id), backgroundId) as LayerInfo;
        expect(layer.fillOpacity).assertEqual(0.25);
      });
    });

    it('moveLayerClampsAtBothEnds', 0, () => {
      withDocument(60, 40, 'move', (id: number) => {
        const ids: number[] = threeLayerIds(id);
        const aId: number = ids[1];
        const up: DocumentInfo = Engine.moveLayer(id, aId, 5);
        expect(up.layers[0].id).assertEqual(aId);
        const down: DocumentInfo = Engine.moveLayer(id, aId, -5);
        expect(down.layers[down.layers.length - 1].id).assertEqual(aId);
      });
    });

    it('duplicateLayerAddsCopy', 0, () => {
      withDocument(60, 40, 'dup', (id: number) => {
        const backgroundId: number = Engine.info(id).layers[0].id;
        const duplicated = Engine.duplicateLayer(id, backgroundId);
        expect(duplicated.info.layers.length).assertEqual(2);
        const copy = findLayer(duplicated.info, duplicated.layerId) as LayerInfo;
        expect(copy.name.indexOf('copy') >= 0).assertTrue();
      });
    });

    it('deleteLayerUndoRestores', 0, () => {
      withDocument(60, 40, 'delete', (id: number) => {
        const a: number = Engine.createLayer(id, 0, 'A').layerId;
        Engine.deleteLayer(id, a);
        expect(Engine.info(id).layers.length).assertEqual(1);
        const restored: DocumentInfo = Engine.undo(id);
        expect(restored.layers.length).assertEqual(2);
        expect(findLayer(restored, a) !== null).assertTrue();
      });
    });

    it('groupAndUngroupRoundTrip', 0, () => {
      withDocument(60, 40, 'group', (id: number) => {
        const a: number = Engine.createLayer(id, 0, 'A').layerId;
        const b: number = Engine.createLayer(id, 0, 'B').layerId;
        const grouped = Engine.layerGroup(id, [a, b], 'G');
        expect(grouped.group.isGroup).assertTrue();
        expect(grouped.group.children.length).assertEqual(2);
        expect(Engine.info(id).layers.length).assertEqual(2);

        const count: number = Engine.layerUngroup(id, grouped.groupId);
        expect(count).assertEqual(2);
        expect(Engine.info(id).layers.length).assertEqual(3);
      });
    });

    it('groupEmptySelectionThrows', 0, () => {
      withDocument(60, 40, 'group-empty', (id: number) => {
        const message = messageOf(() => {
          Engine.layerGroup(id, [], 'G');
        });
        expect(message.indexOf('group needs at least one layer') >= 0).assertTrue();
        expect(Engine.info(id).layers.length).assertEqual(1);
      });
    });

    it('groupOwnerAndChildTogetherThrows', 0, () => {
      withDocument(60, 40, 'group-cycle', (id: number) => {
        const a: number = Engine.createLayer(id, 0, 'A').layerId;
        const b: number = Engine.createLayer(id, 0, 'B').layerId;
        const grouped = Engine.layerGroup(id, [a, b], 'G');
        const message = messageOf(() => {
          Engine.layerGroup(id, [grouped.groupId, a], 'Bad');
        });
        expect(message.indexOf('cannot group a group into itself') >= 0).assertTrue();
      });
    });

    it('layerExpandedKeepsRevision', 0, () => {
      withDocument(60, 40, 'expanded', (id: number) => {
        const a: number = Engine.createLayer(id, 0, 'A').layerId;
        const b: number = Engine.createLayer(id, 0, 'B').layerId;
        const grouped = Engine.layerGroup(id, [a, b], 'G');
        const before: number = Engine.revision(id);
        const expanded: boolean = Engine.layerExpanded(id, grouped.groupId, false);
        expect(expanded).assertFalse();
        expect(Engine.revision(id)).assertEqual(before);
        const layer = findLayer(Engine.info(id), grouped.groupId) as LayerInfo;
        expect(layer.expanded).assertFalse();
      });
    });

    it('clippedWithoutBaseThrows', 0, () => {
      withDocument(60, 40, 'clip-bad', (id: number) => {
        const backgroundId: number = Engine.info(id).layers[0].id;
        const message = messageOf(() => {
          Engine.layerClipped(id, backgroundId, true);
        });
        expect(message.indexOf('layer has no layer below to clip to') >= 0).assertTrue();
      });
    });

    it('clippedOnUpperLayerSucceeds', 0, () => {
      withDocument(60, 40, 'clip', (id: number) => {
        const a: number = Engine.createLayer(id, 0, 'A').layerId;
        const clipped: boolean = Engine.layerClipped(id, a, true);
        expect(clipped).assertTrue();
        const layer = findLayer(Engine.info(id), a) as LayerInfo;
        expect(layer.clipped).assertTrue();
        // 同值调用是空操作，不占撤销栈。
        const before: number = Engine.revision(id);
        expect(Engine.layerClipped(id, a, true)).assertTrue();
        expect(Engine.revision(id)).assertEqual(before);
      });
    });

    it('deleteManyIsSingleCheckpoint', 0, () => {
      withDocument(60, 40, 'delete-many', (id: number) => {
        const a: number = Engine.createLayer(id, 0, 'A').layerId;
        const b: number = Engine.createLayer(id, 0, 'B').layerId;
        const before: number = Engine.revision(id);
        const deleted: number = Engine.layerDeleteMany(id, [a, b]);
        expect(deleted).assertEqual(2);
        expect(Engine.info(id).layers.length).assertEqual(1);
        expect(Engine.revision(id)).assertEqual(before + 1);
        const restored: DocumentInfo = Engine.undo(id);
        expect(restored.layers.length).assertEqual(3);
      });
    });

    it('duplicateManyReturnsNewIds', 0, () => {
      withDocument(60, 40, 'dup-many', (id: number) => {
        const a: number = Engine.createLayer(id, 0, 'A').layerId;
        const outcome = Engine.duplicateLayerMany(id, JSON.stringify([a]));
        expect(outcome.layerIds.length).assertEqual(1);
        expect(outcome.info.layers.length).assertEqual(3);
        expect(outcome.layerIds[0] !== a).assertTrue();
        expect(findLayer(outcome.info, outcome.layerIds[0]) !== null).assertTrue();
      });
    });

    it('mergeDownReducesLayers', 0, () => {
      withDocument(60, 40, 'merge', (id: number) => {
        const a: number = Engine.createLayer(id, 0, 'A').layerId;
        const merged = Engine.mergeLayer(id, a);
        expect(merged.info.layers.length).assertEqual(1);
        expect(Engine.info(id).layers.length).assertEqual(1);
        const restored: DocumentInfo = Engine.undo(id);
        expect(restored.layers.length).assertEqual(2);
      });
    });
  });

  describe('HistoryTest', () => {
    it('undoEmptyStackThrows', 0, () => {
      withDocument(40, 30, 'undo-empty', (id: number) => {
        const message = messageOf(() => {
          Engine.undo(id);
        });
        expect(message.indexOf('nothing to undo') >= 0).assertTrue();
      });
    });

    it('redoEmptyStackThrows', 0, () => {
      withDocument(40, 30, 'redo-empty', (id: number) => {
        const message = messageOf(() => {
          Engine.redo(id);
        });
        expect(message.indexOf('nothing to redo') >= 0).assertTrue();
      });
    });

    it('historyLabelsRecordCreateLayer', 0, () => {
      withDocument(40, 30, 'labels', (id: number) => {
        Engine.createLayer(id, 0, 'A');
        const labels = Engine.history(id);
        expect(labels.undo.length).assertEqual(1);
        expect(labels.undo[0]).assertEqual('新建图层');
        expect(labels.redo.length).assertEqual(0);
      });
    });

    it('undoStepsJumpsMultipleWithOneRevision', 0, () => {
      withDocument(40, 30, 'undo-steps', (id: number) => {
        Engine.createLayer(id, 0, 'A');
        Engine.createLayer(id, 0, 'B');
        Engine.createLayer(id, 0, 'C');
        expect(Engine.info(id).layers.length).assertEqual(4);

        const before: number = Engine.revision(id);
        const jump = Engine.undoSteps(id, 2);
        expect(jump.steps).assertEqual(2);
        expect(jump.info.layers.length).assertEqual(2);
        expect(Engine.revision(id)).assertEqual(before + 1);
        expect(Engine.history(id).redo.length).assertEqual(2);
      });
    });

    it('redoStepsRestoresInOneRevision', 0, () => {
      withDocument(40, 30, 'redo-steps', (id: number) => {
        Engine.createLayer(id, 0, 'A');
        Engine.createLayer(id, 0, 'B');
        Engine.undoSteps(id, 2);
        const before: number = Engine.revision(id);
        const jump = Engine.redoSteps(id, 2);
        expect(jump.steps).assertEqual(2);
        expect(jump.info.layers.length).assertEqual(3);
        expect(Engine.revision(id)).assertEqual(before + 1);
      });
    });
  });
}