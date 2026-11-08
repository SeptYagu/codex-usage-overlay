# 多来源 5H 预测与轮播外观：实施审查

- 审查基线：`f2212ff`（2026-11-08）
- 结论：暂不通过发布门槛；先修复下述 P2 设置回退问题，再完成版本构建与发布验证。

## 已核对的实现

- Codex、Claude、DSH 各自保存 `legacy`／`weighted`／`personalized` 选择。旧版 Codex 顶层选择迁移到 Codex 列；切换算法只重投影缓存，不重新读取或训练。Weekly 与保护逻辑保持原路径。
- DSH 的预测历史按实例和账户隔离，传输地址变化后仍使用同一预测命名空间；旧地址路径中的状态可迁入。Claude 只在本地元数据有有效账户 UUID 且采样期间身份未变时开放进阶候选。身份缺失时回退经典，并切断原账户历史。
- 轮播开关不再增加可见来源文字或候选最大尺寸占位。来源真实切换仍使用现有胶囊测量和贴边回写。

## 发现与修复顺序

| 级别 | 位置 | 发现 | 修复与验收 |
| --- | --- | --- | --- |
| P2 | `src/providerSettings.ts` | 旧设置对象尚未带来源级算法字段时，Claude／DSH 的前端投影会继承 Codex 顶层算法。即使后端迁移通常会补全字段，前端回退本身仍会跨来源泄漏。 | 非 Codex 缺字段时固定回退 `legacy`，Codex 才继承旧顶层字段；加迁移前对象和三来源独立选择测试。 |

## 验证记录与发布边界

- `npm run build` 通过；`npm test` 14 个文件、187 项通过。
- Edge 浏览器布局脚本覆盖 296 个轮播外观组合，全部通过。比较时将会自然跳动的倒计时文本标准化，仍核对当前来源的结构、宽高及裁切。
- 无空格验证副本的 GNU `cargo check --locked` 与 `cargo check --tests --locked` 通过。GNU `cargo test` 在生成 Tauri `cdylib` 时因链接器 `export ordinal too large` 失败；仅 `rlib` 测试目标可编译，但运行时出现 `STATUS_ENTRYPOINT_NOT_FOUND`。这不是测试断言结果，需由正式 Windows CI 的 MSVC 环境完成完整 Rust 测试。
- 本机没有可用 Claude Code 账户与 DSH Host 实例，所以账户身份的实际供应商路径、真实轮播切页和安装包实机行为仍需发布前后分别核验。缺身份时进阶预测按设计停用。

修复后重新运行前端测试、构建、浏览器外观脚本和 Rust 代码检查；正式发布以 GitHub Actions 全套测试、Windows 打包与附件核验为准。
