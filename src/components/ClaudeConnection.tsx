import React from 'react';
import { useTranslation } from 'react-i18next';
import { ClaudeConnectionStatus, OverlaySettings } from '../types';
import { BridgeConnectionCard } from './BridgeConnectionCard';

interface ClaudeConnectionViewProps {
  settings: OverlaySettings;
  onPatchSettings: (patch: Partial<OverlaySettings>, debounce?: boolean) => void;
}

/**
 * Connects the bundled `claude-usage-bridge` helper to Claude Code (statusline +
 * guard hooks) and shows what it last reported.
 */
export const ClaudeConnectionView: React.FC<ClaudeConnectionViewProps> = ({ settings, onPatchSettings }) => {
  const { t } = useTranslation();
  const live = settings.claudeLivePolling;

  return (
    <BridgeConnectionCard<ClaudeConnectionStatus>
      settings={settings}
      provider="claude"
      statusCommand="claude_connection_status"
      connectCommand="claude_connect"
      disconnectCommand="claude_disconnect"
      liveChecked={live}
      onToggleLive={(checked) => onPatchSettings({ claudeLivePolling: checked })}
      titleKey="claudeConnection"
      connectedKey="claudeConnected"
      partlyConnectedKey="claudePartlyConnected"
      notConnectedKey="claudeNotConnected"
      bridgeMissingKey="claudeBridgeMissing"
      workingKey="claudeWorking"
      connectKey="claudeConnect"
      disconnectKey="claudeDisconnect"
      livePollingKey="claudeLivePolling"
      livePollingHintKey="claudeLivePollingHint"
      protectionSummariesPrefix="claudeProtectionSummaries"
      blockedKey="claudeBlocked"
      renderConnectHint={(status) =>
        status.settingsPath ? (
          <p className="text-xs text-slate-500 dark:text-slate-400 leading-relaxed break-all">
            {t('claudeConnectHint', { path: status.settingsPath })}
          </p>
        ) : null
      }
      renderLiveFailure={(status, formatReason) =>
        live && status.readFailure && status.readFailure !== 'notConnected'
          ? status.readFailure === 'authentication'
            ? t('claudeLiveSignIn')
            : status.readHttpStatus
              ? t('claudeLiveHttp', { status: status.readHttpStatus })
              : t('claudeLiveFailed', { reason: formatReason(status.readFailure) })
          : null
      }
      renderSnapshotText={(status, formatTime) =>
        status.snapshotCapturedAt
          ? status.claudeVersion
            ? t('claudeSnapshotVersion', { version: status.claudeVersion, time: formatTime(status.snapshotCapturedAt) })
            : t('claudeSnapshot', { time: formatTime(status.snapshotCapturedAt) })
          : t('claudeNoSnapshot')
      }
      noSnapshotKey="claudeNoSnapshot"
    />
  );
};
