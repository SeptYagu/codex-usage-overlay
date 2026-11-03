const { spawn } = require('node:child_process');
const { chromium } = require(process.argv[2] || 'playwright');
const fs = require('node:fs');

// Use a real rendering engine: DOM-only tests cannot detect invalid resolved
// colors or transforms that move the grid outside its native window bounds.
(async () => {
    const output = 'output/percentage-grid';
    fs.mkdirSync(output, { recursive: true });
    const server = spawn(process.execPath, ['node_modules/vite/bin/vite.js', '--config', 'scripts/display-controls-qa/vite.config.mjs'], { windowsHide: true, stdio: 'pipe' });
    let log = '';
    server.stdout.on('data', b => log += b);
    server.stderr.on('data', b => log += b);
    let browser;
    const results = [];
    try {
        for (let attempt = 0; ; attempt++) {
            if (server.exitCode !== null || attempt > 40) throw new Error('QA server failed: ' + log);
            try {
                if ((await fetch('http://127.0.0.1:1427/scripts/display-controls-qa/index.html')).ok) break;
            } catch {}
            await new Promise(resolve => setTimeout(resolve, 250));
        }
        browser = await chromium.launch({ channel: 'msedge', headless: true });
        for (const theme of ['light', 'dark']) for (const dpr of [1, 1.75]) {
            const context = await browser.newContext({ deviceScaleFactor: dpr, colorScheme: theme });
            const page = await context.newPage();
            await page.goto('http://127.0.0.1:1427/scripts/display-controls-qa/index.html');
            await page.waitForFunction(() => window.qa);
            for (const edge of ['left', 'right', 'top', 'bottom']) for (const scale of [100, 175, 250]) {
                const horizontal = edge === 'top' || edge === 'bottom';
                // Match the backend's scaled logical window, including rounding.
                await page.setViewportSize({ width: Math.round((horizontal ? 100 : 46) * scale / 100), height: Math.round((horizontal ? 46 : 100) * scale / 100) });
                for (const dual of [true, false]) for (const grid of [true, false]) for (const fixture of ['normal', 'full', 'empty', 'missing']) {
                    await page.evaluate(({ edge, scale, dual, grid, fixture }) => window.qa.render({ language: 'en-US', scalePercent: scale, showFiveHourQuota: dual, showPercentageGrid: grid }, edge, fixture === 'missing', 'overlay', fixture), { edge, scale, dual, grid, fixture });
                    // Let the fill-height transition settle before measuring it.
                    await page.evaluate(async () => {
                        await Promise.all(document.getAnimations().map(animation => animation.finished.catch(() => {})));
                    });
                    const metrics = await page.evaluate(() => {
                        const host = document.querySelector('.overlay-pill-host').getBoundingClientRect();
                        const inside = (rect, bounds) => rect.left >= bounds.left - .1 && rect.top >= bounds.top - .1 && rect.right <= bounds.right + .1 && rect.bottom <= bounds.bottom + .1;
                        const viewport = { left: 0, top: 0, right: innerWidth, bottom: innerHeight };
                        const tracks = [...document.querySelectorAll('.overlay-pill-track')].map(track => {
                            const rect = track.getBoundingClientRect();
                            const style = getComputedStyle(track);
                            const fill = track.querySelector('.overlay-pill-fill');
                            return {
                                background: style.backgroundColor,
                                clipped: !inside(rect, host) || !inside(rect, viewport),
                                thickness: Math.min(rect.width, rect.height),
                                length: Math.max(rect.width, rect.height),
                                // Use the unrotated CSS height for zero/small fills.
                                fillHeightPercent: fill ? parseFloat(getComputedStyle(fill).height) / parseFloat(style.height) * 100 : null,
                                ticks: [...track.querySelectorAll('.overlay-pill-tick')].map(tick => ({
                                    position: parseFloat(getComputedStyle(tick).bottom) / parseFloat(style.height) * 100,
                                    visible: getComputedStyle(tick).backgroundColor !== 'rgba(0, 0, 0, 0)' && inside(tick.getBoundingClientRect(), rect),
                                })),
                            };
                        });
                        return { tracks };
                    });
                    const expectedFill = fixture === 'missing' ? null : fixture === 'full' ? 100 : fixture === 'empty' ? 0 : dual ? [69, 62] : [62];
                    const errors = [];
                    if (metrics.tracks.length !== (dual ? 2 : 1)) errors.push('track count');
                    metrics.tracks.forEach((track, index) => {
                        if (track.background === 'rgba(0, 0, 0, 0)') errors.push('transparent track');
                        if (track.clipped) errors.push('clipped track');
                        if (Math.abs(track.thickness - 9 * scale / 100) > .1 || Math.abs(track.length - 72 * scale / 100) > .1) errors.push('track dimensions');
                        if (track.ticks.length !== (grid ? 9 : 0) || track.ticks.some((tick, i) => !tick.visible || Math.abs(tick.position - (i + 1) * 10) > .1)) errors.push('grid divisions');
                        const expected = Array.isArray(expectedFill) ? expectedFill[index] : expectedFill;
                        if (expected === null ? track.fillHeightPercent !== null : track.fillHeightPercent === null || Math.abs(track.fillHeightPercent - expected) > .1) errors.push('fill proportion');
                    });
                    results.push({ theme, dpr, edge, scale, dual, grid, fixture, errors, ...metrics });
                    if (dpr === 1 && grid && dual && fixture === 'normal' && scale !== 175) {
                        await page.screenshot({ path: `${output}/${theme}-${edge}-${scale}.png`, omitBackground: true });
                    }
                }
            }
            await context.close();
        }
        const failures = results.filter(result => result.errors.length);
        fs.writeFileSync(`${output}/results.json`, JSON.stringify({ cases: results.length, failures, results }, null, 2));
        console.log(JSON.stringify({ cases: results.length, failures: failures.length, examples: failures.slice(0, 3) }, null, 2));
        process.exitCode = failures.length ? 1 : 0;
    } finally {
        if (browser) await browser.close();
        server.kill();
    }
})().catch(error => { console.error(error); process.exitCode = 1; });
