/*
 * 大图特效性能基线（1.3.4）：2400×1500 真实尺寸像素上逐个应用重滤镜，
 * 用 console.info('PERF ...') 打点，由 hdc hilog 取回。hypium 单测上限 5s，
 * 故按组拆成多个用例；只断言不报错，不设硬性耗时阈值（设备差异过大）。
 */
import { describe, it, expect } from '@ohos/hypium';
import { Engine } from '../../../main/ets/engine/Engine';
import { findFilter, buildFilterArgs } from '../../../main/ets/engine/FilterCatalog';
import { findGalleryPreset, buildGalleryArgs } from '../../../main/ets/engine/GalleryCatalog';

const W: number = 2400;
const H: number = 1500;

async function applyTop(id: string, layer: number, doc: number, overrides: Map<string, number>): Promise<void> {
  const args: string = buildFilterArgs(findFilter(id), overrides);
  const started: number = Date.now();
  const info = await Engine.filterApply(doc, layer, args);
  console.info('PERF top=' + id + ' ms=' + (Date.now() - started) + ' size=' + W + 'x' + H + ' layers=' + info.layers.length);
}

async function applyGallery(id: string, layer: number, doc: number): Promise<void> {
  const args: string = buildGalleryArgs(findGalleryPreset(id), new Map<string, number>());
  const started: number = Date.now();
  const info = await Engine.filterApply(doc, layer, args);
  console.info('PERF gallery=' + id + ' ms=' + (Date.now() - started) + ' size=' + W + 'x' + H + ' layers=' + info.layers.length);
}

async function seed(layer: number, doc: number): Promise<void> {
  // 云彩 + 杂色：给纯白底造出纹理，避免重滤镜走退化快路径。
  await applyTop('clouds', layer, doc, new Map<string, number>());
  await applyTop('addNoise', layer, doc, new Map<string, number>());
}

interface PerfDoc {
  id: number;
  layer: number;
}

function openPerfDoc(): PerfDoc {
  const created = Engine.newDocument(W, H, 'perf-large');
  const layer: number = Engine.info(created.id).layers[0].id;
  const result: PerfDoc = { id: created.id, layer: layer };
  return result;
}

function openPerfDocBig(): PerfDoc {
  const created = Engine.newDocument(4000, 3000, 'perf-4k');
  const layer: number = Engine.info(created.id).layers[0].id;
  const result: PerfDoc = { id: created.id, layer: layer };
  return result;
}

async function applyTopBig(id: string, layer: number, doc: number, overrides: Map<string, number>): Promise<void> {
  const args: string = buildFilterArgs(findFilter(id), overrides);
  const started: number = Date.now();
  const info = await Engine.filterApply(doc, layer, args);
  console.info('PERF4K top=' + id + ' ms=' + (Date.now() - started) + ' size=4000x3000 layers=' + info.layers.length);
}

async function applyGalleryBig(id: string, layer: number, doc: number): Promise<void> {
  const args: string = buildGalleryArgs(findGalleryPreset(id), new Map<string, number>());
  const started: number = Date.now();
  const info = await Engine.filterApply(doc, layer, args);
  console.info('PERF4K gallery=' + id + ' ms=' + (Date.now() - started) + ' size=4000x3000 layers=' + info.layers.length);
}

export default function perfTest() {
  describe('PerfTest', () => {
    it('blurPerf', 0, async () => {
      const d: PerfDoc = openPerfDoc();
      try {
        await seed(d.layer, d.id);
        await applyTop('gaussianBlur', d.layer, d.id, new Map<string, number>([['radius', 40]]));
        await applyTop('unsharpMask', d.layer, d.id, new Map<string, number>([['radius', 20], ['amount', 200]]));
        await applyTop('smartBlur', d.layer, d.id, new Map<string, number>([['radius', 50]]));
        await applyTop('lensBlur', d.layer, d.id, new Map<string, number>([['radius', 60]]));
        await applyTop('shapeBlur', d.layer, d.id, new Map<string, number>([['radius', 60]]));
        await applyTop('tiltShift', d.layer, d.id, new Map<string, number>([['blur', 60]]));
        expect(true).assertTrue();
      } finally {
        Engine.close(d.id);
      }
    });

    it('enhancePerf', 0, async () => {
      const d: PerfDoc = openPerfDoc();
      try {
        await seed(d.layer, d.id);
        await applyTop('reduceNoise', d.layer, d.id, new Map<string, number>());
        await applyTop('relight', d.layer, d.id, new Map<string, number>());
        await applyTop('lensFlare', d.layer, d.id, new Map<string, number>());
        await applyTop('colorHalftone', d.layer, d.id, new Map<string, number>());
        expect(true).assertTrue();
      } finally {
        Engine.close(d.id);
      }
    });

    it('oilAndCutoutPerf', 0, async () => {
      const d: PerfDoc = openPerfDoc();
      try {
        await seed(d.layer, d.id);
        await applyTop('oilPaint', d.layer, d.id, new Map<string, number>([['stylization', 8], ['scale', 8]]));
        await applyGallery('cutout', d.layer, d.id);
        expect(true).assertTrue();
      } finally {
        Engine.close(d.id);
      }
    });

    it('galleryTexturePerf', 0, async () => {
      const d: PerfDoc = openPerfDoc();
      try {
        await seed(d.layer, d.id);
        await applyGallery('texturizer', d.layer, d.id);
        await applyGallery('grain', d.layer, d.id);
        await applyGallery('stainedGlass', d.layer, d.id);
        await applyGallery('roughPastels', d.layer, d.id);
        await applyGallery('mosaicTiles', d.layer, d.id);
        expect(true).assertTrue();
      } finally {
        Engine.close(d.id);
      }
    });

    it('tiltShift4KPerf', 0, async () => {
      const d: PerfDoc = openPerfDocBig();
      try {
        await applyTopBig('addNoise', d.layer, d.id, new Map<string, number>());
        await applyTopBig('tiltShift', d.layer, d.id, new Map<string, number>([['blur', 60]]));
        expect(true).assertTrue();
      } finally {
        Engine.close(d.id);
      }
    });

    it('cutout4KPerf', 0, async () => {
      const d: PerfDoc = openPerfDocBig();
      try {
        await applyTopBig('addNoise', d.layer, d.id, new Map<string, number>());
        await applyGalleryBig('cutout', d.layer, d.id);
        expect(true).assertTrue();
      } finally {
        Engine.close(d.id);
      }
    });
  });
}
