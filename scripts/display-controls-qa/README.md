# Display controls browser QA

Run from the repository root with Node, installed project dependencies, Playwright, and Microsoft Edge:

```powershell
node scripts/display-controls-qa/check.cjs
```

If Playwright is provided by a separate runtime rather than this project's `node_modules`, pass its module directory as the first argument:

```powershell
node scripts/display-controls-qa/check.cjs C:\path\to\node_modules\playwright
```

The runner starts a loopback-only Vite server on port 1427, renders the production React components/CSS with offline Tauri effects, and closes its browser and server on completion. Keep port 1427 free. It checks 384 expanded compositions (normal and 100% quota fixtures, three languages, two layouts, eight visibility combinations, two UI scales and two device scale factors), 16 Weekly-only pill cases, and six settings-window cases at 960x620. It rejects undersized provisional budgets, clipped spans, stretched/off-center pill bars, or default settings-column overflow. Reports/screenshots are saved under ignored `output/display-qa/`.

These checks exercise Edge rendering and simulated DPI. Native WebView2 sizing, OS tray installation, mixed-monitor transitions and live hover capture still require a separate desktop smoke test before packaging or release.
