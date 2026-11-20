import { AbilityConstant, ConfigurationConstant, UIAbility, Want } from '@kit.AbilityKit';
import { deviceInfo } from '@kit.BasicServicesKit';
import { hilog } from '@kit.PerformanceAnalysisKit';
import { window } from '@kit.ArkUI';

const DOMAIN = 0x0A02;
const TAG = 'ArtsPhoto';
/** insightIntent（小艺）意图名 → quickAction 映射；键名取自 resources/base/profile/insight_intent.json */
const INSIGHT_INTENT_TARGETS: Record<string, string> = {
  'CreateNewDocument': 'newDocument',
  'OpenSampleImage': 'openSample',
  'OpenRecentDocument': 'openRecent',
  'ContinueLastEdit': 'continueLast',
};

export default class EntryAbility extends UIAbility {
  onCreate(want: Want, launchParam: AbilityConstant.LaunchParam): void {
    this.handleIntent(want);
  }

  onNewWant(want: Want, launchParam: AbilityConstant.LaunchParam): void {
    this.handleIntent(want);
  }

  /** 卡片快捷动作 + insightIntent（小艺）意图框架统一入口：target/意图名 → quickAction 存入 AppStorage，Index 页分发 */
  private handleIntent(want: Want): void {
    const params = want.parameters;
    if (params !== undefined && params !== null) {
      const target = params['target'];
      if (typeof target === 'string' && target.length > 0) {
        AppStorage.setOrCreate<string>('quickAction', target);
      }
      const intentName = params['ohos.insightIntent.name'];
      if (typeof intentName === 'string' && intentName.length > 0) {
        const mapped = INSIGHT_INTENT_TARGETS[intentName];
        if (mapped !== undefined && mapped.length > 0) {
          AppStorage.setOrCreate<string>('quickAction', mapped);
        }
      }
    }
  }

  onDestroy(): void {
  }

  onWindowStageCreate(windowStage: window.WindowStage): void {
    // Main window is created, set main page for this ability
    windowStage.loadContent('pages/Index', (err) => {
      if (err.code) {
        hilog.error(DOMAIN, TAG, 'Failed to load the content. Cause: %{public}s', JSON.stringify(err));
        return;
      }
    });

    try {
      this.context.getApplicationContext().setColorMode(ConfigurationConstant.ColorMode.COLOR_MODE_NOT_SET);
    } catch (err) {
      hilog.error(DOMAIN, TAG, 'Failed to set colorMode. Cause: %{public}s', JSON.stringify(err));
    }

    this.maximizeIf2in1(windowStage);
  }

  // 2in1 设备上默认窗口化会露出桌面与任务栏，应用市场截图会因“窗口留白”被判不合规。
  // 仅 2in1 最大化，手机/平板布局不受影响；失败静默降级为原窗口。
  private maximizeIf2in1(windowStage: window.WindowStage): void {
    if (deviceInfo.deviceType !== '2in1') {
      return;
    }
    try {
      windowStage.getMainWindowSync().maximize().catch((err: Error) => {
        hilog.warn(DOMAIN, TAG, 'maximize failed: %{public}s', JSON.stringify(err));
      });
    } catch (err) {
      hilog.warn(DOMAIN, TAG, 'maximize skipped: %{public}s', JSON.stringify(err));
    }
  }

  onWindowStageDestroy(): void {
    // Main window is destroyed, release UI related resources
  }

  onForeground(): void {
    // Ability has brought to foreground
  }

  onBackground(): void {
    // Ability has back to background
  }
}