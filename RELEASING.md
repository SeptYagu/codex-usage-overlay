# Publishing a Tauri release / 发布 Tauri 版本

GitHub Actions builds release assets from the tagged source. Use locally built packages for verification; published packages come from the workflow.

GitHub Actions 从标签对应的源码生成发布附件。本机构建仅用于验证，正式附件由发布流程生成。

1. Update the app version in `package.json`, `package-lock.json`, `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock`, and `src-tauri/tauri.conf.json`.
2. Run `npm test`, `npm run build`, and `cargo test --manifest-path src-tauri/Cargo.toml --locked`. Complete Windows smoke checks and record any remaining limitations in preview release notes.
3. Commit and push the source branch. Tauri previews may be tagged on `tauri-rewrite`; stable releases should use the approved stable branch.
4. Tag that exact commit with the matching app version. Preview tags use `vX.Y.Z-preview`:

```powershell
git tag -a v0.3.2-preview -m "v0.3.2-preview"
git push origin v0.3.2-preview
```

5. The **Release overlay** workflow installs dependencies, runs frontend/Rust tests, builds NSIS and MSI installers, and packages the standalone executable into a ZIP. It verifies ZIP contents against the source executable and documentation and generates SHA256 checksums for all three packages.
6. A tag containing a prerelease suffix creates a GitHub prerelease and does not replace the latest stable release. If a draft release already exists for the tag, the workflow uploads assets to that draft; publish it after the workflow succeeds and the four expected assets are verified.

Expected assets for `v0.3.2-preview`:

- `codex-usage-overlay_0.3.2_x64-setup.exe`
- `codex-usage-overlay_0.3.2_x64_en-US.msi`
- `CodexUsageOverlay-v0.3.2-preview-Windows-x64.zip`
- `SHA256SUMS.txt`

Local packaging check:

```powershell
npm run tauri -- build --ci --bundles nsis,msi
.\scripts\Build-TauriRelease.ps1 -Tag v0.3.2-preview
```

The portable ZIP contains `CodexUsageOverlay.exe` and the Chinese/English README files. It does not include a PowerShell launcher. Historical PowerShell releases remain available under their original tags.
