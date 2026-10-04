import { describe, expect, it } from 'vitest';
import { classifyUnlockError, tickCountdown } from './unlockError';

describe('classifyUnlockError', () => {
  it('parses the throttle wait from a string error', () => {
    expect(classifyUnlockError('unlock_throttled:16')).toEqual({ kind: 'throttled', retryAfterSecs: 16 });
  });
  it('clamps a malformed throttle wait to 1 s', () => {
    expect(classifyUnlockError('unlock_throttled:abc')).toEqual({ kind: 'throttled', retryAfterSecs: 1 });
    expect(classifyUnlockError('unlock_throttled:0')).toEqual({ kind: 'throttled', retryAfterSecs: 1 });
  });
  it('recognizes aborted', () => {
    expect(classifyUnlockError('unlock_aborted')).toEqual({ kind: 'aborted' });
  });
  it('treats a wrong password as other and unwraps Error objects', () => {
    expect(classifyUnlockError('incorrect password')).toEqual({ kind: 'other', message: 'incorrect password' });
    expect(classifyUnlockError(new Error('unlock_aborted'))).toEqual({ kind: 'aborted' });
  });
});

describe('tickCountdown', () => {
  it('counts down to 0 and stops', () => {
    expect(tickCountdown(2)).toBe(1);
    expect(tickCountdown(1)).toBe(0);
    expect(tickCountdown(0)).toBe(0);
  });
});
