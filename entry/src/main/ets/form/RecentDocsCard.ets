/**
 * 2x2 服务卡片：ArtsPhoto 最近文档
 * 打开文档后，主界面将最近文档路径保存到 preferences（recentFiles）。
 * 卡片点击后进入主界面，主界面 Panel 区域展示最近文档列表。
 * 近场文件流转：通过 HarmonyOS 系统分享（system share）跨设备传输。
 *
 * 视觉（C-CARD-UI@1 §B）：与 QuickOpenCard 同族的「星云光圈」语言，
 * 主色调换成青蓝极光（区别入口卡的紫），玻璃面板内呈现最近文档状态。
 */
let recentDocsCardStorage: LocalStorage = new LocalStorage();

/** 主底渐变：深海蓝 → 靛 → 亮蓝（135° 对角）。 */
const RD_CARD_BG: [string, number][] = [['#08122e', 0], ['#0f3a8c', 0.58], ['#2f7bff', 1]];
/** 粉色辉光球渐变。 */
const RD_ORB_PINK: [string, number][] = [['rgba(244,114,182,0.40)', 0], ['rgba(244,114,182,0.0)', 1]];
/** 青色辉光球渐变。 */
const RD_ORB_CYAN: [string, number][] = [['rgba(34,211,238,0.45)', 0], ['rgba(34,211,238,0.0)', 1]];
/** CTA 胶囊 A：青 → 蓝。 */
const RD_PILL_A: [string, number][] = [['#22d3ee', 0], ['#3b82f6', 1]];
/** CTA 胶囊 B：粉 → 紫。 */
const RD_PILL_B: [string, number][] = [['#f472b6', 0], ['#8b5cf6', 1]];

@Entry(recentDocsCardStorage)
@Component
struct RecentDocsCard {
  /** 渐变镜头标志：外圈光晕 + 玻璃镜芯 + 高光点。 */
  @Builder
  LensMark(size: number) {
    Stack() {
      Column()
        .width(size)
        .height(size)
        .borderRadius('50%')
        .linearGradient({ angle: 135, colors: [['#22d3ee', 0], ['#3b82f6', 1]] })
        .border({ width: 1.5, color: 'rgba(255,255,255,0.65)' })
      Column()
        .width(size * 0.56)
        .height(size * 0.56)
        .borderRadius('50%')
        .backgroundColor('rgba(4,14,36,0.9)')
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
      // 辉光装饰球：粉（右上）+ 青（左下）。
      Column()
        .width(120)
        .height(120)
        .borderRadius('50%')
        .linearGradient({ angle: 135, colors: RD_ORB_PINK })
        .position({ x: 96, y: -34 })
      Column()
        .width(110)
        .height(110)
        .borderRadius('50%')
        .linearGradient({ angle: 135, colors: RD_ORB_CYAN })
        .position({ x: -28, y: 80 })

      Column({ space: 8 }) {
        // 品牌行：镜头标志 + 标题 + 尾部小辉点。
        Row({ space: 8 }) {
          this.LensMark(30)
          Column({ space: 1 }) {
            Text('ArtsPhoto')
              .fontSize(15)
              .fontWeight(FontWeight.Bold)
              .fontColor(Color.White)
            Text('最近文档')
              .fontSize(8)
              .fontColor('rgba(186,225,255,0.85)')
          }
          .alignItems(HorizontalAlign.Start)

          Text('')
            .layoutWeight(1)
          Column()
            .width(8)
            .height(8)
            .borderRadius('50%')
            .linearGradient({ angle: 90, colors: [['#f472b6', 0], ['#22d3ee', 1]] })
        }
        .width('100%')

        // 渐变细线。
        Row() {
          Column()
            .height(2)
            .layoutWeight(1)
            .borderRadius(2)
            .linearGradient({ angle: 90, colors: [['rgba(244,114,182,0.0)', 0], ['#f472b6', 0.5], ['rgba(34,211,238,0.0)', 1]] })
        }
        .width('100%')

        // 玻璃拟态「最近文档」面板。
        Column({ space: 6 }) {
          if (this.hasRecentDocs) {
            Text('最近编辑')
              .fontSize(9)
              .fontColor('rgba(210,235,255,0.9)')
            Row({ space: 6 }) {
              Column()
                .width(6)
                .height(6)
                .borderRadius('50%')
                .backgroundColor('#22d3ee')
              Text('继续上次创作 →')
                .fontSize(10)
                .fontColor(Color.White)
                .layoutWeight(1)
                .maxLines(1)
                .textOverflow({ overflow: TextOverflow.Ellipsis })
                .onClick(() => {
                  postCardAction(this, {
                    action: 'router',
                    abilityName: 'EntryAbility',
                    params: { target: 'openRecent' }
                  });
                })
            }
            .width('100%')
          } else {
            Text('暂无最近文档')
              .fontSize(10)
              .fontWeight(FontWeight.Medium)
              .fontColor(Color.White)
            Text('打开一张图片开始创作吧')
              .fontSize(8)
              .fontColor('rgba(210,235,255,0.7)')
          }
        }
        .width('100%')
        .padding({ top: 9, bottom: 9, left: 10, right: 10 })
        .backgroundColor('rgba(255,255,255,0.10)')
        .borderRadius(12)
        .border({ width: 1, color: 'rgba(255,255,255,0.22)' })
        .alignItems(HorizontalAlign.Start)

        // 底部操作行。
        Row({ space: 8 }) {
          this.Pill('打开', RD_PILL_A, 'openSample')
          this.Pill('新建', RD_PILL_B, 'newDocument')
        }
        .justifyContent(FlexAlign.Center)
        .width('100%')
      }
      .width('100%')
      .height('100%')
      .padding(12)
      .justifyContent(FlexAlign.Center)
    }
    .width('100%')
    .height('100%')
    .linearGradient({ angle: 135, colors: RD_CARD_BG })
    .borderRadius(16)
    .clip(true)
  }

  get hasRecentDocs(): boolean {
    // 卡片端不持有远程状态；主界面会通过 AppStorage quickAction 通知。
    return false;
  }
}
