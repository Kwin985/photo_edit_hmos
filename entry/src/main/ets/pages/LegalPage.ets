import { router } from '@kit.ArkUI';
import { bundleManager } from '@kit.AbilityKit';

const DEVELOPER_NAME: string = '王伟';
const CONTACT_EMAIL: string = '344922772@qq.com';
const POLICY_DATE: string = '2026-11-12';

const PRIVACY_SECTIONS: string[] = [
  '更新日期：' + POLICY_DATE + '  生效日期：' + POLICY_DATE,
  'ArtsPhoto（以下简称“本应用”）是一款运行在 HarmonyOS 设备上的本地图像编辑工具。我们深知个人信息对您的重要性，并会依照《中华人民共和国个人信息保护法》等法律法规处理您的信息。本政策说明本应用如何处理与您有关的信息。',
  '一、适用范围\n本政策适用于 ArtsPhoto 应用本身。本应用不包含账号体系、广告、统计分析或任何在线服务，也不链接到第三方网站。',
  '二、我们如何收集和使用信息\n1. 本应用不收集、不上传任何个人信息，不采集设备标识符（IMEI、OAID、SN 等）、位置、通讯录、通话记录、短信、剪贴板或应用列表。\n2. 您通过系统文件选择器主动选择的图片文件，仅在您的设备本地内存中被读取、编辑与导出。文件内容不会发送到任何服务器，本应用也未申请网络权限。\n3. 本应用没有广告 SDK、统计 SDK、推送 SDK、崩溃上报 SDK 或其他第三方 SDK。',
  '三、本地存储的信息\n为记住您的使用偏好，本应用在设备本地保存以下设置：标尺是否显示、网格间距、参考线是否显示、打开图片时的默认缩放方式。上述数据仅保存在本机应用沙箱内，不会上传；卸载本应用或清除应用数据后即被删除。',
  '四、系统权限与文件访问\n本应用不申请任何系统敏感权限。打开与保存文件均通过 HarmonyOS 系统文件选择器完成，本应用只能访问您在该选择器中明确选择的文件，无法访问其他文件或相册。',
  '五、信息的共享、转让与公开披露\n由于本应用不收集个人信息，因此不存在向第三方共享、转让或公开披露个人信息的情形，也不存在跨境传输。',
  '六、信息的存储地点与期限\n偏好设置存储在您的设备本地，保存期限为您使用本应用期间；卸载应用或清除应用数据后即删除。您选择的图片文件由您自行保管，本应用不保留副本。',
  '七、信息安全\n本应用在本地处理图片与设置，不经过网络传输。请妥善保管您的设备与文件；因设备丢失、系统被破解或您主动分享文件造成的信息泄露，不属于本应用可控范围。',
  '八、未成年人保护\n本应用为通用图像编辑工具，不面向未成年人单独收集信息。未成年人应在监护人指导下使用本应用。若监护人发现未成年人向我们提供了信息，可通过本政策末尾的方式联系我们。',
  '九、您的权利\n1. 您可以在“关于 ArtsPhoto”页面查看本政策与用户协议。\n2. 首次启动时的隐私声明同意由 HarmonyOS 系统隐私托管服务统一记录与管理；如需重新阅读本政策，您可以在“关于 ArtsPhoto”页面随时查看。\n3. 您可以随时卸载本应用或清除应用数据，以删除本地保存的全部设置。\n4. 由于本应用不收集个人信息，不存在需要查阅、复制、更正或转移的个人信息。',
  '十、政策更新\n若本应用的功能或信息处理方式发生变化，我们会更新本政策，并在应用内以显著方式提示您重新阅读与同意。未经您同意，我们不会削减您依据本政策享有的权利。',
  '十一、联系我们\n开发者：' + DEVELOPER_NAME + '\n联系邮箱：' + CONTACT_EMAIL + '\n我们将在收到您的问题后 15 个工作日内予以答复。'
];

