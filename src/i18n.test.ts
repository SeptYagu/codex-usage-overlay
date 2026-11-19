import { describe, expect, it } from 'vitest';
import i18n from './i18n';

const keys = (node: object, prefix = ''): string[] =>
  Object.entries(node).flatMap(([key, value]) =>
    value && typeof value === 'object' ? keys(value, `${prefix}${key}.`) : [`${prefix}${key}`]);

const bundle = (locale: string) => keys(i18n.getResourceBundle(locale, 'translation')).sort();

describe('locale key parity', () => {
  it.each(['zh-CN', 'zh-Hant', 'ru-RU'])('%s defines exactly the en-US keys', (locale) => {
    expect(bundle(locale)).toEqual(bundle('en-US'));
  });
});
