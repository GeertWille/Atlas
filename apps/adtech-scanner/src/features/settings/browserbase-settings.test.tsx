import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, test, vi } from "vitest";
import { api } from "@/lib/api";
import { useBrowser, useSettings } from "@/lib/query/hooks";
import { BROWSERBASE_DISCLOSURE } from "@/shared/domain/browserbase";
import { renderWithApp } from "@/test/render";
import { SettingsScreen } from "./SettingsScreen";

// Runs against the real api module: outside Tauri that is the fake backend.
vi.hoisted(() => {
  localStorage.setItem("adscan.fake.profile", "fresh");
  localStorage.setItem("adscan.fake.speed", "5000");
});

function ConnectedSettings() {
  const settings = useSettings();
  const browser = useBrowser();
  return settings.data ? <SettingsScreen settings={settings.data} browser={browser.data} /> : null;
}

const KEY = "bb_live_0123456789abcdef";
const PAGE = {
  url: "https://www.example.com/",
  device: "desktop",
  observeSeconds: 5,
  autoScroll: true,
  acceptConsent: true,
  aiEnabled: false,
  browser: { kind: "browserbase" },
} as const;

beforeEach(async () => {
  await api.deleteAllScans();
  await api.clearBrowserbaseCredentials();
  for (const proxy of (await api.getSettings()).proxies) await api.deleteProxyProfile(proxy.id);
  await api.updateSettings({
    browserbase: { region: "us-west-2", defaultProxy: { kind: "none" }, persistentProfile: false },
  });
});

const section = async () => within(await screen.findByRole("region", { name: "Browserbase" }));
const defaults = async () => within(await screen.findByRole("region", { name: "Scan defaults" }));
const keyInput = async () => (await section()).findByLabelText("Browserbase API key");

test("before a key is saved: the disclosure, the key field beside its save button, and nothing else", async () => {
  await renderWithApp(<ConnectedSettings />);
  const browserbase = await section();
  expect(browserbase.getByRole("note", { name: "What using Browserbase means" })).toHaveTextContent(
    BROWSERBASE_DISCLOSURE,
  );
  expect(await keyInput()).toHaveAttribute("type", "password");
  expect(browserbase.getByRole("button", { name: "Save Browserbase key" })).toBeEnabled();
  // No ritual in between: no checkbox to tick first.
  expect(browserbase.queryByRole("checkbox")).toBeNull();
  for (const name of ["Test connection", "Check what a page sees"]) {
    expect(browserbase.queryByRole("button", { name })).toBeNull();
  }
  expect(browserbase.queryByRole("combobox", { name: "Default region" })).toBeNull();

  // Until then scans run on this Mac, and the default cannot say otherwise.
  const scanDefaults = await defaults();
  expect(scanDefaults.getByRole("radio", { name: "This Mac" })).toBeChecked();
  expect(scanDefaults.getByRole("radio", { name: "Browserbase" })).toBeDisabled();
  expect(scanDefaults.getByText(/Set up Browserbase above to run scans in its cloud\./)).toBeInTheDocument();
});

test("a refused key says why and stores nothing; an accepted one is adopted with its project, by name", async () => {
  const user = userEvent.setup();
  await renderWithApp(<ConnectedSettings />);
  const browserbase = await section();

  await user.click(browserbase.getByRole("button", { name: "Save Browserbase key" }));
  expect(await browserbase.findByRole("alert")).toHaveTextContent("Paste your API key.");

  await user.type(await keyInput(), "bb_live_invalid");
  await user.click(browserbase.getByRole("button", { name: "Save Browserbase key" }));
  expect(await browserbase.findByRole("alert")).toHaveTextContent("Browserbase did not accept the API key.");
  expect(await keyInput()).toBeInvalid();
  expect((await api.getSettings()).browserbase.connected).toBe(false);

  await user.clear(await keyInput());
  await user.type(await keyInput(), KEY);
  await user.click(browserbase.getByRole("button", { name: "Save Browserbase key" }));
  expect(
    await browserbase.findByText("Connected to the project “Local”. The key is in your system keychain."),
  ).toBeInTheDocument();
  expect(browserbase.queryByLabelText("Browserbase API key")).toBeNull();
  expect(await screen.findByText("Browserbase key validated and saved to your keychain")).toBeInTheDocument();
  // The disclosure stays in view once connected.
  expect(browserbase.getByRole("note", { name: "What using Browserbase means" })).toBeInTheDocument();
  // Saving the key does not move scans to Browserbase by itself.
  expect(await api.getSettings()).toMatchObject({ defaultBrowser: "local", browserbase: { connected: true } });
});

