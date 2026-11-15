import React from 'react';
import { useTranslation } from 'react-i18next';
import { AntigravityConnectionStatus, AntigravityPool, OverlaySettings } from '../types';
import { BridgeConnectionCard } from './BridgeConnectionCard';

interface AntigravityConnectionViewProps {
  settings: OverlaySettings;
  onPatchSettings: (patch: Partial<OverlaySettings>, debounce?: boolean) => void;
}

const POOLS: AntigravityPool[] = ['auto', 'gemini', 'argon', '3p'];

/**
 * Connects the bundled `antigravity-usage-bridge` helper to Antigravity CLI
 * (statusline + `PreInvocation` guard hook) and selects which quota pool is projected.
 */
export const AntigravityConnectionView: React.FC<AntigravityConnectionViewProps> = ({ settings, onPatchSettings }) => {
  const { t } = useTranslation();
  const pool = settings.antigravityPool ?? 'auto';
  const live = settings.antigravityLivePolling ?? true;

  return (
    <BridgeConnectionCard<AntigravityConnectionStatus>
      settings={settings}
      provider="antigravity"
      statusCommand="antigravity_connection_status"
      connectCommand="antigravity_connect"
      disconnectCommand="antigravity_disconnect"
      liveChecked={live}
      onToggleLive={(checked) => onPatchSettings({ antigravityLivePolling: checked })}
      titleKey="antigravityConnection"
      connectedKey="antigravityConnected"
      partlyConnectedKey="antigravityPartlyConnected"
      notConnectedKey="antigravityNotConnected"
      bridgeMissingKey="antigravityBridgeMissing"
      workingKey="antigravityWorking"
      connectKey="antigravityConnect"
      disconnectKey="antigravityDisconnect"
      livePollingKey="antigravityLivePolling"
      livePollingHintKey="antigravityLivePollingHint"
      protectionSummariesPrefix="antigravityProtectionSummaries"
      blockedKey="antigravityBlocked"
      renderConnectHint={(status) =>
        status.settingsPath && status.hooksPath ? (
          <p className="text-xs text-slate-500 dark:text-slate-400 leading-relaxed break-all">
            {t('antigravityConnectHint', { settingsPath: status.settingsPath, hooksPath: status.hooksPath })}
          </p>
        ) : null
      }
      renderLiveFailure={(status, formatReason) =>
        live
          ? !status.cliAvailable
            ? t('antigravityCliMissing')
            : status.readFailure && status.readFailure !== 'notConnected'
              ? status.readFailure === 'authentication'
                ? t('antigravityLiveSignIn')
                : status.readFailure === 'missingExecutable'
                  ? t('antigravityCliMissing')
                  : t('antigravityLiveFailed', { reason: formatReason(status.readFailure) })
              : null
          : null
      }
      renderSnapshotText={(status, formatTime) =>
        status.snapshotCapturedAt
          ? status.antigravityVersion
            ? t('antigravitySnapshotVersion', { version: status.antigravityVersion, time: formatTime(status.snapshotCapturedAt) })
            : t('antigravitySnapshot', { time: formatTime(status.snapshotCapturedAt) })
          : t('antigravityNoSnapshot')
      }
      noSnapshotKey="antigravityNoSnapshot"
    >
      {(status) => {
        const modelDisplay = status?.activeModelName ?? status?.activeModelId;
        return (
          <>
            <div className="flex items-center justify-between gap-2 pt-0.5">
              <label htmlFor="antigravity-pool-select" className="text-sm font-medium">{t('antigravityPoolLabel')}</label>
              <select
                id="antigravity-pool-select"
                aria-label={t('antigravityPoolLabel')}
                value={pool}
                onChange={(e) => onPatchSettings({ antigravityPool: e.target.value as AntigravityPool })}
                className="bg-white dark:bg-slate-700 border border-slate-300 dark:border-slate-600 text-xs rounded-md px-2 py-1 focus:outline-none focus:ring-1 focus:ring-cyan-500"
              >
                {POOLS.map((item) => (
                  <option key={item} value={item}>{t(`antigravityPool_${item}`)}</option>
                ))}
              </select>
            </div>
            {modelDisplay && status?.effectivePool && (
              <p className="text-xs text-slate-500 dark:text-slate-400">
                {t('antigravityDetectedModel', {
                  model: modelDisplay,
                  pool: t(`antigravityPool_${status.effectivePool}`),
                })}
              </p>
            )}
          </>
        );
      }}
    </BridgeConnectionCard>
  );
};
