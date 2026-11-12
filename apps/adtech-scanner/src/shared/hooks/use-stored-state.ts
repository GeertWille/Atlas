import { useState } from "react";

function read<T>(key: string, fallback: T, isValid: (value: unknown) => value is T): T {
  try {
    const raw = localStorage.getItem(key);
    if (raw === null) return fallback;
    const parsed: unknown = JSON.parse(raw);
    return isValid(parsed) ? parsed : fallback;
  } catch {
    return fallback;
  }
}

/**
 * State that outlives the window: a layout choice such as a panel's size or which columns are shown.
 * A stored value of the wrong shape (an older version's, or a hand-edited one) is ignored.
 */
export function useStoredState<T>(key: string, fallback: T, isValid: (value: unknown) => value is T) {
  const [value, setValue] = useState(() => read(key, fallback, isValid));
  const store = (next: T) => {
    setValue(next);
    try {
      localStorage.setItem(key, JSON.stringify(next));
    } catch {
      // Storage full or unavailable: the choice then only lasts for this window.
    }
  };
  return [value, store] as const;
}

export const isBoolean = (value: unknown): value is boolean => typeof value === "boolean";
export const isFiniteNumber = (value: unknown): value is number => typeof value === "number" && Number.isFinite(value);
