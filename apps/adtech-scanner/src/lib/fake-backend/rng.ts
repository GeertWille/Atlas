export type Rng = {
  next(): number;
  between(min: number, max: number): number;
  int(min: number, max: number): number;
  chance(probability: number): boolean;
  pick<T>(items: readonly T[]): T;
};

export function hashString(input: string): number {
  let hash = 2166136261;
  for (let index = 0; index < input.length; index += 1) {
    hash ^= input.charCodeAt(index);
    hash = Math.imul(hash, 16777619);
  }
  return hash >>> 0;
}

/** mulberry32: small deterministic PRNG so a given site always scans the same way. */
export function createRng(seed: number): Rng {
  let state = seed >>> 0;
  const next = () => {
    state = (state + 0x6d2b79f5) >>> 0;
    let t = state;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
  const between = (min: number, max: number) => min + next() * (max - min);
  return {
    next,
    between,
    int: (min, max) => Math.floor(between(min, max + 1)),
    chance: (probability) => next() < probability,
    pick: (items) => items[Math.floor(next() * items.length)] as (typeof items)[number],
  };
}

export function hexId(rng: Rng, length: number): string {
  let out = "";
  while (out.length < length) out += Math.floor(rng.next() * 16).toString(16);
  return out;
}
