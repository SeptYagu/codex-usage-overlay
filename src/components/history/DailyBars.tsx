import React, { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { HistoryDay, KIND_COLOR, KIND_LABEL_KEY, WindowKind, formatDay, formatPercent, niceScale } from '../../history';
import { Legend } from './Legend';
import { useChartWidth } from './useChartWidth';

const HEIGHT = 220;
const M = { top: 12, right: 16, bottom: 28, left: 44 };
const MAX_BAR = 24;
const BAR_GAP = 2;

/** A bar with a 4px rounded data end and a square foot on the baseline. */
function barPath(x: number, y: number, w: number, h: number): string {
  const r = Math.min(4, w / 2, h);
  return `M${x},${y + h}V${y + r}Q${x},${y} ${x + r},${y}H${x + w - r}Q${x + w},${y} ${x + w},${y + r}V${y + h}Z`;
}

export const DailyBars: React.FC<{ days: HistoryDay[]; kinds: WindowKind[] }> = ({ days, kinds }) => {
  const { t, i18n } = useTranslation();
  const locale = i18n.resolvedLanguage;
  const [ref, width] = useChartWidth();
  const [active, setActive] = useState<number | null>(null);
  const [showTable, setShowTable] = useState(false);
  const plotW = Math.max(10, width - M.left - M.right);
  const plotH = HEIGHT - M.top - M.bottom;
  const { max, ticks: yTicks } = niceScale(Math.max(0, ...days.flatMap(day => kinds.map(kind => day[kind]))));
  const y = (value: number) => M.top + (1 - value / max) * plotH;
  const band = plotW / Math.max(1, days.length);
  const barW = Math.max(1, Math.min(MAX_BAR, (band * 0.7 - BAR_GAP * (kinds.length - 1)) / Math.max(1, kinds.length)));
  const groupW = barW * kinds.length + BAR_GAP * (kinds.length - 1);
  const labelEvery = Math.max(1, Math.ceil(days.length / Math.max(2, Math.floor(plotW / 70))));
  const day = active !== null ? days[active] : null;
  const tooltipLeft = active !== null ? Math.min(Math.max(M.left + band * (active + 1), 0), Math.max(0, width - 190)) : 0;

  return (
    <div className="space-y-2">
      <div className="flex items-center justify-between gap-2">
        <Legend kinds={kinds} shape="bar" />
        <button type="button" className="ml-auto text-xs rounded border border-slate-300 dark:border-slate-600 px-2 py-0.5" aria-pressed={showTable} onClick={() => setShowTable(value => !value)}>
          {t(showTable ? 'historyHideTable' : 'historyShowTable')}
        </button>
      </div>
      <div ref={ref} className="relative">
        <svg className="viz-chart block" width={width} height={HEIGHT} role="img" aria-label={t('historyDailyTitle')} onPointerLeave={() => setActive(null)}>
          {yTicks.map(tick => (
            <g key={tick}>
              <line x1={M.left} x2={M.left + plotW} y1={y(tick)} y2={y(tick)} stroke={tick === 0 ? 'var(--viz-axis)' : 'var(--viz-grid)'} strokeWidth={1} />
              <text x={M.left - 8} y={y(tick)} dy="0.32em" textAnchor="end" fontSize={11} fill="var(--viz-muted)">{Math.round(tick)}%</text>
            </g>
          ))}
          {days.map((entry, index) => {
            const left = M.left + band * index + (band - groupW) / 2;
            return (
              <g
                key={entry.day}
                data-day={entry.day}
                tabIndex={0}
                aria-label={`${formatDay(entry.start, locale)}: ${kinds.map(kind => `${t(KIND_LABEL_KEY[kind])} ${formatPercent(entry[kind])}`).join(', ')}`}
                onPointerEnter={() => setActive(index)}
                onFocus={() => setActive(index)}
                onBlur={() => setActive(null)}
              >
                <rect x={M.left + band * index} y={M.top} width={band} height={plotH} fill="transparent" />
                {kinds.map((kind, k) => {
                  const value = entry[kind];
                  if (value <= 0) return null;
                  const top = y(value);
                  return (
                    <path
                      key={kind}
                      data-bar={kind}
                      d={barPath(left + k * (barW + BAR_GAP), top, barW, Math.max(1, M.top + plotH - top))}
                      fill={KIND_COLOR[kind]}
                      fillOpacity={entry.approximate ? 0.55 : 1}
                    />
                  );
                })}
                {index % labelEvery === 0 && (
                  <text x={M.left + band * (index + 0.5)} y={HEIGHT - 8} textAnchor="middle" fontSize={11} fill="var(--viz-muted)">{formatDay(entry.start, locale)}</text>
                )}
              </g>
            );
          })}
        </svg>
        {day && (
          <div role="tooltip" className="viz-tooltip" style={{ left: tooltipLeft, top: M.top }}>
            <div className="mb-1 text-[11px]" style={{ color: 'var(--viz-ink-2)' }}>{formatDay(day.start, locale)}</div>
            {kinds.map(kind => (
              <div key={kind} className="flex items-center justify-between gap-3">
                <span className="flex items-center gap-1.5" style={{ color: 'var(--viz-ink-2)' }}>
                  <span aria-hidden className="inline-block h-2 w-2 rounded-sm" style={{ background: KIND_COLOR[kind] }} />
                  {t(KIND_LABEL_KEY[kind])}
                </span>
                <strong>{formatPercent(day[kind])}</strong>
              </div>
            ))}
            {day.approximate && <div className="mt-1 text-[11px]" style={{ color: 'var(--viz-ink-2)' }}>{t('historyApproximate')}</div>}
          </div>
        )}
      </div>
      {showTable && (
        <table className="w-full text-xs tabular-nums">
          <thead>
            <tr className="text-left" style={{ color: 'var(--viz-ink-2)' }}>
              <th className="py-1 font-medium">{t('historyDate')}</th>
              {kinds.map(kind => <th key={kind} className="py-1 font-medium text-right">{t(KIND_LABEL_KEY[kind])}</th>)}
              <th className="py-1" />
            </tr>
          </thead>
          <tbody>
            {[...days].reverse().map(entry => (
              <tr key={entry.day} className="border-t" style={{ borderColor: 'var(--viz-grid)' }}>
                <td className="py-1">{formatDay(entry.start, locale)}</td>
                {kinds.map(kind => <td key={kind} className="py-1 text-right">{formatPercent(entry[kind])}</td>)}
                <td className="py-1 pl-3" style={{ color: 'var(--viz-ink-2)' }}>{entry.approximate ? t('historyApproximate') : ''}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
};
