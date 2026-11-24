import React, { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { HistoryWindow, KIND_COLOR, KIND_LABEL_KEY, WindowKind, formatDateTime, formatDuration, formatPercent } from '../../history';

export const WindowTable: React.FC<{ windows: HistoryWindow[]; kinds: WindowKind[] }> = ({ windows, kinds }) => {
  const { t, i18n } = useTranslation();
  const locale = i18n.resolvedLanguage;
  const [filter, setFilter] = useState<WindowKind | 'all'>('all');
  const shown = windows.filter(window => kinds.includes(window.kind) && (filter === 'all' || window.kind === filter));
  const filters: (WindowKind | 'all')[] = ['all', ...kinds];

  return (
    <div className="space-y-2">
      <div role="group" aria-label={t('historyColWindow')} className="flex gap-1">
        {filters.map(option => (
          <button
            key={option}
            type="button"
            aria-pressed={filter === option}
            onClick={() => setFilter(option)}
            className={`text-xs rounded-full px-2.5 py-0.5 border ${filter === option ? 'border-cyan-600 text-cyan-700 dark:text-cyan-300' : 'border-slate-300 dark:border-slate-600'}`}
          >
            {option === 'all' ? t('historyFilterAll') : t(KIND_LABEL_KEY[option])}
          </button>
        ))}
      </div>
      {shown.length === 0 ? (
        <p className="text-xs" style={{ color: 'var(--viz-ink-2)' }}>{t('historyWindowsEmpty')}</p>
      ) : (
        <div className="overflow-x-auto">
          <table className="w-full text-xs tabular-nums">
            <thead>
              <tr className="text-left" style={{ color: 'var(--viz-ink-2)' }}>
                <th className="py-1 pr-3 font-medium">{t('historyColWindow')}</th>
                <th className="py-1 pr-3 font-medium">{t('historyColStart')}</th>
                <th className="py-1 pr-3 font-medium">{t('historyColResets')}</th>
                <th className="py-1 pr-3 font-medium text-right">{t('historyColPeak')}</th>
                <th className="py-1 pr-3 font-medium">{t('historyColLimit')}</th>
                <th className="py-1 pr-3 font-medium text-right">{t('historyColTimeToLimit')}</th>
                <th className="py-1 font-medium text-right">{t('historyColUnused')}</th>
              </tr>
            </thead>
            <tbody>
              {shown.map(window => (
                <tr key={`${window.kind}-${window.resetsAt}-${window.firstSeen}`} data-window={window.kind} className="border-t" style={{ borderColor: 'var(--viz-grid)' }}>
                  <td className="py-1 pr-3">
                    <span className="flex items-center gap-1.5">
                      <span aria-hidden className="inline-block h-2 w-2 rounded-sm" style={{ background: KIND_COLOR[window.kind] }} />
                      {t(KIND_LABEL_KEY[window.kind])}{window.pool ? ` · ${window.pool}` : ''}
                    </span>
                  </td>
                  <td className="py-1 pr-3">{formatDateTime(window.start, locale)}</td>
                  <td className="py-1 pr-3">{formatDateTime(window.resetsAt, locale)}</td>
                  <td className="py-1 pr-3 text-right">{formatPercent(window.peak)}</td>
                  <td className="py-1 pr-3">{window.hitLimit ? t('historyYes') : t('historyNo')}</td>
                  <td className="py-1 pr-3 text-right">{window.timeToLimitSecs !== null ? formatDuration(window.timeToLimitSecs, t) : '—'}</td>
                  <td className="py-1 text-right">{window.complete ? formatPercent(window.unusedAtReset) : t('historyInProgress')}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
};
