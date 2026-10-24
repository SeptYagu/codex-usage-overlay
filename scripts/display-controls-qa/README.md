# Display controls browser QA

Run from the repository root with Node, installed project dependencies, Playwright, and Microsoft Edge:

```powershell
node scripts/display-controls-qa/check.cjs
```

If Playwright is provided by a separate runtime rather than this project's `node_modules`, pass its module directory as the first argument:

```powershell
node scripts/display-controls-qa/check.cjs C:\path\to\node_modules\playwright
```

The runner starts a loopback-only Vite server on port 1427, renders the production React components/CSS with offline Tauri effects, and closes its browser and server on completion. Keep port 1427 free. Its 616 cases cover 384 expanded overlay compositions, 16 Weekly-only pills, six original Settings layouts, 168 protection Settings layouts and 42 protection tray menus. Protection fixtures include armed, low, accepted-but-unconfirmed, bridge failure, quota retry, quota failure, and stale states in all three languages, at device scale factors 1 and 1.75; Settings run at 960×620 and 760×500 with collapsed and expanded details. It rejects undersized provisional budgets, clipped spans, stretched/off-center pill bars, inaccessible controls or horizontal overflow. Reports/screenshots are saved under ignored `output/display-qa/`.

These checks exercise Edge rendering and simulated DPI. Native WebView2 sizing, OS tray installation, mixed-monitor transitions and live hover capture still require a separate desktop smoke test before packaging or release.
