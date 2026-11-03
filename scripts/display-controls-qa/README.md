# Display controls browser QA

Run from the repository root with Node, installed project dependencies, Playwright, and Microsoft Edge:

```powershell
node scripts/display-controls-qa/check.cjs
```

If Playwright is provided by a separate runtime rather than this project's `node_modules`, pass its module directory as the first argument:

```powershell
node scripts/display-controls-qa/check.cjs C:\path\to\node_modules\playwright
```

The runner starts a loopback-only Vite server on port 1427, renders the production React components/CSS with offline Tauri effects, and closes its browser and server on completion. Keep port 1427 free. Its 634 cases cover 384 expanded overlay compositions, 16 Weekly-only pills, 24 Settings layouts with edge auto-hide on/off at both window sizes, 168 protection Settings layouts and 42 protection tray menus. Protection fixtures include armed, low, accepted-but-unconfirmed, bridge failure, quota retry, quota failure, and stale states in all three languages, at device scale factors 1 and 1.75; Settings run at 960×620 and 760×500 with collapsed and expanded details. It rejects undersized provisional budgets, clipped spans, stretched/off-center pill bars, inaccessible controls, missing descriptions, incorrect grid-option visibility or horizontal overflow. Reports/screenshots are saved under ignored `output/display-qa/`.

These checks exercise Edge rendering and simulated DPI. Native WebView2 sizing, OS tray installation, mixed-monitor transitions and live hover capture still require a separate desktop smoke test before packaging or release.

`node scripts/display-controls-qa/multi-provider.cjs` adds 216 cases for mouse/Enter/Space logo switching, provider-specific credits, display/configuration independence, reachable Settings controls, and translations across both themes, layouts, supported window sizes and device scale factors. It accepts the same optional Playwright directory argument. Reports/screenshots go under ignored `output/multi-provider-qa/`.

For focused percentage-grid regression checks, run:

```powershell
node scripts/display-controls-qa/percentage-grid.cjs
```

The optional first argument accepts the same external Playwright module directory as `check.cjs`. Its 768 cases check light/dark themes, all four edges, 100%/175%/250% scales, device scale factors 1 and 1.75, one/two quotas, grid on/off, and partial/full/zero/unknown quota. It verifies visible track backgrounds, nine evenly spaced grid divisions, fill proportions, and clipping against the scaled native-window bounds. Results and screenshots go under ignored `output/percentage-grid/`.