test("a key with several projects asks which one to use", async () => {
  const user = userEvent.setup();
  await renderWithApp(<ConnectedSettings />);
  const browserbase = await section();
  await user.type(await keyInput(), "bb_live_multi_0123456789");
  await user.click(browserbase.getByRole("button", { name: "Save Browserbase key" }));
  expect(await browserbase.findByRole("alert")).toHaveTextContent(
    "This key has more than one Browserbase project. Choose the one to use.",
  );
  // The key stays in the field while the project is chosen.
  expect(await keyInput()).toHaveValue("bb_live_multi_0123456789");
  await user.click(browserbase.getByRole("combobox", { name: "Project" }));
  expect((await screen.findAllByRole("option")).map((option) => option.textContent)).toEqual(["Local", "Staging"]);
  await user.click(screen.getByRole("option", { name: "Staging" }));
  await user.click(browserbase.getByRole("button", { name: "Save Browserbase key" }));
  expect(await browserbase.findByText(/Connected to the project “Staging”\./)).toBeInTheDocument();
});

test("a project chosen for one key is not sent with another key", async () => {
  const user = userEvent.setup();
  await renderWithApp(<ConnectedSettings />);
  const browserbase = await section();
  await user.type(await keyInput(), "bb_live_multi_0123456789");
  await user.click(browserbase.getByRole("button", { name: "Save Browserbase key" }));
  await user.click(await browserbase.findByRole("combobox", { name: "Project" }));
  await user.click(await screen.findByRole("option", { name: "Staging" }));

  // Another key, with one project of its own: "Staging" is not among them.
  await user.clear(await keyInput());
  await user.type(await keyInput(), KEY);
  await user.click(browserbase.getByRole("button", { name: "Save Browserbase key" }));
  expect(await browserbase.findByRole("alert")).toHaveTextContent(
    "This Browserbase project was not found for this key.",
  );
  // The choice that belonged to the other key is gone with its list, so the next save is not stuck on it.
  expect(browserbase.queryByRole("combobox", { name: "Project" })).toBeNull();
  await user.click(browserbase.getByRole("button", { name: "Save Browserbase key" }));
  expect(await browserbase.findByText(/Connected to the project “Local”\./)).toBeInTheDocument();
});

test("test connection is free and shows the project, its limit and what is running", async () => {
  const user = userEvent.setup();
  await api.setBrowserbaseCredentials(KEY);
  await renderWithApp(<ConnectedSettings />);
  const browserbase = await section();
  expect(browserbase.getByText(/^Free: asks Browserbase for the project/)).toHaveTextContent(
    "Another application that uses the same project shares that limit.",
  );
  const probe = vi.spyOn(api, "probeBrowserbase");
  await user.click(await browserbase.findByRole("button", { name: "Test connection" }));
  expect(await browserbase.findByText(/Project “Local”/)).toBeInTheDocument();
  expect(browserbase.getByText(/up to 3 browsers at once, 1 running now\./)).toHaveTextContent(
    "Used so far by everything in the project: 483 browser minutes and 2.00 GB of proxy traffic.",
  );
  // It starts no browser.
  expect(probe).not.toHaveBeenCalled();
  probe.mockRestore();
});

