export type FakeProfile = "seeded" | "fresh" | "no-browser";

export type FakeConfig = {
  /** Which starting state the fake backend boots with. */
  profile: FakeProfile;
  /** Simulated time runs this many times faster than a real scan. */
  speed: number;
  /** Jev System One fails part-way through script triage, so scans show the fallback. */
  jevFails?: boolean;
};

const PROFILE_KEY = "adscan.fake.profile";
const SPEED_KEY = "adscan.fake.speed";
const JEV_KEY = "adscan.fake.jev";
const DEFAULT_SPEED = 3;

function isProfile(value: string | null): value is FakeProfile {
  return value === "seeded" || value === "fresh" || value === "no-browser";
}

/**
 * `?fakeProfile=fresh&fakeSpeed=40&fakeJev=fail` configures the fake backend; the values are
 * remembered in localStorage so they survive reloads and hash navigation. `fakeJev=ok` ends the outage.
 */
export function readFakeConfig(): FakeConfig {
  const params = new URLSearchParams(window.location.search);
  const profileParam = params.get("fakeProfile");
  const speedParam = params.get("fakeSpeed");
  const jevParam = params.get("fakeJev");
  if (isProfile(profileParam)) localStorage.setItem(PROFILE_KEY, profileParam);
  if (speedParam && Number(speedParam) > 0) localStorage.setItem(SPEED_KEY, speedParam);
  if (jevParam === "fail" || jevParam === "ok") localStorage.setItem(JEV_KEY, jevParam);
  if (profileParam !== null || speedParam !== null || jevParam !== null) {
    // One-shot switches: once stored they have no business staying in the address bar.
    params.delete("fakeProfile");
    params.delete("fakeSpeed");
    params.delete("fakeJev");
    const query = params.size > 0 ? `?${params}` : "";
    window.history.replaceState(null, "", `${window.location.pathname}${query}${window.location.hash}`);
  }

  const storedProfile = localStorage.getItem(PROFILE_KEY);
  const storedSpeed = Number(localStorage.getItem(SPEED_KEY));
  return {
    profile: isProfile(storedProfile) ? storedProfile : "seeded",
    speed: storedSpeed > 0 ? storedSpeed : DEFAULT_SPEED,
    jevFails: localStorage.getItem(JEV_KEY) === "fail",
  };
}
