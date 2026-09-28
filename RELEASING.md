# Publishing a Tauri release / 发布 Tauri 版本

GitHub Actions builds release assets from the tagged source. Use locally built packages for verification; published packages come from the workflow.

Generate a Tauri updater key once with `npm run tauri -- signer generate --write-keys <private-key-path>`. Put the public key in `src-tauri/tauri.conf.json` and store the private key as the GitHub Actions secret `TAURI_SIGNING_PRIVATE_KEY`. Keep the private key outside the repository. If the key is password-protected, also set `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`.

GitHub Actions 从标签对应的源码生成发布附件。本机构建仅用于验证，正式附件由发布流程生成。

1. Update the app version in `package.json`, `package-lock.json`, `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock`, and `src-tauri/tauri.conf.json`.
2. Run `npm test`, `npm run build`, and `cargo test --manifest-path src-tauri/Cargo.toml --locked`. Complete Windows smoke checks and record any remaining limitations in preview release notes.
3. Commit and push the source branch (`main`). Stable releases are tagged directly on `main`; preview tags use `vX.Y.Z-preview`.
4. Tag that exact commit with the matching app version:

```powershell
git tag -a v1.0.1 -m "v1.0.1"
git push origin v1.0.1
```

5. The **Release overlay** workflow installs dependencies, runs frontend/Rust tests, builds NSIS and MSI installers, and packages the standalone executable into a ZIP. It verifies ZIP contents against the source executable and documentation and generates SHA256 checksums for all three packages.
6. A tag containing a prerelease suffix creates a GitHub prerelease and does not replace the latest stable release. If a draft release already exists for the tag, the workflow uploads assets to that draft; publish it after the workflow succeeds and the expected installers, signatures, portable ZIP, checksums, and `latest.json` are verified.

Expected assets for `v1.0.1`:

- `codex-usage-overlay_1.0.1_x64-setup.exe` and its `.sig`
- `codex-usage-overlay_1.0.1_x64_en-US.msi` and its `.sig`
- `CodexUsageOverlay-v1.0.1-Windows-x64.zip`
- `SHA256SUMS.txt`
- `latest.json`

Local packaging check:

```powershell
npm run tauri -- build --ci --bundles nsis,msi
cargo build --manifest-path src-tauri/Cargo.toml --locked --release --bin codex-usage-updater
.\scripts\Build-TauriRelease.ps1 -Tag v1.0.1
```

The portable ZIP contains `CodexUsageOverlay.exe`, `CodexUsageUpdater.exe`, and the Chinese/English README files. It does not include a PowerShell launcher.
