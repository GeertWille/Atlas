export type FakeProfile = "seeded" | "fresh" | "no-browser";

export type FakeConfig = {
  /** Which starting state the fake backend boots with. */
  profile: FakeProfile;
  /** Simulated time runs this many times faster than a real scan. */
  speed: number;
  /** Jev System One fails part-way through script triage, so scans show the fallback. */
  jevFails?: boolean;
  /** Every scan's request list is padded to at least this many rows, to try the large-list cases. */
  rows?: number;
  /** Calls answer at once instead of after a simulated delay: what is timed is then the interface alone. */
  instant?: boolean;
  /** A script source larger than this many bytes is "too large to show", instead of the backend's 8 MB. */
  sourceLimitBytes?: number;
};

const PROFILE_KEY = "adscan.fake.profile";
const SPEED_KEY = "adscan.fake.speed";
const JEV_KEY = "adscan.fake.jev";
const ROWS_KEY = "adscan.fake.rows";
const SOURCE_LIMIT_KEY = "adscan.fake.source-limit";
const LATENCY_KEY = "adscan.fake.latency";
const DEFAULT_SPEED = 3;

function isProfile(value: string | null): value is FakeProfile {
  return value === "seeded" || value === "fresh" || value === "no-browser";
}

/**
 * `?fakeProfile=fresh&fakeSpeed=40&fakeJev=fail&fakeRows=3000&fakeSourceLimit=100000&fakeLatency=off`
 * configures the fake backend; the values are remembered in localStorage so they survive reloads and hash
 * navigation. `fakeJev=ok` ends the outage, `fakeRows=0` the padding, `fakeSourceLimit=0` the lowered limit
 * and `fakeLatency=on` brings the simulated delays back.
 */
export function readFakeConfig(): FakeConfig {
  const params = new URLSearchParams(window.location.search);
  const profileParam = params.get("fakeProfile");
  const speedParam = params.get("fakeSpeed");
  const jevParam = params.get("fakeJev");
  const rowsParam = params.get("fakeRows");
  const sourceLimitParam = params.get("fakeSourceLimit");
  const latencyParam = params.get("fakeLatency");
  if (isProfile(profileParam)) localStorage.setItem(PROFILE_KEY, profileParam);
  if (speedParam && Number(speedParam) > 0) localStorage.setItem(SPEED_KEY, speedParam);
  if (jevParam === "fail" || jevParam === "ok") localStorage.setItem(JEV_KEY, jevParam);
  if (rowsParam !== null && Number(rowsParam) >= 0) localStorage.setItem(ROWS_KEY, rowsParam);
  if (sourceLimitParam !== null && Number(sourceLimitParam) >= 0) {
    localStorage.setItem(SOURCE_LIMIT_KEY, sourceLimitParam);
  }
  if (latencyParam === "off" || latencyParam === "on") localStorage.setItem(LATENCY_KEY, latencyParam);
  if ([profileParam, speedParam, jevParam, rowsParam, sourceLimitParam, latencyParam].some((param) => param !== null)) {
    // One-shot switches: once stored they have no business staying in the address bar.
    params.delete("fakeProfile");
    params.delete("fakeSpeed");
    params.delete("fakeJev");
    params.delete("fakeRows");
    params.delete("fakeSourceLimit");
    params.delete("fakeLatency");
    const query = params.size > 0 ? `?${params}` : "";
    window.history.replaceState(null, "", `${window.location.pathname}${query}${window.location.hash}`);
  }

  const storedProfile = localStorage.getItem(PROFILE_KEY);
  const storedSpeed = Number(localStorage.getItem(SPEED_KEY));
  return {
    profile: isProfile(storedProfile) ? storedProfile : "seeded",
    speed: storedSpeed > 0 ? storedSpeed : DEFAULT_SPEED,
    jevFails: localStorage.getItem(JEV_KEY) === "fail",
    rows: Number(localStorage.getItem(ROWS_KEY)) || 0,
    sourceLimitBytes: Number(localStorage.getItem(SOURCE_LIMIT_KEY)) || undefined,
    instant: localStorage.getItem(LATENCY_KEY) === "off",
  };
}
