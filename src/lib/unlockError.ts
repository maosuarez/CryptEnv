/** Classification of unlock failures returned by `vault_unlock` / `biometric_unlock`.
 *  The backend sends stable prefixes (`UnlockError::gui_error`): `unlock_throttled:<secs>`
 *  and `unlock_aborted`; anything else is a wrong password or a plain message. */
export type UnlockFailure =
  | { kind: 'throttled'; retryAfterSecs: number }
  | { kind: 'aborted' }
  | { kind: 'other'; message: string };

const THROTTLED_PREFIX = 'unlock_throttled:';
const ABORTED = 'unlock_aborted';

export function classifyUnlockError(e: unknown): UnlockFailure {
  const message = e instanceof Error ? e.message : String(e);
  if (message.startsWith(THROTTLED_PREFIX)) {
    const secs = Number.parseInt(message.slice(THROTTLED_PREFIX.length), 10);
    return { kind: 'throttled', retryAfterSecs: Number.isFinite(secs) && secs > 0 ? secs : 1 };
  }
  if (message === ABORTED) return { kind: 'aborted' };
  return { kind: 'other', message };
}

/** Seconds left after one tick of a throttle countdown, never below 0. */
export function tickCountdown(secs: number): number {
  return Math.max(0, secs - 1);
}
