/**
 * insightIntent 意图执行入口（srcEntry）
 * 意图声明见 resources/base/profile/insight_intent.json（CreateNewDocument / OpenSampleImage）
 * 触发方式：小艺以 UIABILITY foreground 模式拉起 EntryAbility 并携带意图名，
 * EntryAbility.handleIntent 解析 'ohos.insightIntent.name' 映射为 quickAction 存入 AppStorage，
 * Index 页 aboutToAppear / onPageShow 读取 quickAction 分发到对应功能；
 * 本文件作为意图代码入口参与编译（loader 收集），onExecute 保持轻量占位。
 */

/** 意图执行结果（意图框架约定：code 0 表示成功） */
export interface IntentExecResult {
  code: number;
  result: Record<string, Object>;
}

export class IntentExecResultImpl implements IntentExecResult {
  code: number = 0;
  result: Record<string, Object> = {};
}

export class ArtsPhotoIntentExecutor {
  onExecute(intentName: string, params: Record<string, Object>): Promise<IntentExecResult> {
    return Promise.resolve(new IntentExecResultImpl());
  }
}

export default new ArtsPhotoIntentExecutor();