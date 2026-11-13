import React from 'react';
import { createRoot } from 'react-dom/client';
import { OverlayView } from '/src/components/OverlayView';
import { SettingsView } from '/src/components/SettingsView';
import { TrayMenuView } from '/src/components/TrayMenuView';
import { setProtectionFixture } from './tauri-stub.js';
import { DEFAULT_SETTINGS } from '/src/types';
import i18n from '/src/i18n';
import '/src/index.css';
const root = createRoot(document.getElementById('root')!);
const usage = { fiveHourRemainingPercent: 69, fiveHourBurnRatePerHour: 16.8, weekRemainingPercent: 62, weekBurnRatePerHour: 0.6, creditsDisplay: '12.50', creditsBalance: '12.50', hasCredits: true, fiveHourResetsAt: Date.now() / 1000 + 18000, weekResetsAt: Date.now() / 1000 + 604800, fetchedAt: Date.now() / 1000 };
window.qa = { render: async (settings, edge = null, missing = false, view = 'overlay', fixture = 'normal') => {
        await i18n.changeLanguage(settings.language);
        const s = { ...DEFAULT_SETTINGS, ...settings };
        const order = ['codex', 'claude', 'dsh', 'antigravity'];
        const samples = {
            codex: usage,
            claude: { ...usage, fiveHourRemainingPercent: 41, weekRemainingPercent: 38, hasCredits: false },
            dsh: { ...usage, fiveHourRemainingPercent: 23, weekRemainingPercent: 12 },
            antigravity: { ...usage, fiveHourRemainingPercent: 55, weekRemainingPercent: 48, hasCredits: false },
        };
        const monitors = Object.fromEntries(order.map(provider => [provider, {
            provider, sourceKey: provider, sourceGeneration: 0, revision: 1, monitorEnabled: true,
            available: true, health: 'current', error: null, usage: {...samples[provider],sourceKey:provider,sourceGeneration:0},
        }]));
        setProtectionFixture(fixture);
        const sample = fixture === 'full' ? { ...usage, fiveHourRemainingPercent: 100, weekRemainingPercent: 100 }
            : fixture === 'empty' ? { ...usage, fiveHourRemainingPercent: 0, weekRemainingPercent: 0 }
            : ['multi','carousel'].includes(fixture) ? samples[s.provider] : usage;
        root.render(view === 'settings' ? <SettingsView settings={s} onPatchSettings={() => { }}/> : view === 'tray'
            ? <TrayMenuView settings={s} onPatchSettings={() => { }}/>
            : <OverlayView settings={s} usage={missing ? null : sample} isLoading={missing}
                monitors={['multi','carousel'].includes(fixture) ? monitors : {}}
                onCycleProvider={fixture === 'multi' ? async () => {
                    await window.qa.render({ ...settings, provider: order[(order.indexOf(s.provider) + 1) % 4] }, edge, missing, view, fixture);
                } : undefined}
                dockState={edge ? { docked: true, expanded: false, hidden: false, edge } : undefined}/>);
        await document.fonts.ready;
        await new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)));
    } };
