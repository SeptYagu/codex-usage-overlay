# Publishing a Tauri release / 发布 Tauri 版本

GitHub Actions builds release assets from the tagged source. Use locally built packages for verification; published packages come from the workflow. Preview versions are numbered, `X.Y.Z-preview.N`, tagged `vX.Y.Z-preview.N`, and do not replace the latest stable release.

Generate a Tauri updater key once with `npm run tauri -- signer generate --write-keys <private-key-path>`. Put the public key in `src-tauri/tauri.conf.json` and store the private key as the GitHub Actions secret `TAURI_SIGNING_PRIVATE_KEY`. Keep the private key outside the repository. If the key is password-protected, also set `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`.

GitHub Actions 从标签对应的源码生成发布附件。本机构建仅用于验证，正式附件由发布流程生成。

Each release has its own app version, because `latest.json` carries it and the updater installs only a higher one: `1.5.0-preview.1` < `1.5.0-preview.2` < `1.5.0`. A preview that reused the stable version would never be offered to the previous preview, nor the stable release to it.

1. Update the app version in `package.json`, `package-lock.json` (the root version and the root package entry), `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock`, `src-tauri/tauri.conf.json`, and `src/version.ts`. The last one is the frontend constant behind the settings-window badge and the tray menu, so a release that skips it ships an installer that shows the previous version. `src/version.test.ts` fails when the locations drift apart. The two frontend tests that assert the rendered version (`SettingsView.test.tsx`, `TrayMenuView.test.tsx`) read that constant instead of a hardcoded string, so they need no manual update — keep it that way, a hardcoded version there is a seventh place that silently drifts. Windows Installer accepts only numeric versions, so a preview also sets `bundle.windows.wix.version` in `tauri.conf.json` to `X.Y.Z.N` (`1.6.0-preview.1` → `1.6.0.1`); a stable release removes it or sets `X.Y.Z`. The version test checks this too.
2. Run `npm test`, `npm run build`, and `cargo test --manifest-path src-tauri/Cargo.toml --locked`.
3. Complete Windows smoke checks, record any remaining limitations in preview release notes, then commit and push the source branch (`main`).
4. Tag that exact commit with `v` plus the app version. Packaging fails when they differ:

```powershell
git tag -a v1.6.0-preview.1 -m "v1.6.0-preview.1"
git push origin v1.6.0-preview.1
```

5. The **Release overlay** workflow installs dependencies, runs frontend/Rust tests, builds NSIS and MSI installers, and packages the standalone executable into a ZIP. It verifies ZIP contents against the source executable and documentation and generates SHA256 checksums for all three packages.
6. A tag containing a prerelease suffix creates a GitHub prerelease and does not replace the latest stable release. If a draft release already exists for the tag, the workflow uploads assets to that draft; publish it after the workflow succeeds and the expected installers, signatures, portable ZIP, checksums, and `latest.json` are verified.

For a new release, the workflow uses `docs/releases/<tag>.md` as its release body when that file exists; otherwise it generates GitHub release notes. Keep preview limitations and migration changes in that file before pushing the tag.

Expected assets for `v1.6.0-preview.1`:

- `codex-usage-overlay_1.6.0-preview.1_x64-setup.exe` and its `.sig`
- `codex-usage-overlay_1.6.0-preview.1_x64_en-US.msi` and its `.sig`
- `CodexUsageOverlay-v1.6.0-preview.1-Windows-x64.zip`
- `SHA256SUMS.txt`
- `latest.json`

Local packaging check:

```powershell
cargo build --manifest-path src-tauri/Cargo.toml --locked --release --bin claude-usage-bridge
npm run tauri -- build --ci --bundles nsis,msi
cargo build --manifest-path src-tauri/Cargo.toml --locked --release --bin codex-usage-updater
.\scripts\Build-TauriRelease.ps1 -Tag v1.6.0-preview.1
```

The NSIS and MSI bundlers install every Cargo binary that exists at bundle time, including `claude-usage-bridge.exe`, so build the bridge before `tauri build`. Do not also list it as a bundle resource: the MSI would install the same file from two components and fail WiX validation (ICE30). The portable ZIP contains `CodexUsageOverlay.exe`, `CodexUsageUpdater.exe`, `claude-usage-bridge.exe`, and the Chinese/English README files. It does not include a PowerShell launcher.

Preview releases are published as GitHub prereleases with `latest.json`. Installed copies only see them when "Include preview releases" is on, because the stable endpoint (`releases/latest`) never names a prerelease.