test("the page check states its cost on the button's description and shows what a page sees", async () => {
  const user = userEvent.setup();
  await api.setBrowserbaseCredentials(KEY);
  await api.updateSettings({ browserbase: { region: "eu-central-1" } });
  await renderWithApp(<ConnectedSettings />);
  const browserbase = await section();
  const button = await browserbase.findByRole("button", { name: "Check what a page sees" });
  expect(button).toHaveAccessibleDescription(
    "Starts a Browserbase browser for under a minute in Frankfurt (eu-central-1), the default region, and reads what a blank page can tell there. Browserbase bills it as one browser minute, about $0.002 at its list price.",
  );
  const probe = vi.spyOn(api, "probeBrowserbase");
  await user.click(button);
  expect(await browserbase.findByRole("button", { name: "Starting a browser…" })).toBeDisabled();
  const facts = within(
    await browserbase.findByRole("group", { name: "What a page sees" }, { timeout: 4_000 }),
  ).getAllByRole("definition");
  expect(probe).toHaveBeenCalledWith("eu-central-1");
  expect(facts.map((fact) => fact.textContent)).toEqual([
    "Frankfurt, Germany (eu-central-1)",
    "Chrome/157.0.8103.79",
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/157.0.0.0 Safari/537.36",
    "false: the page is not told the visit is automated",
    "America/Los_Angeles; a scan in this region sets Europe/Berlin unless it uses a Browserbase location",
    "en-US",
    "119 ms per command",
    "6 s, billed as one minute",
  ]);
  probe.mockRestore();
});

test("a check that fails shows the backend's message", async () => {
  const user = userEvent.setup();
  await api.setBrowserbaseCredentials(KEY);
  await renderWithApp(<ConnectedSettings />);
  const browserbase = await section();
  const refused = {
    code: "browserbaseLimit",
    message: "Browserbase is at its limit of new browsers per minute. Try again in a minute.",
  };
  const test = vi.spyOn(api, "testBrowserbase").mockRejectedValue(refused);
  const probe = vi.spyOn(api, "probeBrowserbase").mockRejectedValue(refused);
  await user.click(await browserbase.findByRole("button", { name: "Test connection" }));
  await user.click(browserbase.getByRole("button", { name: "Check what a page sees" }));
  await waitFor(() =>
    expect(browserbase.getAllByRole("alert").map((alert) => alert.textContent)).toEqual([
      refused.message,
      refused.message,
    ]),
  );
  test.mockRestore();
  probe.mockRestore();
});

test("the default region says what a region decides, and is saved", async () => {
  const user = userEvent.setup();
  await api.setBrowserbaseCredentials(KEY);
  await renderWithApp(<ConnectedSettings />);
  const browserbase = await section();
  expect(
    browserbase.getByText(
      "Where Browserbase starts the browser. The region decides both speed (one near you answers faster) and, without a proxy, where the visit comes from.",
    ),
  ).toBeInTheDocument();
  await user.click(await browserbase.findByRole("combobox", { name: "Default region" }));
  expect((await screen.findAllByRole("option")).map((option) => option.textContent)).toEqual([
    "Oregon, United States (us-west-2)",
    "Virginia, United States (us-east-1)",
    "Frankfurt, Germany (eu-central-1)",
    "Singapore (ap-southeast-1)",
  ]);
  await user.click(screen.getByRole("option", { name: /Frankfurt/ }));
  await waitFor(async () => expect((await api.getSettings()).browserbase.region).toBe("eu-central-1"));
});

