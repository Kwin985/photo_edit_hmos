/**
 * 1x2 服务卡片：ArtsPhoto 快速新建
 * 点击按钮走 postCardAction router 拉起 EntryAbility 并携带 target='newDocument'，
 * EntryAbility.handleIntent 映射为 quickAction 存入 AppStorage，Index 页分发为新建文档。
 *
 * 视觉（C-CARD-UI@1 §C）：1x2 宽扁形态，横向布局——左侧渐变镜头标志，
 * 中间品牌文案，右侧渐变胶囊 CTA；品红极光底与另两张卡同族。
 */
let quickCreateCardStorage: LocalStorage = new LocalStorage();

/** 主底渐变：深紫黑 → 品红紫 → 亮紫（135° 对角）。 */
const QC_CARD_BG: [string, number][] = [['#1a0b2e', 0], ['#6d1fa8', 0.6], ['#c026d3', 1]];
/** 青色辉光球渐变。 */
const QC_ORB_CYAN: [string, number][] = [['rgba(34,211,238,0.40)', 0], ['rgba(34,211,238,0.0)', 1]];
/** CTA 胶囊：青 → 靛。 */
const QC_PILL_CTA: [string, number][] = [['#22d3ee', 0], ['#6366f1', 1]];

@Entry(quickCreateCardStorage)
@Component
struct QuickCreateCard {
  /** 渐变镜头标志：外圈光晕 + 玻璃镜芯 + 高光点。 */
  @Builder
  LensMark(size: number) {
    Stack() {
      Column()
        .width(size)
        .height(size)
        .borderRadius('50%')
        .linearGradient({ angle: 135, colors: [['#e879f9', 0], ['#8b5cf6', 1]] })
        .border({ width: 1.5, color: 'rgba(255,255,255,0.65)' })
      Column()
        .width(size * 0.56)
        .height(size * 0.56)
        .borderRadius('50%')
        .backgroundColor('rgba(10,6,30,0.9)')
      Column()
        .width(size * 0.16)
        .height(size * 0.16)
        .borderRadius('50%')
        .backgroundColor('rgba(255,255,255,0.95)')
    }
    .width(size)
    .height(size)
  }

  build() {
    Stack() {
      // 辉光装饰球：青（右上）+ 品红（左下）。
      Column()
        .width(110)
        .height(110)
        .borderRadius('50%')
        .linearGradient({ angle: 135, colors: QC_ORB_CYAN })
        .position({ x: 210, y: -40 })
      Column()
        .width(100)
        .height(100)
        .borderRadius('50%')
        .linearGradient({ angle: 135, colors: [['rgba(232,121,249,0.35)', 0], ['rgba(232,121,249,0.0)', 1]] })
        .position({ x: -26, y: 64 })

      Row({ space: 10 }) {
        this.LensMark(34)
        Column({ space: 2 }) {
          Text('ArtsPhoto')
            .fontSize(14)
            .fontWeight(FontWeight.Bold)
            .fontColor(Color.White)
          Text('快速新建空白文档')
            .fontSize(8)
            .fontColor('rgba(232,214,255,0.85)')
            .maxLines(1)
        }
        .alignItems(HorizontalAlign.Start)
        .layoutWeight(1)

        Text('新建')
          .fontSize(12)
          .fontWeight(FontWeight.Bold)
          .fontColor(Color.White)
          .textAlign(TextAlign.Center)
          .padding({ left: 18, right: 18, top: 8, bottom: 8 })
          .borderRadius(999)
          .linearGradient({ angle: 90, colors: QC_PILL_CTA })
          .border({ width: 1, color: 'rgba(255,255,255,0.4)' })
          .onClick(() => {
            postCardAction(this, {
              action: 'router',
              abilityName: 'EntryAbility',
              params: { target: 'newDocument' }
            });
          })
      }
      .width('100%')
      .height('100%')
      .padding({ left: 14, right: 14 })
      .alignItems(VerticalAlign.Center)
    }
    .width('100%')
    .height('100%')
    .linearGradient({ angle: 135, colors: QC_CARD_BG })
    .borderRadius(16)
    .clip(true)
  }
}
