import { useEffect, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useVaultStore } from '../store';

const ACTIVITY_EVENTS = ['mousemove', 'keydown', 'mousedown', 'touchstart'] as const;

/** Minimum gap between `vault_touch` IPC calls — well under the 30 s
 *  granularity of the backend auto-lock loop, so activity is never missed. */
export const TOUCH_THROTTLE_MS = 15_000;

export function useAutoLock() {
  const screen      = useVaultStore((s) => s.screen);
  const lockTimeout = useVaultStore((s) => s.lockTimeout);
  const lock        = useVaultStore((s) => s.lock);

  const timerRef     = useRef<ReturnType<typeof setTimeout> | null>(null);
  const lastTouchRef = useRef(0);

  useEffect(() => {
    if (screen === 'lock' || lockTimeout === 0) {
      if (timerRef.current) clearTimeout(timerRef.current);
      return;
    }

    const ms = lockTimeout * 60 * 1000;

    const reset = () => {
      if (timerRef.current) clearTimeout(timerRef.current);
      timerRef.current = setTimeout(() => lock(), ms);
      // Keep the backend idle timer in step with real user activity so it
      // doesn't auto-lock underneath an active session.
      const now = Date.now();
      if (now - lastTouchRef.current >= TOUCH_THROTTLE_MS) {
        lastTouchRef.current = now;
        invoke('vault_touch').catch(() => {});
      }
    };

    ACTIVITY_EVENTS.forEach((e) => window.addEventListener(e, reset, { passive: true }));
    reset();

    return () => {
      ACTIVITY_EVENTS.forEach((e) => window.removeEventListener(e, reset));
      if (timerRef.current) clearTimeout(timerRef.current);
    };
  }, [screen, lockTimeout, lock]);
}
