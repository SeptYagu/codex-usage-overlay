import React from 'react';
import { createRoot } from 'react-dom/client';
import { OverlayView } from '/src/components/OverlayView';
import { SettingsView } from '/src/components/SettingsView';
import { DEFAULT_SETTINGS } from '/src/types';
import i18n from '/src/i18n';
import '/src/index.css';
const root = createRoot(document.getElementById('root')!);
const usage = { fiveHourRemainingPercent: 69, fiveHourBurnRatePerHour: 16.8, weekRemainingPercent: 62, weekBurnRatePerHour: 0.6, creditsDisplay: '12.50', creditsBalance: '12.50', hasCredits: true, fiveHourResetsAt: Date.now() / 1000 + 18000, weekResetsAt: Date.now() / 1000 + 604800, fetchedAt: Date.now() / 1000 };
window.qa = { render: async (settings, edge = null, missing = false, view = 'overlay', fixture = 'normal') => {
        await i18n.changeLanguage(settings.language);
        const s = { ...DEFAULT_SETTINGS, ...settings };
        const sample = fixture === 'full' ? { ...usage, fiveHourRemainingPercent: 100, weekRemainingPercent: 100 } : usage;
        root.render(view === 'settings' ? <SettingsView settings={s} onPatchSettings={() => { }}/> : <OverlayView settings={s} usage={missing ? null : sample} isLoading={missing} dockState={edge ? { docked: true, expanded: false, hidden: false, edge } : undefined}/>);
        await document.fonts.ready;
        await new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)));
    } };
