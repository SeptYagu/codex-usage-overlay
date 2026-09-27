import i18n from 'i18next';
import { initReactI18next } from 'react-i18next';

const resources = {
  'en-US': {
    translation: {
      refreshNow: "Refresh Now",
      refreshing: "Refreshing usage",
      settings: "Settings...",
      hide: "Hide Overlay",
      exit: "Exit Overlay",
      windowSettings: "Overlay Settings",
      settingsLoading: "Loading settings...",
      settingsLoadFailed: "Unable to load settings. Please try again.",
      retry: "Retry",
      scale: "Overlay Size",
      overlayLayout: "Overlay layout",
      groupedLayout: "Grouped capsule",
      stacksLayout: "Metric stacks",
      fiveHourLabel: "5 HOUR",
      weeklyLabel: "WEEKLY",
      creditsLabel: "CREDITS",
      balanceLabel: "Balance",
      transparency: "Background Transparency",
      showCredits: "Show Credit Balance",
      autoStart: "Start automatically on boot",
      refreshInterval: "Refresh Interval",
      sec30: "30 seconds",
      sec60: "60 seconds (Default)",
      min2: "2 minutes",
      min5: "5 minutes",
      hint: "Hint: Sliders show instant preview. Transparency only affects the background plate.",
      feedback: "Feedback & Suggestions:",
      done: "Done",
      language: "Language",
      autoSystem: "Auto (System)",
      loadingTooltip: "Connecting to Codex app-server... Hold Left-Click to drag; Right-Click for menu.",
      tooltipUpdated: "Last update: {{time}}. Hold Left-Click to drag; Right-Click for menu.",
      defaultScale: "175% (Default)",
      defaultAlpha: "23% (Default)",
      opaque: "0% (Opaque)",
      transparent: "80% (Transparent)",
    }
  },
  'zh-CN': {
    translation: {
      refreshNow: "立即刷新用量",
      refreshing: "正在刷新用量",
      settings: "浮窗设置…",
      hide: "隐藏悬浮窗",
      exit: "退出悬浮窗",
      windowSettings: "浮窗设置",
      settingsLoading: "正在加载设置…",
      settingsLoadFailed: "无法加载设置，请重试。",
      retry: "重试",
      scale: "浮窗大小",
      overlayLayout: "浮窗布局",
      groupedLayout: "分组胶囊",
      stacksLayout: "指标堆叠",
      fiveHourLabel: "5 小时",
      weeklyLabel: "每周",
      creditsLabel: "CREDITS",
      balanceLabel: "余额",
      transparency: "背景透明度",
      showCredits: "显示 Credit 余额",
      autoStart: "开机时自动启动",
      refreshInterval: "刷新频率",
      sec30: "30 秒",
      sec60: "60 秒 (默认)",
      min2: "2 分钟",
      min5: "5 分钟",
      hint: "提示：滑块调节即时预览生效。透明度仅影响悬浮窗底板，文字和数值始终保持高对比度清晰显示。",
      feedback: "反馈与建议：",
      done: "完成",
      language: "语言 / Language",
      autoSystem: "自动 / Auto (System)",
      loadingTooltip: "正在连接 Codex app-server... 按住鼠标左键可拖动；右键打开菜单。",
      tooltipUpdated: "最近更新：{{time}}。按住鼠标左键可拖动；右键打开菜单。",
      defaultScale: "175% (默认)",
      defaultAlpha: "23% (默认)",
      opaque: "0% (完全不透明)",
      transparent: "80% (高透明)",
    }
  },
  'zh-Hant': {
    translation: {
      refreshNow: "立即重新整理用量",
      refreshing: "正在重新整理用量",
      settings: "浮窗設定…",
      hide: "隱藏懸浮窗",
      exit: "結束懸浮窗",
      windowSettings: "浮窗設定",
      settingsLoading: "正在載入設定…",
      settingsLoadFailed: "無法載入設定，請重試。",
      retry: "重試",
      scale: "浮窗大小",
      overlayLayout: "浮窗版面配置",
      groupedLayout: "分組膠囊",
      stacksLayout: "指標堆疊",
      fiveHourLabel: "5 小時",
      weeklyLabel: "每週",
      creditsLabel: "CREDITS",
      balanceLabel: "餘額",
      transparency: "背景透明度",
      showCredits: "顯示 Credit 餘額",
      autoStart: "開機時自動啟動",
      refreshInterval: "重新整理頻率",
      sec30: "30 秒",
      sec60: "60 秒 (預設)",
      min2: "2 分鐘",
      min5: "5 分鐘",
      hint: "提示：滑桿調節即時預覽生效。透明度僅影響懸浮窗底板，文字和數值始終保持高對比度清晰顯示。",
      feedback: "意見反映與建議：",
      done: "完成",
      language: "語言 / Language",
      autoSystem: "自動 / Auto (System)",
      loadingTooltip: "正在連線 Codex app-server... 按住滑鼠左鍵可拖曳；右鍵開啟功能表。",
      tooltipUpdated: "最近更新：{{time}}。按住滑鼠左鍵可拖曳；右鍵開啟功能表。",
      defaultScale: "175% (預設)",
      defaultAlpha: "23% (預設)",
      opaque: "0% (完全不透明)",
      transparent: "80% (高透明)",
    }
  }
};

i18n
  .use(initReactI18next)
  .init({
    resources,
    lng: 'en-US',
    fallbackLng: 'en-US',
    interpolation: {
      escapeValue: false
    }
  });

export const ZH_HANT_LOCALES = new Set([
  'zh-tw',
  'zh-hk',
  'zh-mo',
  'zh-hant',
  'zh-hant-tw',
  'zh-hant-hk',
  'zh-hant-mo',
]);

export const ZH_CN_LOCALES = new Set([
  'zh',
  'zh-cn',
  'zh-sg',
  'zh-hans',
  'zh-hans-cn',
  'zh-hans-sg',
]);

export const matchSupportedLocale = (systemLocale: string): 'en-US' | 'zh-CN' | 'zh-Hant' => {
  const normalized = (systemLocale || '').trim().toLowerCase().replace(/_/g, '-');
  if (ZH_HANT_LOCALES.has(normalized)) {
    return 'zh-Hant';
  }
  if (ZH_CN_LOCALES.has(normalized)) {
    return 'zh-CN';
  }
  return 'en-US';
};

export const resolveClientLocale = (lang: string): 'en-US' | 'zh-CN' | 'zh-Hant' => {
  if (lang === 'zh-CN' || lang === 'zh-Hant' || lang === 'en-US') {
    return lang;
  }
  const nav = typeof navigator !== 'undefined' ? navigator.language : '';
  return matchSupportedLocale(nav);
};

export const updateLanguage = (lang: string) => {
  i18n.changeLanguage(resolveClientLocale(lang));
};

export default i18n;
