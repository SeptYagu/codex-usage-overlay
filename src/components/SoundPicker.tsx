import React, { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { useTranslation } from 'react-i18next';
import { QuotaKind } from '../types';

interface SoundPickerProps {
  kind: QuotaKind;
  path: string | null;
  disabled?: boolean;
  onChangePath: (path: string | null) => void;
}

const ERROR_TRANSLATIONS: Record<string, string> = {
  sound_unsupported_format: 'soundErrUnsupported',
  sound_path_missing: 'soundErrPathMissing',
  sound_undecodable: 'soundErrUndecodable',
  sound_device_unavailable: 'soundErrDevice',
  sound_audio_busy: 'soundErrBusy',
};

function errorKey(error: unknown): string {
  return String(error).split(':', 1)[0].trim();
}

export const SoundPicker: React.FC<SoundPickerProps> = ({ kind, path, disabled = false, onChangePath }) => {
  const { t } = useTranslation();
  const [error, setError] = useState<string | null>(null);
  const [previewing, setPreviewing] = useState(false);

  useEffect(() => {
    let unlistenError: (() => void) | undefined;
    let unlistenFinished: (() => void) | undefined;
    listen<{ kind?: QuotaKind; error: string }>('sound_playback_error', (event) => {
      if (!event.payload.kind || event.payload.kind === kind) {
        setError(errorKey(event.payload.error));
        setPreviewing(false);
      }
    }).then((unlisten) => { unlistenError = unlisten; }).catch(() => {});
    listen('sound_preview_finished', () => setPreviewing(false))
      .then((unlisten) => { unlistenFinished = unlisten; }).catch(() => {});
    return () => {
      unlistenError?.();
      unlistenFinished?.();
    };
  }, [kind]);

  const chooseFile = async () => {
    setError(null);
    try {
      const selected = await invoke<string | null>('pick_sound_file', { kind });
      if (selected) onChangePath(selected);
    } catch (reason) {
      setError(errorKey(reason));
    }
  };

  const preview = async () => {
    if (!path) return;
    if (previewing) {
      try {
        await invoke('stop_preview_sound');
      } finally {
        setPreviewing(false);
      }
      return;
    }
    setError(null);
    try {
      await invoke('preview_sound', { kind });
      setPreviewing(true);
    } catch (reason) {
      setError(errorKey(reason));
    }
  };

  return (
    <div className="space-y-2">
      <p className="break-all select-text rounded bg-white/70 dark:bg-slate-900/60 px-2 py-1 text-xs text-slate-600 dark:text-slate-300" title={path ?? undefined}>
        {path || t('chooseSoundFile')}
      </p>
      <div className="flex flex-wrap gap-2">
        <button type="button" className="rounded border border-slate-300 dark:border-slate-600 px-2 py-1 text-xs hover:bg-white dark:hover:bg-slate-700 disabled:opacity-50" disabled={disabled} onClick={chooseFile}>
          {t('chooseSoundFileAction')}
        </button>
        <button type="button" className="rounded border border-slate-300 dark:border-slate-600 px-2 py-1 text-xs hover:bg-white dark:hover:bg-slate-700 disabled:opacity-50" disabled={disabled || !path} onClick={preview}>
          {previewing ? t('stopPreview') : t('previewSound')}
        </button>
        <button type="button" className="rounded border border-slate-300 dark:border-slate-600 px-2 py-1 text-xs hover:bg-white dark:hover:bg-slate-700 disabled:opacity-50" disabled={disabled || !path} onClick={() => { setError(null); onChangePath(null); }}>
          {t('clearSoundFile')}
        </button>
      </div>
      {error && <p role="alert" className="text-xs text-rose-600 dark:text-rose-400">{t(ERROR_TRANSLATIONS[error] ?? 'soundErrGeneric')}</p>}
    </div>
  );
};