test("the default location: the region, one of your proxies, or a Browserbase location by country", async () => {
  const user = userEvent.setup();
  await api.setBrowserbaseCredentials(KEY);
  await api.saveProxyProfile({ name: "Belgium", scheme: "http", host: "be.proxy.example", port: 8080, username: null });
  const [belgium] = (await api.getSettings()).proxies;
  await renderWithApp(<ConnectedSettings />);
  const browserbase = await section();
  const location = await browserbase.findByRole("combobox", { name: "Default location" });
  expect(location).toHaveTextContent("Region only: leaves from Oregon (us-west-2)");
  // The price is Browserbase's list price; no plan is named.
  expect(browserbase.getByText(/^Where a Browserbase scan leaves from/)).toHaveTextContent(
    "billed per megabyte: $12 per GB at Browserbase's list price.",
  );
  expect(browserbase.queryByText(/Developer|Startup|overage/)).toBeNull();

  await user.click(location);
  expect((await screen.findAllByRole("option")).map((option) => option.textContent)).toEqual([
    "Region only: leaves from Oregon (us-west-2)",
    "Belgium",
    "Browserbase location…",
  ]);
  await user.click(screen.getByRole("option", { name: "Belgium" }));
  await waitFor(async () =>
    expect((await api.getSettings()).browserbase.defaultProxy).toMatchObject({ kind: "profile", id: belgium?.id }),
  );

  // A location is saved once it has a country.
  await user.click(location);
  await user.click(await screen.findByRole("option", { name: "Browserbase location…" }));
  const save = browserbase.getByRole("button", { name: "Save location" });
  // Choosing it stores nothing yet, and the form says so until it is saved.
  expect(browserbase.getByRole("status")).toHaveTextContent("Not saved yet");
  await user.click(save);
  const alert = await browserbase.findByRole("alert");
  expect(alert).toHaveTextContent("Enter the country as a two-letter code, for example BE.");
  expect(browserbase.getByLabelText("Default country code")).toBeInvalid();
  expect(browserbase.getByLabelText("Default country code")).toHaveAttribute("aria-describedby", alert.id);
  expect((await api.getSettings()).browserbase.defaultProxy.kind).toBe("profile");
  await user.type(browserbase.getByLabelText("Default country code"), "us");
  // The field that is wrong is the one marked, not always the first.
  await user.type(browserbase.getByLabelText("Default state code, optional"), "n");
  await user.click(save);
  expect(await browserbase.findByRole("alert")).toHaveTextContent(
    "Enter the state as a two-letter code, for example NY.",
  );
  expect(browserbase.getByLabelText("Default state code, optional")).toBeInvalid();
  expect(browserbase.getByLabelText("Default country code")).toBeValid();
  await user.type(browserbase.getByLabelText("Default state code, optional"), "y");
  await user.type(browserbase.getByLabelText("Default city, optional"), "new york");
  await user.click(save);
  await waitFor(async () =>
    expect((await api.getSettings()).browserbase.defaultProxy).toEqual({
      kind: "browserbaseLocation",
      country: "US",
      state: "NY",
      city: "NEW_YORK",
    }),
  );
  await waitFor(() => expect(location).toHaveTextContent("Browserbase location: New York, NY, United States"));
  // What is typed is what is stored: there is nothing left to save until it is edited again.
  expect(save).toBeDisabled();
  expect(browserbase.queryByRole("status")).toBeNull();
  await user.type(browserbase.getByLabelText("Default city, optional"), " city");
  expect(save).toBeEnabled();
  expect(browserbase.getByRole("status")).toHaveTextContent("Not saved yet");
  expect(location).toHaveTextContent("Browserbase location…");

  await user.click(location);
  await user.click(await screen.findByRole("option", { name: /^Region only/ }));
  await waitFor(async () => expect((await api.getSettings()).browserbase.defaultProxy).toEqual({ kind: "none" }));
  expect(browserbase.queryByLabelText("Default country code")).toBeNull();
});

