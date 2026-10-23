import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { ProtectionStatus } from '../types';

export function useProtectionStatus() {
  const [status, setStatus] = useState<ProtectionStatus | null>(null);
  const [unavailable, setUnavailable] = useState(false);
  const revision = useRef(-1);
  useEffect(() => {
    let active = true;
    let subscribed = false;
    let unlisten: (() => void) | undefined;
    const apply = (next: ProtectionStatus | null) => {
      if (!active || !next || !Number.isSafeInteger(next.revision) || next.revision < revision.current) return;
      revision.current = next.revision;
      setStatus(next);
      setUnavailable(!subscribed);
    };
    const hydrate = async () => {
      const before = revision.current;
      try { apply(await invoke<ProtectionStatus>('get_auto_pause_status')); }
      catch { if (active && before === revision.current) setUnavailable(true); }
    };
    // Subscribe first. Revision ordering also covers live events during hydration.
    void listen<ProtectionStatus>('auto_pause_status_changed', event => apply(event.payload))
      .then(fn => {
        if (!active) { fn(); return; }
        unlisten = fn;
        subscribed = true;
        void hydrate();
      })
      .catch(() => { if (active) { setUnavailable(true); void hydrate(); } });
    const onVisible = () => { if (document.visibilityState === 'visible') void hydrate(); };
    window.addEventListener('focus', hydrate);
    document.addEventListener('visibilitychange', onVisible);
    return () => {
      active = false;
      unlisten?.();
      window.removeEventListener('focus', hydrate);
      document.removeEventListener('visibilitychange', onVisible);
    };
  }, []);
  return { status, unavailable };
}
