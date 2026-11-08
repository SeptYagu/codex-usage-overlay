# 额度轮播保持浮窗原样的改动方案

- 日期：2026-11-08（America/Indianapolis）
- 状态：方案；本次不修改应用代码。
- 源码基线：`main` 的 `ec11114`。本方案修订 [11 月 6 日轮播方案](2026-11-06-quota-carousel-plan.md)中关于常驻来源文字和候选最大尺寸预留的外观规则。

## 目标与现状

开启额度轮播只改变显示来源和切换时机，不额外添加文字框，也不因开关本身改变当前来源的胶囊内容、间距或尺寸。来源实际切换时，浮窗继续按该来源现有内容自然调整尺寸，与手动切换使用同一套逻辑。

当前额外内容集中在 `src/components/OverlayView.tsx`：`CapsuleReadings` 在 `quotaCarouselEnabled` 为真时渲染 `.overlay-source-label`，附带来源名称及缓存、旧读数或无读数说明。轮播还在不可见的 `.overlay-measurements` 中渲染所有候选，并通过 `reservedSize` 给可见胶囊设置 `minWidth`、`minHeight`。所以即使还没切页，开启轮播也可能使胶囊多出文字和空白尺寸。对应样式在 `src/index.css`。

现有 `fitCapsuleSize`、`ResizeObserver`、`overlay_size_invalidated` 和 `dock_window_resized` 已负责测量真实胶囊、调整原生窗口及贴边位置。`App.tsx` 已将 effective 来源的设置和读数交给 `OverlayView`。本次复用这些路径，不增加另一套尺寸计算或来源切换逻辑。

## 最小改动

| 位置 | 改动 |
| --- | --- |
| `src/components/OverlayView.tsx` 的 `CapsuleReadings` | 删除轮播专属的 `.overlay-source-label` 节点及只服务于它的 `providerLabel`、`healthLabel`、`health`、`status` 参数；额度、余额、现有 logo 和加载点维持原来的渲染条件。 |
| `OverlayView` 尺寸部分 | 删除 `measureRef`、`reservedSize`、候选来源隐藏渲染、轮播专属 `minWidth`／`minHeight` 和关联的测量 effect；保留现有真实胶囊测量、DPI 补偿及 dock 回写。`fitCapsuleSize` 的依赖项移除 `reservedSize`。 |
| `OverlayView` 与 `App.tsx` 接口 | `display` 属性只用于上述文字和隐藏测量，删除后移除该属性及传参；`monitors` 仍用于现有 logo 的手动“下一来源”按钮。 |
| `src/index.css`、`src/i18n.ts` | 删除不再使用的 `.overlay-source-label`、`.overlay-measurements` 样式和 `carouselCached`、`carouselStale`、`carouselUnavailable` 三语言文案。设置页和托盘菜单仍使用的 `carouselStatus_*` 与 `monitorHealth` 保留。 |
| `README.md`、`README.en.md` | 代码落地时更新“隐藏 logo 仍显示来源文字”的当前功能说明；`v1.7.0` 发布说明及 11 月 6 日实施记录作为历史记录保留。 |

来源识别复用现有入口：打开 `showProviderLogo` 时仍显示当前来源图标；logo 关闭时，浮窗不显示常驻来源标识。将当前 `overlay-capsule` 已有的更新时间 `title` 补成“来源名称 · 更新时间／加载状态”，让悬停时能查看来源，并继续在设置页和托盘菜单查看来源与轮播状态。这只改现有提示内容，不添加可见节点或强制打开 logo。无常驻标识时，未悬停的用户无法仅凭胶囊读数区分来源，这一取舍应写入用户说明。

## 行为边界

1. 同一来源、同一设置及同一读数下，轮播开关从关到开或从开到关，可见胶囊 DOM 与实际宽高应相同；不能出现来源文字或额外留白。
2. Codex、Claude、DSH 的内容宽度本来可能不同，例如 Claude 没有余额栏。自动切页允许像现有手动切换一样按真实内容改变原生窗口尺寸；贴边时继续使用现有 `dock_window_resized` 定位。此方案不承诺三个来源始终等宽。
3. 轮播的候选、定时、手动停留、交互暂停、缓存身份校验及保护逻辑不变。贴边收起状态继续暂停轮播；原有收起条不增加标识。
4. 若实机切到较宽来源时出现短暂裁切，先检查现有 `fitCapsuleSize`／`overlay_size_invalidated` 的触发和 dock 回写；修复应沿用这条测量路径，不恢复轮播专属的最大尺寸占位。

## 实施后的验收

- 调整 `src/components/OverlayView.test.tsx`：原“隐藏 logo 时显示轮播来源文字”断言改为无新增文字节点、无 logo 按钮；保留 hover／focus 暂停断言。对同一来源分别渲染轮播关闭和开启，核对可见内容结构一致。
- 调整 `scripts/display-controls-qa/carousel.cjs`：删除必须存在 `.overlay-source-label` 及跨来源尺寸恒定的旧断言；改为同一来源开／关轮播时胶囊尺寸一致、内容不裁切。覆盖三个来源、两种布局、logo 开／关、三语言、明暗主题和现有 DPI 档位。
- Windows 实机检查开启开关前后没有突然扩窗或新增文字；自动切换到有／无余额来源时，窗口贴合内容且贴边锚点正确；隐藏 logo 时既无文字框也不强制显示图标。此项需要在实施后执行，当前方案阶段不计为已通过。

完成标准：开启轮播不改变当前浮窗外观；自动换页只呈现目标来源原有布局，且不发生裁切。用户说明与实际行为一致。
