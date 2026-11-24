/*
 * 蒙版 / 选区 / 渲染 / 文字形状 / 图层效果 / 调整层的无 UI 回归。
 *
 * 与 Document.test.ets 同口径：内存文档 + finally close，不依赖设备 UI。
 */
import { describe, it, expect } from '@ohos/hypium';
import { Engine, DocumentInfo, LayerInfo, LayerMaskInfo, SelectionInfo } from '../../../main/ets/engine/Engine';

function messageOf(action: () => void): string {
  try {
    action();
  } catch (error) {
    return error instanceof Error ? error.message : String(error);
  }
  return '';
}

function withDocument(width: number, height: number, name: string, body: (id: number) => void): void {
  const created = Engine.newDocument(width, height, name);
  try {
    body(created.id);
  } finally {
    Engine.close(created.id);
  }
}

async function withDocumentAsync(width: number, height: number, name: string, body: (id: number) => Promise<void>): Promise<void> {
  const created = Engine.newDocument(width, height, name);
  try {
    await body(created.id);
  } finally {
    Engine.close(created.id);
  }
}

async function messageOfAsync(action: () => Promise<void>): Promise<string> {
  try {
    await action();
  } catch (error) {
    return error instanceof Error ? error.message : String(error);
  }
  return '';
}

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