test("keeping a profile on Browserbase is off by default, says what it stores, and can be cleared", async () => {
  const user = userEvent.setup();
  await api.setBrowserbaseCredentials(KEY);
  // The switch for this Mac's browser plays no part.
  await api.updateSettings({ persistentProfile: true });
  await renderWithApp(<ConnectedSettings />);
  const browserbase = await section();
  const keep = await browserbase.findByRole("switch", { name: "Keep a browser profile on Browserbase" });
  expect(keep).not.toBeChecked();
  expect(keep).toHaveAccessibleDescription(
    "On: Browserbase scans share one profile per device type that Browserbase stores: cookies, consent choices and site storage, until you clear it here. Off: every Browserbase scan starts fresh, whatever the setting for this Mac's browser says.",
  );
  expect(await browserbase.findByText("No browser profile is kept at Browserbase.")).toBeInTheDocument();

  await user.click(keep);
  await waitFor(async () => expect((await api.getSettings()).browserbase.persistentProfile).toBe(true));
  const { finished } = await scanFinishes();
  await api.startScan(PAGE);
  await finished;
  await api.updateSettings({ persistentProfile: false });
  const view = await renderWithApp(<ConnectedSettings />);
  const again = within(await within(view.container).findByRole("region", { name: "Browserbase" }));
  expect(await again.findByText(/^A browser profile is kept at Browserbase, last used /)).toBeInTheDocument();

  await user.click(again.getByRole("button", { name: "Clear Browserbase profile…" }));
  const dialog = await screen.findByRole("alertdialog", { name: "Clear the browser profile at Browserbase?" });
  expect(dialog).toHaveTextContent("The profile on this Mac is not touched.");
  await user.click(within(dialog).getByRole("button", { name: "Clear profile" }));
  expect(await again.findByText("No browser profile is kept at Browserbase.")).toBeInTheDocument();
  expect((await api.getBrowserbaseProfile()).exists).toBe(false);
});

async function scanFinishes() {
  let finish: () => void = () => {};
  const finished = new Promise<void>((resolve) => {
    finish = resolve;
  });
  await api.subscribeScanEvents((event) => {
    if (event.type === "scanUpdated" && event.summary.status !== "running") finish();
  });
  return { finished };
}

test("the default for where the browser runs can be Browserbase once it is set up, and removing the key undoes it", async () => {
  const user = userEvent.setup();
  await api.setBrowserbaseCredentials(KEY);
  await renderWithApp(<ConnectedSettings />);
  const scanDefaults = await defaults();
  const cloud = await scanDefaults.findByRole("radio", { name: "Browserbase" });
  await waitFor(() => expect(cloud).toBeEnabled());
  expect(
    scanDefaults.getByText(
      /An address on this Mac or your network, like the demo site, is always scanned on this Mac\./,
    ),
  ).toBeInTheDocument();
  await user.click(cloud);
  await waitFor(async () => expect((await api.getSettings()).defaultBrowser).toBe("browserbase"));

  const browserbase = await section();
  await user.click(browserbase.getByRole("button", { name: "Remove Browserbase key" }));
  const dialog = await screen.findByRole("alertdialog", { name: "Remove the Browserbase key?" });
  expect(dialog).toHaveTextContent(
    "Scans run on this Mac again, and saved configurations that use Browserbase cannot run until a key is added.",
  );
  await user.click(within(dialog).getByRole("button", { name: "Remove Browserbase key" }));
  expect(await keyInput()).toHaveValue("");
  expect(await api.getSettings()).toMatchObject({ defaultBrowser: "local", browserbase: { connected: false } });
  await waitFor(() => expect(scanDefaults.getByRole("radio", { name: "This Mac" })).toBeChecked());
  expect(scanDefaults.getByRole("radio", { name: "Browserbase" })).toBeDisabled();
});

test("what is stored and where traffic goes is said for Browserbase too", async () => {
  await renderWithApp(<ConnectedSettings />);
  const data = within(await screen.findByRole("region", { name: "Data" }));
  expect(data.getByText(/^Reports keep findings/)).toHaveTextContent(
    "Browserbase additionally keeps its own video and log of that session, which this app neither stores nor can delete.",
  );
  const about = within(await screen.findByRole("region", { name: "About" }));
  expect(about.getByText(/^Finds hidden revenue leaks/)).toHaveTextContent(
    "the app also talks to api.browserbase.com and to the browser at connect.browserbase.com",
  );
  expect(about.getByText(/^Finds hidden revenue leaks/)).toHaveTextContent(
    "Scan results are stored on this Mac only, and leave it only when you export a report or copy something out of it yourself.",
  );
});