const TERMS_SECTIONS: string[] = [
  '更新日期：' + POLICY_DATE + '  生效日期：' + POLICY_DATE,
  '欢迎使用 ArtsPhoto。本协议是您与开发者之间就使用本应用达成的约定。请您在使用前仔细阅读；您在首次启动的隐私声明弹窗中选择同意或实际使用本应用，即表示您已阅读并同意本协议。',
  '一、许可范围\n开发者授予您一项个人的、不可转让、不可再许可、非排他的许可，用于在您拥有或控制的 HarmonyOS 设备上安装和使用本应用。',
  '二、使用规范\n1. 您不得利用本应用制作、复制、发布、传播违反法律法规或公序良俗的内容。\n2. 您不得对本应用进行反向工程、反编译、破解、篡改，或删除应用内的权利声明。\n3. 您不得将本应用用于侵犯他人著作权、肖像权、名誉权或其他合法权益的行为。\n4. 您应自行对编辑、保存与分享的图片内容承担责任。',
  '三、您的内容\n您通过本应用打开、编辑与导出的图片及成果归您或相应权利人所有。本应用仅在您的设备本地处理这些内容，不主张任何权利，也不上传或留存副本。',
  '四、知识产权\n本应用的软件、界面、图标、文字及其他由开发者提供的素材，其知识产权归开发者或相应权利人所有。本应用使用的开源组件遵循其各自许可证，相关声明可在“关于 ArtsPhoto”页面查看。',
  '五、免责声明\n1. 本应用按“现状”提供。我们会尽力保持功能稳定，但不保证应用完全不中断、无错误或满足您的特定需求。\n2. 因设备故障、系统升级、误操作、文件损坏或不可抗力导致的数据丢失或损坏，开发者不承担责任。建议您在编辑重要文件前自行备份。\n3. 法律法规对本条另有强制性规定的，从其规定。',
  '六、服务变更与终止\n我们可能因功能升级、合规要求或业务调整而更新、暂停或终止本应用的全部或部分功能。终止前，我们会在合理范围内予以提示。',
  '七、法律适用与争议解决\n本协议适用中华人民共和国法律。因本协议产生的争议，双方应友好协商解决；协商不成的，可向开发者所在地有管辖权的人民法院提起诉讼。',
  '八、联系方式\n开发者：' + DEVELOPER_NAME + '\n联系邮箱：' + CONTACT_EMAIL
];

@Entry
@Component
struct LegalPage {
  @State tab: string = 'privacy';
  @State version: string = '1.0.0';

  aboutToAppear(): void {
    this.loadVersion();
  }

  private loadVersion(): void {
    try {
      const info = bundleManager.getBundleInfoForSelfSync(bundleManager.BundleFlag.GET_BUNDLE_INFO_DEFAULT);
      if (info.versionName.length > 0) {
        this.version = info.versionName;
      }
    } catch (err) {
      this.version = '1.0.0';
    }
  }

  private back(): void {
    router.back();
  }


  @Builder
  TabButton(label: string, value: string) {
    Text(label)
      .fontSize(13)
      .fontColor(this.tab === value ? '#ffffff' : '#8b95a4')
      .backgroundColor(this.tab === value ? '#2b6cf6' : '#1b1f27')
      .height(32)
      .padding({ left: 14, right: 14 })
      .borderRadius(16)
      .onClick(() => {
        this.tab = value;
      })
  }

  @Builder
  SectionList(sections: string[]) {
    Column({ space: 14 }) {
      ForEach(sections, (text: string) => {
        Text(text)
          .fontSize(13)
          .lineHeight(21)
          .fontColor('#c7d0dc')
          .width('100%')
      }, (text: string, index: number) => index.toString() + text.length.toString())
    }
    .width('100%')
    .alignItems(HorizontalAlign.Start)
    .padding({ left: 18, right: 18, top: 16, bottom: 28 })
  }

