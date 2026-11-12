export type FakeProfile = "seeded" | "fresh" | "no-browser";

export type FakeConfig = {
  /** Which starting state the fake backend boots with. */
  profile: FakeProfile;
  /** Simulated time runs this many times faster than a real scan. */
  speed: number;
};

const PROFILE_KEY = "adscan.fake.profile";
const SPEED_KEY = "adscan.fake.speed";
const DEFAULT_SPEED = 3;

function isProfile(value: string | null): value is FakeProfile {
  return value === "seeded" || value === "fresh" || value === "no-browser";
}

/**
 * `?fakeProfile=fresh&fakeSpeed=40` configures the fake backend; both values are
 * remembered in localStorage so they survive reloads and hash navigation.
 */
export function readFakeConfig(): FakeConfig {
  const params = new URLSearchParams(window.location.search);
  const profileParam = params.get("fakeProfile");
  const speedParam = params.get("fakeSpeed");
  if (isProfile(profileParam)) localStorage.setItem(PROFILE_KEY, profileParam);
  if (speedParam && Number(speedParam) > 0) localStorage.setItem(SPEED_KEY, speedParam);
  if (profileParam !== null || speedParam !== null) {
    // One-shot switches: once stored they have no business staying in the address bar.
    params.delete("fakeProfile");
    params.delete("fakeSpeed");
    const query = params.size > 0 ? `?${params}` : "";
    window.history.replaceState(null, "", `${window.location.pathname}${query}${window.location.hash}`);
  }

  const storedProfile = localStorage.getItem(PROFILE_KEY);
  const storedSpeed = Number(localStorage.getItem(SPEED_KEY));
  return {
    profile: isProfile(storedProfile) ? storedProfile : "seeded",
    speed: storedSpeed > 0 ? storedSpeed : DEFAULT_SPEED,
  };
}
