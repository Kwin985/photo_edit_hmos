/**
 * 服务卡片 FormExtensionAbility：3 张卡片统一入口。
 * onAddForm 返回含最近文档的绑定数据；onUpdateForm 从 preferences 读取 recentFiles 推送更新，
 * 使 RecentDocsCard 可感知价值（卡片刷新链路）。
 */
import { FormExtensionAbility, formBindingData, formProvider } from '@kit.FormKit';
import { Want } from '@kit.AbilityKit';
import { preferences } from '@kit.ArkData';

const PREFS_NAME: string = 'artsphoto_prefs';
const PREF_KEY_RECENT_FILES: string = 'recentFiles';

/** 提取文件名（不含扩展名），用于卡片展示。 */
function getShortName(path: string): string {
  const idx = path.lastIndexOf('/');
  const name = idx >= 0 ? path.substring(idx + 1) : path;
  const extIdx = name.lastIndexOf('.');
  return extIdx > 0 ? name.substring(0, extIdx) : name;
}

export default class EntryFormAbility extends FormExtensionAbility {
  onAddForm(want: Want): formBindingData.FormBindingData {
    return this.buildFormData();
  }

  onRemoveForm(formId: string): void {
  }

  onUpdateForm(formId: string): void {
    try {
      formProvider.updateForm(formId, this.buildFormData()).catch(() => {});
    } catch (err) {
    }
  }

  onFormEvent(formId: string, message: string): void {
  }

  private buildFormData(): formBindingData.FormBindingData {
    try {
      const store = preferences.getPreferencesSync(this.context, { name: PREFS_NAME });
      const recent = store.getSync(PREF_KEY_RECENT_FILES, []) as string[];
      const hasRecent = recent.length > 0;
      return formBindingData.createFormBindingData({
        'brand': 'ArtsPhoto',
        'hasRecentDocs': hasRecent,
        'recentDocName': hasRecent ? getShortName(recent[0]) : ''
      });
    } catch (err) {
      return formBindingData.createFormBindingData({ 'brand': 'ArtsPhoto' });
    }
  }
}