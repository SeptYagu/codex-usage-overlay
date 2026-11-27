import React, { useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import {
  HistoryViewData, KIND_COLOR, KIND_LABEL_KEY, WindowKind,
  formatDateTime, formatPercent, isolatedPoints, linePath, nearestIndex, stackLabels, timeTicks,
} from '../../history';
import { Legend } from './Legend';
import { useChartWidth } from './useChartWidth';

const HEIGHT = 260;
const M = { top: 12, right: 16, bottom: 28, left: 44 };
const Y_TICKS = [0, 25, 50, 75, 100];

/** Readings are used percent; the chart shows used or remaining percent. */
export const TimelineChart: React.FC<{
  view: HistoryViewData;
  kinds: WindowKind[];
  thresholds: Partial<Record<WindowKind, number>>;
  showUsed?: boolean;
}> = ({ view, kinds, thresholds, showUsed = false }) => {
  const { t, i18n } = useTranslation();
  const locale = i18n.resolvedLanguage;
  const [ref, width] = useChartWidth();
  const [active, setActive] = useState<number | null>(null);
  const plotW = Math.max(10, width - M.left - M.right);
  const plotH = HEIGHT - M.top - M.bottom;
  const span = Math.max(1, view.to - view.from);
  const x = (time: number) => M.left + ((time - view.from) / span) * plotW;
  const y = (value: number) => M.top + (1 - Math.min(100, Math.max(0, value)) / 100) * plotH;
  const shown = (used: number) => (showUsed ? used : 100 - used);
  const yShown = (used: number) => y(shown(used));
  const ticks = useMemo(() => timeTicks(view.from, view.to, locale, Math.max(3, Math.floor(plotW / 110))), [view.from, view.to, locale, plotW]);
  const paths = kinds.map(kind => ({ kind, d: linePath(view.points, kind, x, yShown) }));
  const point = active !== null ? view.points[active] : null;
  const thresholdKinds = kinds.filter(kind => thresholds[kind] !== undefined);
  const thresholdLabelY = stackLabels(thresholdKinds.map(kind => yShown(thresholds[kind]!)));

  const pick = (clientX: number, rect: DOMRect) => {
    const time = view.from + ((clientX - rect.left - M.left) / plotW) * span;
    const index = nearestIndex(view.points, time);
    setActive(index >= 0 ? index : null);
  };

  const onKeyDown = (event: React.KeyboardEvent) => {
    if (!view.points.length) return;
    const last = view.points.length - 1;
    const step = event.key === 'ArrowRight' ? 1 : event.key === 'ArrowLeft' ? -1 : 0;
    if (event.key === 'Home') setActive(0);
    else if (event.key === 'End') setActive(last);
    else if (step) setActive(current => Math.min(last, Math.max(0, (current ?? (step > 0 ? -1 : last + 1)) + step)));
    else if (event.key === 'Escape') setActive(null);
    else return;
    event.preventDefault();
  };

  const tooltipLeft = point ? Math.min(Math.max(x(point.t) + 12, 0), Math.max(0, width - 190)) : 0;

  return (
    <div className="space-y-2">
      <Legend kinds={kinds} />
      <div ref={ref} className="relative">
        <svg
          className="viz-chart block"
          width={width}
          height={HEIGHT}
          role="img"
          tabIndex={0}
          aria-label={t(showUsed ? 'historyTimelineTitleUsed' : 'historyTimelineTitle')}
          onPointerMove={event => pick(event.clientX, event.currentTarget.getBoundingClientRect())}
          onPointerLeave={() => setActive(null)}
          onKeyDown={onKeyDown}
          onBlur={() => setActive(null)}
        >
          {view.gaps.map(gap => (
            <rect key={`gap-${gap.from}`} data-gap x={x(gap.from)} y={M.top} width={Math.max(1, x(gap.to) - x(gap.from))} height={plotH} fill="var(--viz-gap)" />
          ))}
          {Y_TICKS.map(tick => (
            <g key={tick}>
              <line x1={M.left} x2={M.left + plotW} y1={y(tick)} y2={y(tick)} stroke={tick === 0 ? 'var(--viz-axis)' : 'var(--viz-grid)'} strokeWidth={1} />
              <text x={M.left - 8} y={y(tick)} dy="0.32em" textAnchor="end" fontSize={11} fill="var(--viz-muted)">{tick}%</text>
            </g>
          ))}
          {ticks.map(tick => (
            <text key={tick.t} x={x(tick.t)} y={HEIGHT - 8} textAnchor="middle" fontSize={11} fill="var(--viz-muted)">{tick.label}</text>
          ))}
          {view.resets.filter(marker => kinds.includes(marker.kind)).map(marker => (
            <line key={`reset-${marker.kind}-${marker.t}`} data-reset={marker.kind} x1={x(marker.t)} x2={x(marker.t)} y1={M.top} y2={M.top + plotH} stroke={KIND_COLOR[marker.kind]} strokeOpacity={0.45} strokeWidth={1} />
          ))}
          {thresholdKinds.map((kind, index) => (
            <g key={`threshold-${kind}`} data-threshold={kind}>
              <line x1={M.left} x2={M.left + plotW} y1={yShown(thresholds[kind]!)} y2={yShown(thresholds[kind]!)} stroke={KIND_COLOR[kind]} strokeOpacity={0.55} strokeWidth={1} strokeDasharray="4 3" />
              <text x={M.left + plotW - 4} y={thresholdLabelY[index]} textAnchor="end" fontSize={10} fill="var(--viz-ink-2)">
                {t(KIND_LABEL_KEY[kind])} · {t('historyThreshold')}
              </text>
            </g>
          ))}
          {paths.map(({ kind, d }) => d && (
            <path key={kind} data-series={kind} d={d} fill="none" stroke={KIND_COLOR[kind]} strokeWidth={2} strokeLinejoin="round" strokeLinecap="round" />
          ))}
          {kinds.flatMap(kind => isolatedPoints(view.points, kind).map(p => (
            <circle key={`dot-${kind}-${p.t}`} data-isolated={kind} cx={x(p.t)} cy={yShown(p[kind]!)} r={2.5} fill={KIND_COLOR[kind]} />
          )))}
          {point && (
            <g pointerEvents="none">
              <line x1={x(point.t)} x2={x(point.t)} y1={M.top} y2={M.top + plotH} stroke="var(--viz-muted)" strokeWidth={1} />
              {kinds.map(kind => point[kind] !== null && point[kind] !== undefined && (
                <circle key={kind} cx={x(point.t)} cy={yShown(point[kind]!)} r={4} fill={KIND_COLOR[kind]} stroke="var(--viz-surface)" strokeWidth={2} />
              ))}
            </g>
          )}
          {!view.points.length && (
            <text x={M.left + plotW / 2} y={M.top + plotH / 2} textAnchor="middle" fontSize={12} fill="var(--viz-muted)">{t('historyNoReadings')}</text>
          )}
        </svg>
        {point && (
          <div role="tooltip" className="viz-tooltip" style={{ left: tooltipLeft, top: M.top }}>
            <div className="mb-1 text-[11px]" style={{ color: 'var(--viz-ink-2)' }}>
              {formatDateTime(point.t, locale)}{point.pool ? ` · ${t('historyPool')} ${point.pool}` : ''}
            </div>
            {kinds.map(kind => (
              <div key={kind} className="flex items-center justify-between gap-3">
                <span className="flex items-center gap-1.5" style={{ color: 'var(--viz-ink-2)' }}>
                  <span aria-hidden className="inline-block h-0.5 w-3 rounded" style={{ background: KIND_COLOR[kind] }} />
                  {t(KIND_LABEL_KEY[kind])}
                </span>
                <strong>{formatPercent(point[kind] === null || point[kind] === undefined ? null : shown(point[kind]!))}</strong>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
};
