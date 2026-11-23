import React from 'react';
import { useTranslation } from 'react-i18next';
import { KIND_COLOR, KIND_LABEL_KEY, WindowKind } from '../../history';

/** Series identity never rests on colour alone: each key is a swatch plus its name. */
export const Legend: React.FC<{ kinds: WindowKind[]; shape?: 'line' | 'bar' }> = ({ kinds, shape = 'line' }) => {
  const { t } = useTranslation();
  if (kinds.length < 2) return null;
  return (
    <ul className="flex flex-wrap gap-4 text-xs" style={{ color: 'var(--viz-ink-2)' }}>
      {kinds.map(kind => (
        <li key={kind} className="flex items-center gap-1.5">
          <span
            aria-hidden
            className={shape === 'line' ? 'inline-block h-0.5 w-4 rounded' : 'inline-block h-2.5 w-2.5 rounded-sm'}
            style={{ background: KIND_COLOR[kind] }}
          />
          {t(KIND_LABEL_KEY[kind])}
        </li>
      ))}
    </ul>
  );
};
