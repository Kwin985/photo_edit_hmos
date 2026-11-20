/**
 * 2x2 服务卡片：ArtsPhoto 快速入口（打开示例图 / 新建文档）
 * 按钮走 postCardAction router 拉起 EntryAbility 并携带 target，
 * EntryAbility.handleIntent 映射为 quickAction 存入 AppStorage，Index 页 aboutToAppear/onPageShow 分发。
 *
 * 视觉（C-CARD-UI@1 §A「星云光圈」）：
 * - 深靛→紫极光渐变底，替代旧版黑灰底；
 * - 双辉光装饰球（青 / 品红，半透明渐变圆）叠在内容后面；
 *   卡片环境若忽略 position 属性，装饰球会回退为内容背后的柔和色晕，不破坏布局；
 * - 渐变镜头标志 + 玻璃拟态面板 + 渐变胶囊按钮（卡片不支持 blur/shadow，发光感靠半透明叠层）。
 */
let quickOpenCardStorage: LocalStorage = new LocalStorage();

/** 主底渐变：深靛 → 皇家紫 → 电紫（135° 对角）。 */
const QO_CARD_BG: [string, number][] = [['#101038', 0], ['#4316a3', 0.62], ['#7d2ae8', 1]];
/** 青色辉光球渐变。 */
const QO_ORB_CYAN: [string, number][] = [['rgba(34,211,238,0.55)', 0], ['rgba(34,211,238,0.0)', 1]];
/** 品红辉光球渐变。 */
const QO_ORB_MAGENTA: [string, number][] = [['rgba(232,121,249,0.45)', 0], ['rgba(232,121,249,0.0)', 1]];
/** CTA 胶囊 A：青 → 靛。 */
const QO_PILL_A: [string, number][] = [['#22d3ee', 0], ['#6366f1', 1]];
/** CTA 胶囊 B：粉 → 紫。 */
const QO_PILL_B: [string, number][] = [['#e879f9', 0], ['#8b5cf6', 1]];

@Entry(quickOpenCardStorage)
@Component
struct QuickOpenCard {
  /** 渐变镜头标志：外圈光晕 + 玻璃镜芯 + 高光点。 */
  @Builder
  LensMark(size: number) {
    Stack() {
      Column()
        .width(size)
        .height(size)
        .borderRadius('50%')
        .linearGradient({ angle: 135, colors: [['#22d3ee', 0], ['#8b5cf6', 1]] })
        .border({ width: 1.5, color: 'rgba(255,255,255,0.65)' })
      Column()
        .width(size * 0.56)
        .height(size * 0.56)
        .borderRadius('50%')
        .backgroundColor('rgba(6,10,40,0.9)')
      Column()
        .width(size * 0.16)
        .height(size * 0.16)
        .borderRadius('50%')
        .backgroundColor('rgba(255,255,255,0.95)')
    }
    .width(size)
    .height(size)
  }

  /** 渐变胶囊按钮。 */
  @Builder
  Pill(label: string, colors: [string, number][], target: string) {
    Text(label)
      .fontSize(11)
      .fontWeight(FontWeight.Medium)
      .fontColor(Color.White)
      .textAlign(TextAlign.Center)
      .padding({ left: 12, right: 12, top: 6, bottom: 6 })
      .borderRadius(999)
      .linearGradient({ angle: 90, colors: colors })
      .border({ width: 1, color: 'rgba(255,255,255,0.35)' })
      .onClick(() => {
        postCardAction(this, {
          action: 'router',
          abilityName: 'EntryAbility',
          params: { target: target }
        });
      })
  }

  build() {
    Stack() {
      // 辉光装饰球：青（右上，溢出裁切）+ 品红（左下）。
      Column()
        .width(132)
        .height(132)
        .borderRadius('50%')
        .linearGradient({ angle: 135, colors: QO_ORB_CYAN })
        .position({ x: 92, y: -38 })
      Column()
        .width(118)
        .height(118)
        .borderRadius('50%')
        .linearGradient({ angle: 135, colors: QO_ORB_MAGENTA })
        .position({ x: -30, y: 74 })

      Column({ space: 8 }) {
        // 品牌行：镜头标志 + 标题 + 尾部小辉点。
        Row({ space: 8 }) {
          this.LensMark(30)
          Column({ space: 1 }) {
            Text('ArtsPhoto')
              .fontSize(16)
              .fontWeight(FontWeight.Bold)
              .fontColor(Color.White)
            Text('本地图像编辑器')
              .fontSize(8)
              .fontColor('rgba(178,214,255,0.85)')
          }
          .alignItems(HorizontalAlign.Start)

          Text('')
            .layoutWeight(1)
          Column()
            .width(8)
            .height(8)
            .borderRadius('50%')
            .linearGradient({ angle: 90, colors: [['#22d3ee', 0], ['#e879f9', 1]] })
        }
        .width('100%')

        // 渐变细线：青 → 紫 淡出。
        Row() {
          Column()
            .height(2)
            .layoutWeight(1)
            .borderRadius(2)
            .linearGradient({ angle: 90, colors: [['rgba(34,211,238,0.0)', 0], ['#22d3ee', 0.5], ['rgba(139,92,246,0.0)', 1]] })
        }
        .width('100%')

        // 玻璃拟态操作面板。
        Column({ space: 8 }) {
          Row({ space: 8 }) {
            this.Pill('示例图', QO_PILL_A, 'openSample')
            this.Pill('新建文档', QO_PILL_B, 'newDocument')
          }
          .justifyContent(FlexAlign.Center)
          .width('100%')
        }
        .width('100%')
        .padding({ top: 10, bottom: 10 })
        .backgroundColor('rgba(255,255,255,0.10)')
        .borderRadius(12)
        .border({ width: 1, color: 'rgba(255,255,255,0.22)' })

        Text('零权限 · 纯本地处理 · 不收集信息')
          .fontSize(8)
          .fontColor('rgba(255,255,255,0.55)')
      }
      .width('100%')
      .height('100%')
      .padding(12)
      .justifyContent(FlexAlign.Center)
    }
    .width('100%')
    .height('100%')
    .linearGradient({ angle: 135, colors: QO_CARD_BG })
    .borderRadius(16)
    .clip(true)
  }
}
