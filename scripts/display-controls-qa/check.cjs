const { spawn } = require('node:child_process');
const { chromium } = require(process.argv[2] || 'playwright');
const fs = require('node:fs');
(async () => {
    fs.mkdirSync('output/display-qa', { recursive: true });
    const server = spawn(process.execPath, ['node_modules/vite/bin/vite.js', '--config', 'scripts/display-controls-qa/vite.config.mjs'], { windowsHide: true, stdio: 'pipe' });
    let log = '';
    server.stdout.on('data', b => log += b);
    server.stderr.on('data', b => log += b);
    let browser;
    try {
        for (let attempt = 0;; attempt++) {
            if (server.exitCode !== null)
                throw new Error("QA server failed: " + log);
            try {
                const res = await fetch('http://127.0.0.1:1427/scripts/display-controls-qa/index.html');
                if (res.ok)
                    break;
            }
            catch { }
            if (attempt === 40)
                throw new Error('QA server did not start: ' + log);
            await new Promise(r => setTimeout(r, 500));
        }
        browser = await chromium.launch({ channel: 'msedge', headless: true });
        const results = [];
        const failures = [];
        for (const dpr of [1, 1.75]) {
            const context = await browser.newContext({ viewport: { width: 1600, height: 1000 }, deviceScaleFactor: dpr });
            const page = await context.newPage();
            await page.goto('http://127.0.0.1:1427/scripts/display-controls-qa/index.html', { waitUntil: 'domcontentloaded', timeout: 60000 });
            await page.waitForFunction(() => window.qa);
            for (const language of ['en-US', 'zh-CN', 'zh-Hant'])
                for (const overlayLayout of ['grouped', 'stacks'])
                    for (const five of [true, false])
                        for (const burn of [true, false])
                            for (const credit of [true, false])
                                for (const scale of [100, 250]) {
                                    const settings = { language, overlayLayout, showFiveHourQuota: five, showBurnRate: burn, showCredits: credit, scalePercent: scale };
                                    await page.evaluate(s => window.qa.render(s), settings);
                                    const metrics = await page.evaluate(() => { const e = document.querySelector('.overlay-capsule'); const r = e.getBoundingClientRect(); return { width: r.width, height: r.height, groups: document.querySelectorAll('.overlay-quota').length, dividers: document.querySelectorAll('.overlay-divider').length, rateSpans: document.querySelectorAll('.overlay-burn-rate').length, clipped: [...e.querySelectorAll('span')].some(s => { const a = s.getBoundingClientRect(); return a.right > r.right + .1 || a.bottom > r.bottom + .1; }) }; });
                                    const baseWidth = (five ? 2 : 1) * (burn ? 140 : 100) + (credit ? 100 : 0) + (overlayLayout === 'stacks' ? 40 : 0);
                                    const baseHeight = overlayLayout === 'stacks' ? 100 : 70;
                                    const item = { ...settings, dpr, ...metrics, baseWidth, baseHeight };
                                    results.push(item);
                                    if (metrics.width > baseWidth * scale / 100 + 1 || metrics.height > baseHeight * scale / 100 + 1 || metrics.clipped)
                                        failures.push(item);
                                    if (dpr === 1 && language === 'en-US' && scale === 100 && credit && burn)
                                        await page.locator('.overlay-capsule').screenshot({ path: `output/display-qa/${overlayLayout}-${five ? 'dual' : 'weekly'}.png` });
                                }
            for (const edge of ['left', 'right', 'top', 'bottom'])
                for (const missing of [false, true]) {
                    await page.evaluate(({ edge, missing }) => window.qa.render({ language: 'zh-CN', showFiveHourQuota: false, showPercentageGrid: true, scalePercent: 100 }, edge, missing), { edge, missing });
                    const pill = await page.evaluate(() => { const host = document.querySelector('.overlay-pill-host').getBoundingClientRect(), bar = document.querySelector('.overlay-pill-track').getBoundingClientRect(), bars = document.querySelector('.overlay-pill-bars').getBoundingClientRect(); return { host: { width: host.width, height: host.height }, thickness: Math.min(bar.width, bar.height), centered: Math.abs((bar.left + bar.right) / 2 - (bars.left + bars.right) / 2) < .1 && Math.abs((bar.top + bar.bottom) / 2 - (bars.top + bars.bottom) / 2) < .1, tracks: document.querySelectorAll('.overlay-pill-track').length, ticks: document.querySelectorAll('.overlay-pill-tick').length }; });
                    results.push({ edge, missing, dpr, ...pill });
                    if (!pill.centered || pill.thickness !== 9 || pill.tracks !== 1 || pill.ticks !== 9)
                        failures.push({ edge, missing, dpr, ...pill });
                    if (dpr === 1 && !missing)
                        await page.locator('.overlay-pill-host').screenshot({ path: `output/display-qa/pill-${edge}.png` });
                }
            await page.setViewportSize({ width: 960, height: 620 });
            for (const language of ['en-US', 'zh-CN', 'zh-Hant']) {
                await page.evaluate(language => window.qa.render({ language }, null, false, 'settings'), language);
                const m = await page.evaluate(() => ({ rootScroll: document.documentElement.scrollHeight > innerHeight, columns: [...document.querySelectorAll('section')].map(e => ({ client: e.clientHeight, scroll: e.scrollHeight })), checks: document.querySelectorAll('section:first-child input[type=checkbox]').length }));
                results.push({ language, dpr, settings: m });
                if (m.rootScroll || m.checks !== 6 || m.columns[0].scroll > m.columns[0].client)
                    failures.push({ language, dpr, settings: m });
                if (dpr === 1)
                    await page.screenshot({ path: `output/display-qa/settings-${language}.png` });
            }
            await context.close();
        }
        fs.writeFileSync('output/display-qa/results.json', JSON.stringify({ cases: results.length, failures, results }, null, 2));
        console.log(JSON.stringify({ cases: results.length, failures }, null, 2));
        process.exitCode = failures.length ? 1 : 0;
    }
    finally {
        if (browser)
            await browser.close();
        server.kill();
    }
})().catch(e => { console.error(e); process.exitCode = 1; });
