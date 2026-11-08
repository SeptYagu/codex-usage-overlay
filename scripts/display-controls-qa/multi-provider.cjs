const { spawn } = require('node:child_process');
const { chromium } = require(process.argv[2] || 'playwright');
const fs = require('node:fs');
(async () => {
    fs.mkdirSync('output/multi-provider-qa', { recursive: true });
    const server = spawn(process.execPath, ['node_modules/vite/bin/vite.js', '--config', 'scripts/display-controls-qa/vite.config.mjs'], { windowsHide: true, stdio: 'pipe' });
    let log = ''; server.stdout.on('data', b => log += b); server.stderr.on('data', b => log += b);
    let browser; const results = []; const failures = [];
    try {
        for (let attempt = 0; ; attempt++) {
            if (server.exitCode !== null || attempt > 40) throw new Error('QA server failed: ' + log);
            try { if ((await fetch('http://127.0.0.1:1427/scripts/display-controls-qa/index.html')).ok) break; } catch {}
            await new Promise(r => setTimeout(r, 250));
        }
        browser = await chromium.launch({ channel: 'msedge', headless: true });
        for (const colorScheme of ['light', 'dark']) for (const dpr of [1, 1.75]) {
            const context = await browser.newContext({ colorScheme, deviceScaleFactor: dpr, viewport: { width: 1600, height: 1000 } });
            const page = await context.newPage();
            await page.goto('http://127.0.0.1:1427/scripts/display-controls-qa/index.html');
            await page.waitForFunction(() => window.qa);
            for (const language of ['en-US', 'zh-CN', 'zh-Hant']) {
                for (const overlayLayout of ['grouped', 'stacks']) for (const scalePercent of [100, 250]) {
                    await page.setViewportSize({ width: 1600, height: 1000 });
                    await page.evaluate(settings => window.qa.render(settings, null, false, 'overlay', 'multi'), { provider: 'codex', language, overlayLayout, scalePercent });
                    await page.evaluate(() => window.qaInvocations = []);
                    const order = ['claude', 'dsh', 'codex']; const values = [41, 23, 69];
                    for (let index = 0; index < 3; index++) {
                        const logo = page.locator('.overlay-provider');
                        if (index === 0) await logo.click();
                        else { await logo.focus(); await page.keyboard.press(index === 1 ? 'Enter' : 'Space'); }
                        await page.waitForFunction(provider => document.querySelector('.overlay-provider svg')?.getAttribute('data-provider') === provider, order[index]);
                        await page.waitForFunction(() => !document.querySelector('.overlay-provider').disabled);
                        const metrics = await page.evaluate(() => {
                            const capsule = document.querySelector('.overlay-capsule'); const rect = capsule.getBoundingClientRect();
                            const logo = capsule.querySelector('.overlay-provider'); const mark = logo.getBoundingClientRect();
                            return { text: capsule.textContent, label: logo.getAttribute('aria-label'),
                                pointer: getComputedStyle(logo).pointerEvents,
                                clipped: [...capsule.querySelectorAll('span,button')].some(el => { const r = el.getBoundingClientRect(); return r.left < rect.left - .1 || r.right > rect.right + .1 || r.bottom > rect.bottom + .1; }),
                                drag: window.qaInvocations.includes('start_dragging'),
                                credits: Boolean(capsule.querySelector('.overlay-credit')),
                                mark: { width: mark.width, height: mark.height } };
                        });
                        const item = { colorScheme, dpr, language, overlayLayout, scalePercent, provider: order[index], ...metrics }; results.push(item);
                        if (!metrics.text.includes(`${values[index]}%`) || metrics.pointer !== 'auto' || metrics.drag || metrics.clipped || /switchProviderHint|singleProviderHint/.test(metrics.label)
                            || metrics.credits !== (order[index] !== 'claude')) failures.push(item);
                    }
                    if (dpr === 1 && scalePercent === 100 && overlayLayout === 'grouped') await page.locator('.overlay-capsule').screenshot({ path: `output/multi-provider-qa/overlay-${language}-${colorScheme}.png` });
                }
                for (const size of [{ width:1320,height:800 },{ width: 960, height: 620 }, { width: 760, height: 500 }]) {
                    await page.setViewportSize(size);
                    await page.evaluate(language => window.qa.render({ language, provider: 'codex' }, null, false, 'settings'), language);
                    for (const provider of ['codex', 'claude', 'dsh']) {
                        await page.locator(`#settings-tab-${provider}`).click();
                        await page.locator(`[data-provider-settings="${provider}"]`).scrollIntoViewIfNeeded();
                        const metrics = await page.evaluate(provider => {
                            const grid = document.querySelector(`[data-provider-settings="${provider}"]`);
                            // Controls inside details become reachable by opening
                            // the disclosure, rather than scrolling a closed one.
                            for (const details of grid.querySelectorAll('details')) details.open = true;
                            const panel = document.querySelector('.settings-content');
                            const overflow = grid.scrollWidth > grid.clientWidth;
                            const inaccessible = [];
                            const accessible = [...grid.querySelectorAll('input,select,button')].every(control => {
                                const column = panel;
                                control.scrollIntoView({ block: 'nearest' });
                                const r = control.getBoundingClientRect(), c = column.getBoundingClientRect();
                                const fits = r.left >= c.left - 1 && r.right <= c.right + 1 && r.top >= c.top - 1 && r.bottom <= c.bottom + 1;
                                if (!fits) inaccessible.push({ label: control.getAttribute('aria-label') || control.id || control.tagName, top: r.top, bottom: r.bottom, columnTop: c.top, columnBottom: c.bottom });
                                return fits;
                            });
                            panel.scrollTop = 0;
                            const ids = [...document.querySelectorAll('[id]')].map(el=>el.id);
                            return { display: document.querySelector('[aria-pressed=true]')?.closest('[data-provider-settings]')?.getAttribute('data-provider-settings') ?? null, configured: grid.getAttribute('data-provider-settings'), duplicateIds:ids.length!==new Set(ids).size, columns:document.querySelectorAll('[data-provider-settings]').length,
                                overflow, accessible, inaccessible, rootScroll: document.documentElement.scrollHeight > innerHeight,
                                missingTranslation: /monitorHealth\.|displayedProviderHint|monitorProvider|usageProviderHint/.test(grid.textContent) };
                        }, provider);
                        const item = { settings: true, colorScheme, dpr, language, provider, size, ...metrics }; results.push(item);
                        if (metrics.display !== (provider === 'codex' ? 'codex' : null) || metrics.configured !== provider || metrics.columns !== 1 || metrics.duplicateIds || metrics.overflow || !metrics.accessible || metrics.missingTranslation) failures.push(item);
                        if (dpr === 1 && size.width >= 960) await page.screenshot({ path: `output/multi-provider-qa/settings-${language}-${provider}-${size.width}-${colorScheme}.png` });
                    }
                }
            }
            await context.close();
        }
        fs.writeFileSync('output/multi-provider-qa/results.json', JSON.stringify({ cases: results.length, failures, results }, null, 2));
        console.log(JSON.stringify({ cases: results.length, failures: failures.length, examples: failures.slice(0, 3) }, null, 2));
        process.exitCode = failures.length ? 1 : 0;
    } finally { if (browser) await browser.close(); server.kill(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