  @Builder
  AboutPane() {
    Column({ space: 16 }) {
      Column({ space: 6 }) {
        Text('ArtsPhoto')
          .fontSize(22)
          .fontWeight(FontWeight.Bold)
          .fontColor('#ffffff')
        Text('版本 ' + this.version)
          .fontSize(13)
          .fontColor('#8b95a4')
        Text('HarmonyOS 本地图像编辑器')
          .fontSize(13)
          .fontColor('#8b95a4')
      }
      .width('100%')
      .alignItems(HorizontalAlign.Start)

      Column({ space: 8 }) {
        Text('数据说明')
          .fontSize(14)
          .fontWeight(FontWeight.Medium)
          .fontColor('#ffffff')
        Text('本应用不收集、不上传任何个人信息。您打开的图片仅在设备本地处理，且不申请网络权限。')
          .fontSize(13)
          .lineHeight(21)
          .fontColor('#c7d0dc')
      }
      .width('100%')
      .alignItems(HorizontalAlign.Start)
      .padding(14)
      .backgroundColor('#18232f')
      .borderRadius(12)

      Column({ space: 8 }) {
        Text('权限概览')
          .fontSize(14)
          .fontWeight(FontWeight.Medium)
          .fontColor('#ffffff')
        Text('网络权限：无')
          .fontSize(13)
          .fontColor('#c7d0dc')
        Text('数据：仅本地内存')
          .fontSize(13)
          .fontColor('#c7d0dc')
        Text('权限申请：0 项')
          .fontSize(13)
          .fontColor('#c7d0dc')
      }
      .width('100%')
      .alignItems(HorizontalAlign.Start)
      .padding(14)
      .backgroundColor('#18232f')
      .borderRadius(12)

      Row({ space: 10 }) {
        Text('查看隐私政策')
          .fontSize(13)
          .fontColor('#9fc4ff')
          .onClick(() => {
            this.tab = 'privacy';
          })
        Text('·').fontSize(13).fontColor('#5d6673')
        Text('用户协议')
          .fontSize(13)
          .fontColor('#9fc4ff')
          .onClick(() => {
            this.tab = 'terms';
          })
      }
      .width('100%')

      Column({ space: 6 }) {
        Text('开发者：' + DEVELOPER_NAME)
          .fontSize(13)
          .fontColor('#c7d0dc')
        Text('联系邮箱：' + CONTACT_EMAIL)
          .fontSize(13)
          .fontColor('#c7d0dc')
        Text('本应用使用 Rust 与 HarmonyOS 开源组件，相关组件遵循其各自许可证。')
          .fontSize(12)
          .lineHeight(19)
          .fontColor('#8b95a4')
      }
      .width('100%')
      .alignItems(HorizontalAlign.Start)
    }
    .width('100%')
    .alignItems(HorizontalAlign.Start)
    .padding({ left: 18, right: 18, top: 16, bottom: 28 })
  }

  build() {
    Column() {
      Row() {
        Text('返回')
          .fontSize(14)
          .fontColor('#9fc4ff')
          .padding({ left: 4, right: 12, top: 6, bottom: 6 })
          .onClick(() => {
            this.back();
          })
        Text(this.tab === 'privacy' ? '隐私政策' : (this.tab === 'terms' ? '用户协议' : '关于 ArtsPhoto'))
          .fontSize(16)
          .fontWeight(FontWeight.Medium)
          .fontColor('#ffffff')
          .layoutWeight(1)
      }
      .width('100%')
      .height(52)
      .padding({ left: 14, right: 16 })
      .backgroundColor('#14161b')
      .border({ width: { bottom: 1 }, color: '#232830' })

      Row({ space: 8 }) {
        this.TabButton('隐私政策', 'privacy')
        this.TabButton('用户协议', 'terms')
        this.TabButton('关于', 'about')
      }
      .width('100%')
      .padding({ left: 14, right: 14, top: 10, bottom: 10 })
      .backgroundColor('#0f1115')

      Scroll() {
        Column() {
          if (this.tab === 'privacy') {
            this.SectionList(PRIVACY_SECTIONS)
          } else if (this.tab === 'terms') {
            this.SectionList(TERMS_SECTIONS)
          } else {
            this.AboutPane()
          }
        }
        .width('100%')
      }
      .width('100%')
      .layoutWeight(1)
      .scrollBar(BarState.Auto)
      .align(Alignment.TopStart)
    }
    .width('100%')
    .height('100%')
    .backgroundColor('#0a0b0d')
  }
}