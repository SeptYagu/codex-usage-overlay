# Publishing a version / 发布版本

Release ZIPs are built from the tagged source by GitHub Actions. Do not upload the tracked `output/` ZIP as a release asset.

GitHub Actions 会从标签对应的源码生成便携版压缩包；不要把仓库 `output/` 中的旧压缩包直接上传为发布附件。

1. Finish and push the changes to `main`.
2. Create and push a version tag on that commit:

   ```powershell
   git tag -a v0.1.1 -m "v0.1.1"
   git push origin v0.1.1
   ```

3. The `Release portable overlay` workflow checks the PowerShell scripts and both translations, builds the ZIP, verifies every packaged file and its SHA256 hash, then publishes a GitHub Release with both files.

Use the next version number instead of `v0.1.1`. To check the package locally before tagging, run:

```powershell
$releaseOutput = Join-Path $env:TEMP 'codex-usage-release-check'
& .\scripts\Build-Portable.ps1 -OutputDirectory $releaseOutput
& .\scripts\Test-Portable.ps1 -OutputDirectory $releaseOutput
```