export default function advancedTest() {
  describe('MaskTest', () => {
    it('maskAddRevealThenRemove', 0, () => {
      withDocument(60, 40, 'mask', (id: number) => {
        const backgroundId: number = Engine.info(id).layers[0].id;
        const added: LayerMaskInfo = Engine.layerMaskAdd(id, backgroundId, 'reveal');
        expect(added.present).assertTrue();
        expect(added.enabled).assertTrue();
        expect(added.linked).assertTrue();
        expect(added.density).assertEqual(1);
        expect(added.feather).assertEqual(0);

        const removed: LayerMaskInfo = Engine.layerMaskRemove(id, backgroundId);
        expect(removed.present).assertFalse();
      });
    });

    it('maskAddInvalidFillThrows', 0, () => {
      withDocument(60, 40, 'mask-fill-bad', (id: number) => {
        const backgroundId: number = Engine.info(id).layers[0].id;
        const before: number = Engine.revision(id);
        const message = messageOf(() => {
          Engine.layerMaskAdd(id, backgroundId, 'gray');
        });
        expect(message.indexOf('fill must be reveal or hide') >= 0).assertTrue();
        expect(Engine.revision(id)).assertEqual(before);
      });
    });

    it('maskAddTwiceThrows', 0, () => {
      withDocument(60, 40, 'mask-twice', (id: number) => {
        const backgroundId: number = Engine.info(id).layers[0].id;
        Engine.layerMaskAdd(id, backgroundId, 'hide');
        const message = messageOf(() => {
          Engine.layerMaskAdd(id, backgroundId, 'reveal');
        });
        expect(message.indexOf('layer already has a mask') >= 0).assertTrue();
      });
    });

    it('maskSetDensityFeatherRoundTrip', 0, () => {
      withDocument(60, 40, 'mask-set', (id: number) => {
        const backgroundId: number = Engine.info(id).layers[0].id;
        Engine.layerMaskAdd(id, backgroundId, 'reveal');
        const updated: LayerMaskInfo = Engine.layerMaskSet(id, backgroundId, '{"enabled":false,"density":0.4,"feather":3.5}');
        expect(updated.enabled).assertFalse();
        // 引擎内部是 f32，回读 0.4 会有 float 误差。
        expect(Math.abs(updated.density - 0.4) < 0.001).assertTrue();
        expect(Math.abs(updated.feather - 3.5) < 0.001).assertTrue();

        const readBack: LayerMaskInfo = Engine.layerMaskGet(id, backgroundId);
        expect(readBack.enabled).assertFalse();
        expect(readBack.feather).assertEqual(3.5);
      });
    });

    it('maskSetSameValuesKeepsRevision', 0, () => {
      withDocument(60, 40, 'mask-idempotent', (id: number) => {
        const backgroundId: number = Engine.info(id).layers[0].id;
        Engine.layerMaskAdd(id, backgroundId, 'reveal');
        const before: number = Engine.revision(id);
        Engine.layerMaskSet(id, backgroundId, '{"enabled":true,"density":1,"feather":0}');
        expect(Engine.revision(id)).assertEqual(before);
      });
    });

    it('maskFillFromSelectionBuildsMask', 0, () => {
      withDocument(60, 40, 'mask-selection', (id: number) => {
        const backgroundId: number = Engine.info(id).layers[0].id;
        Engine.select(id, '{"kind":"all"}');
        const mask: LayerMaskInfo = Engine.layerMaskFillFromSelection(id, backgroundId, 'reveal');
        expect(mask.present).assertTrue();

        // 一次撤销应回到无蒙版状态。
        Engine.undo(id);
        expect(Engine.layerMaskGet(id, backgroundId).present).assertFalse();
      });
    });

    it('maskFillWithoutSelectionThrows', 0, () => {
      withDocument(60, 40, 'mask-no-selection', (id: number) => {
        const backgroundId: number = Engine.info(id).layers[0].id;
        const message = messageOf(() => {
          Engine.layerMaskFillFromSelection(id, backgroundId, 'reveal');
        });
        expect(message.indexOf('no selection') >= 0).assertTrue();
      });
    });
  });

  describe('SelectionTest', () => {
    it('selectRectReportsBounds', 0, () => {
      withDocument(100, 80, 'select-rect', (id: number) => {
        const selection: SelectionInfo = Engine.select(id, '{"kind":"rect","x0":10,"y0":10,"x1":60,"y1":50}');
        expect(selection.active).assertTrue();
        expect(selection.x0).assertEqual(10);
        expect(selection.y0).assertEqual(10);
        expect(selection.x1).assertEqual(60);
        expect(selection.y1).assertEqual(50);
        const readBack: SelectionInfo = Engine.selectionInfo(id);
        expect(readBack.active).assertTrue();
      });
    });

    it('selectNoneClearsSelection', 0, () => {
      withDocument(100, 80, 'select-none', (id: number) => {
        Engine.select(id, '{"kind":"rect","x0":10,"y0":10,"x1":60,"y1":50}');
        const cleared: SelectionInfo = Engine.select(id, '{"kind":"none"}');
        expect(cleared.active).assertFalse();
        expect(Engine.selectionInfo(id).active).assertFalse();
      });
    });

    it('selectUnsupportedKindThrows', 0, () => {
      withDocument(100, 80, 'select-bad', (id: number) => {
        const message = messageOf(() => {
          Engine.select(id, '{"kind":"star"}');
        });
        expect(message.indexOf('unsupported selection kind: star') >= 0).assertTrue();
      });
    });

    it('selectAllThenInvertClears', 0, () => {
      withDocument(100, 80, 'select-invert', (id: number) => {
        const all: SelectionInfo = Engine.select(id, '{"kind":"all"}');
        expect(all.active).assertTrue();
        const inverted: SelectionInfo = Engine.select(id, '{"kind":"invert"}');
        expect(inverted.active).assertFalse();
      });
    });
  });

  describe('RenderTest', () => {
    it('renderPixelsLengthMatchesRoi', 0, () => {
      withDocument(64, 48, 'render-roi', (id: number) => {
        const bytes = new Uint8Array(Engine.renderPixels(id, 0, 0, 32, 24));
        expect(bytes.length).assertEqual(32 * 24 * 4);
      });
    });

    it('renderPixelsRejectsZeroSize', 0, () => {
      withDocument(64, 48, 'render-zero', (id: number) => {
        const message = messageOf(() => {
          Engine.renderPixels(id, 0, 0, 0, 10);
        });
        expect(message.indexOf('invalid render rectangle') >= 0).assertTrue();
      });
    });

    it('paintStrokeLandsOpaquePixel', 0, () => {
      withDocument(100, 80, 'paint', (id: number) => {
        const backgroundId: number = Engine.info(id).layers[0].id;
        const brush = '{"size":20,"hardness":1,"opacity":1,"flow":1,"spacing":0.1,"pressureSize":false,"erase":false,"color":[0,0,1,1]}';
        // 单点采样在部分笔刷实现里不产生 stamp，用一条水平短线覆盖中心。
        const points = '[{"x":30,"y":40},{"x":70,"y":40}]';
        const paintedId: number = Engine.paintStroke(id, backgroundId, brush, points);
        expect(paintedId).assertEqual(backgroundId);

        const pixel = new Uint8Array(Engine.renderPixels(id, 50, 40, 1, 1));
        expect(pixel.length).assertEqual(4);
        // 不透明覆盖：白底被笔刷颜色替换。
        expect(pixel[3]).assertEqual(255);
        expect(pixel[0] + pixel[1] + pixel[2] > 0).assertTrue();
      });
    });

    it('brushAndShapeShareColorOrder', 0, () => {
      // 同一份 `[0,0,1,1]` 分别走画笔与形状：两者最终像素应一致。
      // 不一致说明画笔颜色数组的通道约定与形状不同（红蓝互换类缺陷）。
      let brushPixel: number[] = [0, 0, 0, 0];
      let shapePixel: number[] = [0, 0, 0, 0];

      withDocument(60, 40, 'paint-order', (id: number) => {
        const backgroundId: number = Engine.info(id).layers[0].id;
        const brush = '{"size":20,"hardness":1,"opacity":1,"flow":1,"spacing":0.1,"pressureSize":false,"erase":false,"color":[0,0,1,1]}';
        Engine.paintStroke(id, backgroundId, brush, '[{"x":10,"y":20},{"x":50,"y":20}]');
        const brushBytes = new Uint8Array(Engine.renderPixels(id, 30, 20, 1, 1));
        brushPixel = [brushBytes[0], brushBytes[1], brushBytes[2], brushBytes[3]];
      });

      withDocument(60, 40, 'shape-order', (id: number) => {
        Engine.shapeCreate(id, '{"kind":"rect","x":10,"y":10,"w":40,"h":20,"fill":[0,0,1,1]}');
        const shapeBytes = new Uint8Array(Engine.renderPixels(id, 30, 20, 1, 1));
        shapePixel = [shapeBytes[0], shapeBytes[1], shapeBytes[2], shapeBytes[3]];
      });

      expect(brushPixel.length).assertEqual(4);
      expect(shapePixel.length).assertEqual(4);
      expect(brushPixel[0]).assertEqual(shapePixel[0]);
      expect(brushPixel[1]).assertEqual(shapePixel[1]);
      expect(brushPixel[2]).assertEqual(shapePixel[2]);
      expect(brushPixel[3]).assertEqual(shapePixel[3]);
    });

    it('previewReturnsBytes', 0, () => {
      withDocument(100, 80, 'preview', (id: number) => {
        const preview: ArrayBuffer = Engine.preview(id, 32);
        expect(preview.byteLength > 0).assertTrue();
      });
    });

    it('exportPngReturnsBytes', 0, async () => {
      await withDocumentAsync(40, 30, 'export', async (id: number) => {
        const png: ArrayBuffer = await Engine.exportImage(id, 'png', 90);
        expect(png.byteLength > 0).assertTrue();
        // PNG 魔数：89 50 4E 47。
        const head = new Uint8Array(png);
        expect(head[0]).assertEqual(0x89);
        expect(head[1]).assertEqual(0x50);
        expect(head[2]).assertEqual(0x4e);
        expect(head[3]).assertEqual(0x47);
      });
    });

    it('exportUnsupportedFormatThrows', 0, async () => {
      await withDocumentAsync(40, 30, 'export-bad', async (id: number) => {
        const message = await messageOfAsync(async () => {
          await Engine.exportImage(id, 'bmp', 90);
        });
        expect(message.indexOf('unsupported export format: bmp') >= 0).assertTrue();
      });
    });

    it('revisionIncrementsPerCheckpoint', 0, () => {
      withDocument(40, 30, 'revision', (id: number) => {
        const before: number = Engine.revision(id);
        Engine.createLayer(id, 0, 'A');
        expect(Engine.revision(id)).assertEqual(before + 1);
        Engine.undo(id);
        expect(Engine.revision(id)).assertEqual(before + 2);
      });
    });
  });

  describe('TextShapeTest', () => {
    it('shapeCreateRectAddsShapeLayer', 0, () => {
      withDocument(100, 80, 'shape', (id: number) => {
        const outcome = Engine.shapeCreate(id, '{"kind":"rect","x":10,"y":10,"w":40,"h":30,"name":"Box"}');
        expect(outcome.info.layers.length).assertEqual(2);
        const layer = findLayer(outcome.info, outcome.layerId) as LayerInfo;
        expect(layer.kind).assertEqual('Shape');
        expect(layer.name).assertEqual('Box');
      });
    });

    it('shapeCreateUnsupportedKindThrows', 0, () => {
      withDocument(100, 80, 'shape-bad', (id: number) => {
        const message = messageOf(() => {
          Engine.shapeCreate(id, '{"kind":"star"}');
        });
        expect(message.indexOf('unsupported shape kind: star') >= 0).assertTrue();
        expect(Engine.info(id).layers.length).assertEqual(1);
      });
    });

    it('textCreateEmptyThrows', 0, () => {
      withDocument(100, 80, 'text-empty', (id: number) => {
        const message = messageOf(() => {
          Engine.textCreate(id, '{"text":"   "}');
        });
        expect(message.indexOf('text is empty') >= 0).assertTrue();
      });
    });

    it('textCreateAddsTypeLayer', 0, () => {
      withDocument(100, 80, 'text', (id: number) => {
        const outcome = Engine.textCreate(id, '{"text":"Hi","x":10,"y":20}');
        expect(outcome.info.layers.length).assertEqual(2);
        const layer = findLayer(outcome.info, outcome.layerId) as LayerInfo;
        expect(layer.kind).assertEqual('Type');
      });
    });

    it('layerEditGetReturnsText', 0, () => {
      withDocument(100, 80, 'text-get', (id: number) => {
        const outcome = Engine.textCreate(id, '{"text":"Hi","x":10,"y":20}');
        const edit = Engine.layerEditGet(id, outcome.layerId);
        expect(edit.kind).assertEqual('Type');
        expect(edit.text).assertEqual('Hi');
        expect(edit.x).assertEqual(10);
        expect(edit.y).assertEqual(20);
      });
    });

    it('layerEditApplyChangesText', 0, () => {
      withDocument(100, 80, 'text-edit', (id: number) => {
        const outcome = Engine.textCreate(id, '{"text":"Hi","x":10,"y":20}');
        const before: number = Engine.revision(id);
        const updated = Engine.layerEditApply(id, outcome.layerId, '{"text":"Hello"}');
        expect(updated.text).assertEqual('Hello');
        expect(Engine.revision(id)).assertEqual(before + 1);
        expect(Engine.layerEditGet(id, outcome.layerId).text).assertEqual('Hello');
      });
    });

    it('layerEditApplySameValueKeepsRevision', 0, () => {
      withDocument(100, 80, 'text-noop', (id: number) => {
        const outcome = Engine.textCreate(id, '{"text":"Hi","x":10,"y":20}');
        const before: number = Engine.revision(id);
        const updated = Engine.layerEditApply(id, outcome.layerId, '{"text":"Hi"}');
        expect(updated.text).assertEqual('Hi');
        expect(Engine.revision(id)).assertEqual(before);
      });
    });
  });

  describe('EffectsTest', () => {
    it('effectsStartEmptyAndEnabled', 0, () => {
      withDocument(60, 40, 'fx-empty', (id: number) => {
        const backgroundId: number = Engine.info(id).layers[0].id;
        const effects = Engine.layerEffects(id, backgroundId);
        expect(effects.enabled).assertTrue();
        expect(effects.items.length).assertEqual(0);
      });
    });

    it('effectsAddDropShadow', 0, () => {
      withDocument(60, 40, 'fx-add', (id: number) => {
        const backgroundId: number = Engine.info(id).layers[0].id;
        const effects = Engine.updateLayerEffect(id, backgroundId, '{"op":"add","kind":"dropShadow"}');
        expect(effects.items.length).assertEqual(1);
        expect(effects.items[0].kind).assertEqual('dropShadow');
        expect(effects.items[0].label).assertEqual('Drop Shadow');
        expect(effects.items[0].enabled).assertTrue();
        expect(effects.items[0].blend).assertEqual('Multiply');
      });
    });

    it('effectsUnsupportedOpThrows', 0, () => {
      withDocument(60, 40, 'fx-bad', (id: number) => {
        const backgroundId: number = Engine.info(id).layers[0].id;
        const message = messageOf(() => {
          Engine.updateLayerEffect(id, backgroundId, '{"op":"explode"}');
        });
        expect(message.indexOf('unsupported op') >= 0).assertTrue();
      });
    });

    it('effectsClearRemovesAll', 0, () => {
      withDocument(60, 40, 'fx-clear', (id: number) => {
        const backgroundId: number = Engine.info(id).layers[0].id;
        Engine.updateLayerEffect(id, backgroundId, '{"op":"add","kind":"dropShadow"}');
        Engine.updateLayerEffect(id, backgroundId, '{"op":"add","kind":"stroke"}');
        const cleared = Engine.updateLayerEffect(id, backgroundId, '{"op":"clear"}');
        expect(cleared.items.length).assertEqual(0);
      });
    });
  });

  describe('AdjustmentTest', () => {
    it('adjustmentAddBrightnessContrast', 0, () => {
      withDocument(60, 40, 'adj', (id: number) => {
        Engine.addAdjustment(id, 'brightnessContrast', '{"brightness":0.25}');
        const info: DocumentInfo = Engine.info(id);
        expect(info.layers.length).assertEqual(2);
        expect(info.layers[0].kind).assertEqual('Adjustment');
      });
    });

    it('adjustmentRoundTrip', 0, () => {
      withDocument(60, 40, 'adj-read', (id: number) => {
        Engine.addAdjustment(id, 'brightnessContrast', '{"brightness":0.25,"contrast":-0.5}');
        const layerId: number = Engine.info(id).layers[0].id;
        const adjustment = Engine.adjustment(id, layerId);
        expect(adjustment.type).assertEqual('brightnessContrast');
        expect(adjustment.brightness).assertEqual(0.25);
        expect(adjustment.contrast).assertEqual(-0.5);
      });
    });

    it('adjustmentUnsupportedKindThrows', 0, () => {
      withDocument(60, 40, 'adj-bad', (id: number) => {
        const message = messageOf(() => {
          Engine.addAdjustment(id, 'glow', '{}');
        });
        expect(message.indexOf('unsupported adjustment: glow') >= 0).assertTrue();
        expect(Engine.info(id).layers.length).assertEqual(1);
      });
    });
  });
}