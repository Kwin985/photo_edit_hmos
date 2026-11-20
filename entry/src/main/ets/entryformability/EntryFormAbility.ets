/**
 * 服务卡片 FormExtensionAbility：2x2 快捷卡（ArtsPhoto 打开示例 / 新建文档）
 * 卡片为静态入口，无主应用状态推送；onAddForm 返回空绑定数据，按钮动作走 postCardAction router。
 */
import { FormExtensionAbility, formBindingData } from '@kit.FormKit';
import { Want } from '@kit.AbilityKit';

export default class EntryFormAbility extends FormExtensionAbility {
  onAddForm(want: Want): formBindingData.FormBindingData {
    return formBindingData.createFormBindingData({ 'brand': 'ArtsPhoto' });
  }

  onRemoveForm(formId: string): void {
  }

  onUpdateForm(formId: string): void {
  }

  onFormEvent(formId: string, message: string): void {
  }
}