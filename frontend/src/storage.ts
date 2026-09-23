// Per-browser conveniences (which roster and base were picked last). Storage
// can be unavailable or cleared, so every access is guarded and the app
// works without it.

const PREFIX = "riic-planner:";

export function recall(key: string): string | null {
  try {
    return window.localStorage.getItem(PREFIX + key);
  } catch {
    return null;
  }
}

export function remember(key: string, value: string | null): void {
  try {
    if (value === null) window.localStorage.removeItem(PREFIX + key);
    else window.localStorage.setItem(PREFIX + key, value);
  } catch {
    // Not remembered; nothing depends on it.
  }
}
