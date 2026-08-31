import { describe, expect, it } from 'vitest';

import { hasEnglishTranslation, translate } from './i18n';

describe('interface translations', () => {
  it('keeps Chinese copy unchanged and translates core English copy', () => {
    expect(translate('zh-CN', '设置')).toBe('设置');
    expect(translate('en-US', '设置')).toBe('Settings');
    expect(translate('en-US', '进入本地演示')).toBe('Open local demo');
  });

  it('translates dynamic accessible labels without changing user names', () => {
    expect(translate('en-US', 'Alice 正在输入…')).toBe('Alice is typing…');
    expect(translate('en-US', '查看图片 ferris.png')).toBe('View image ferris.png');
    expect(translate('en-US', 'Alice 来电')).toBe('Incoming call from Alice');
  });

  it('tracks every core label used by the language switch', () => {
    ['界面语言', '简体中文', '外观', '主要导航', '消息记录'].forEach((label) => {
      expect(hasEnglishTranslation(label), label).toBe(true);
    });
  });
});
