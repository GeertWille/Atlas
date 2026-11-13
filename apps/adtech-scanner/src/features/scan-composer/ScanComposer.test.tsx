import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, test, vi } from "vitest";
import { api } from "@/lib/api";
import { browserbaseSettingsFixture, makeConfiguration, makeProxy, settingsFixture } from "@/test/fixtures";
import { renderWithApp } from "@/test/render";
import { ScanComposer } from "./ScanComposer";

// The composer talks to the real api module, which resolves to the fake backend outside Tauri.
vi.hoisted(() => {
  localStorage.setItem("adscan.fake.profile", "fresh");
  localStorage.setItem("adscan.fake.speed", "0.001");
});

beforeEach(async () => {
  await api.deleteAllScans();
  for (const configuration of await api.listScanConfigurations()) await api.deleteScanConfiguration(configuration.id);
});

const validOptions = {
  url: "https://example.com/news",
  device: "desktop",
  observeSeconds: 20,
  autoScroll: true,
  acceptConsent: true,
  aiEnabled: true,
} as const;

const urlInput = () => screen.getByRole("textbox", { name: "Page URL" });
const scanButton = () => screen.getByRole("button", { name: "Scan" });
/** The options that answer "How should the visit behave?". */
const visit = () => within(screen.getByRole("region", { name: "Visit" }));

test("an empty URL is rejected with an explanation and nothing starts", async () => {
  const user = userEvent.setup();
  await renderWithApp(<ScanComposer settings={settingsFixture} />);

  await user.click(scanButton());

  expect(await screen.findByRole("alert")).toHaveTextContent("Enter the address of a page to scan.");
  expect(urlInput()).toBeInvalid();
  expect(urlInput()).toHaveAccessibleDescription("Enter the address of a page to scan.");
  expect((await api.listScans({})).total).toBe(0);
});

test("an invalid URL shows why, and the error clears as soon as the user edits", async () => {
  const user = userEvent.setup();
  await renderWithApp(<ScanComposer settings={settingsFixture} />);

  await user.type(urlInput(), "ftp://example.com{Enter}");
  expect(await screen.findByRole("alert")).toHaveTextContent("Only http and https pages can be scanned.");

  await user.type(urlInput(), "x");
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  expect(urlInput()).toBeValid();
});

test("a bare domain is normalised, started with the chosen options, and the live view opens", async () => {
  const user = userEvent.setup();
  const { router } = await renderWithApp(<ScanComposer settings={settingsFixture} />);

  await user.type(urlInput(), "  News.Example.com/sport ");
  await user.click(screen.getByRole("radio", { name: "Mobile" }));
  await user.click(screen.getByRole("button", { name: "Options" }));
  await user.click(screen.getByRole("radio", { name: "Observe only" }));
  await user.click(scanButton());

  expect(await screen.findByText("scan screen")).toBeInTheDocument();
  const { items } = await api.listScans({});
  expect(items).toHaveLength(1);
  expect(router.state.location.pathname).toBe(`/scans/${items[0]?.id}`);
  const detail = await api.getScan(items[0]?.id ?? "");
  expect(detail.options).toEqual({
    url: "https://news.example.com/sport",
    device: "mobile",
    observeSeconds: 20,
    autoScroll: false,
    acceptConsent: true,
    aiEnabled: true,
    // The backend resolves what the composer leaves to the settings.
    visitor: "none",
    extraPages: 0,
    headless: true,
    persistentProfile: false,
    proxy: { kind: "system" },
    budgetUsd: 0.5,
    seed: expect.any(Number),
    browser: { kind: "local" },
  });
});

test("without an API key the AI option is off and locked, and the scan runs deterministic checks only", async () => {
  const user = userEvent.setup();
  await renderWithApp(<ScanComposer settings={{ ...settingsFixture, hasApiKey: false }} />);

  // Said before any option is opened, with the way to change it.
  expect(scanButton()).toHaveAccessibleDescription("Scroll · browser hidden · system proxy · checks only, no API key");
  expect(screen.getByRole("link", { name: "Add a key" })).toHaveAttribute("href", "/settings");
  await user.click(screen.getByRole("button", { name: "Options" }));
  expect(screen.getByText(/this scan runs the deterministic checks only/)).toBeInTheDocument();
  const aiSwitch = screen.getByRole("switch", { name: "AI logic analysis" });
  expect(aiSwitch).not.toBeChecked();
  expect(aiSwitch).toBeDisabled();

  await user.type(urlInput(), "example.com{Enter}");
  await waitFor(async () => expect((await api.listScans({})).total).toBe(1));
  const [scan] = (await api.listScans({})).items;
  expect((await api.getScan(scan?.id ?? "")).options.aiEnabled).toBe(false);
});

test("with only a TypeSafe key the scan is script triage only, priced as such, and can still be turned off", async () => {
  const user = userEvent.setup();
  const jevOnly = { ...settingsFixture, hasApiKey: false, hasTypesafeKey: true, jevEnabled: true };
  await renderWithApp(<ScanComposer settings={jevOnly} />);

  expect(scanButton()).toHaveAccessibleDescription(
    "Scroll · browser hidden · system proxy · script triage, about $0.001",
  );
  await user.click(screen.getByRole("button", { name: "Options" }));
  expect(screen.getByText(/Estimated/)).toHaveTextContent(
    "Estimated $0.001 for Jev script triage. Add an Anthropic key to analyse wrapper logic.",
  );
  expect(screen.getByRole("link", { name: "Add an Anthropic key" })).toHaveAttribute("href", "/settings");
  expect(screen.queryByRole("switch", { name: "AI logic analysis" })).toBeNull();
  const triageSwitch = screen.getByRole("switch", { name: "AI script triage" });
  expect(triageSwitch).toBeChecked();
  expect(triageSwitch).toBeEnabled();
  expect(screen.getByText("Classify scripts with Jev. Logic analysis needs an Anthropic key.")).toBeInTheDocument();

  await user.type(urlInput(), "example.com{Enter}");
  await waitFor(async () => expect((await api.listScans({})).total).toBe(1));
  const [scan] = (await api.listScans({})).items;
  expect((await api.getScan(scan?.id ?? "")).options.aiEnabled).toBe(true);
});

test("a TypeSafe key with the Jev switch off does not unlock AI on its own", async () => {
  const user = userEvent.setup();
  await renderWithApp(<ScanComposer settings={{ ...settingsFixture, hasApiKey: false, hasTypesafeKey: true }} />);
  expect(scanButton()).toHaveAccessibleDescription(/checks only, no API key$/);
  await user.click(screen.getByRole("button", { name: "Options" }));
  expect(screen.getByText(/this scan runs the deterministic checks only/)).toBeInTheDocument();
  expect(screen.getByRole("switch", { name: "AI logic analysis" })).toBeDisabled();
});

test("with both keys and Jev on, the cost line names both models", async () => {
  const user = userEvent.setup();
  await renderWithApp(<ScanComposer settings={{ ...settingsFixture, hasTypesafeKey: true, jevEnabled: true }} />);
  expect(scanButton()).toHaveAccessibleDescription(/AI on, about \$0\.37$/);
  await user.click(screen.getByRole("button", { name: "Options" }));
  expect(screen.getByText(/Estimated/)).toHaveTextContent(
    "Estimated $0.37 with claude-sonnet-5-5 and Jev script triage, hard cap $0.50 per scan.",
  );
});

test("the cost line reflects the settings and the AI switch", async () => {
  const user = userEvent.setup();
  await renderWithApp(<ScanComposer settings={{ ...settingsFixture, analysisEffort: "high" }} />);

  expect(scanButton()).toHaveAccessibleDescription(/AI on, about \$0\.50$/);
  await user.click(screen.getByRole("button", { name: "Options" }));
  expect(screen.getByText(/Estimated/)).toHaveTextContent(
    "Estimated $0.50 with claude-sonnet-5-5, hard cap $0.50 per scan. Analysis may stop early at the cap.",
  );
  await user.click(screen.getByRole("switch", { name: "AI logic analysis" }));
  expect(screen.getByText("Deterministic checks only. No LLM cost.")).toBeInTheDocument();
  expect(scanButton()).toHaveAccessibleDescription(/· AI off$/);
  // A cap has nothing to cap then, and says so.
  expect(screen.getByRole("textbox", { name: "LLM budget" })).toHaveAccessibleDescription("Not used while AI is off.");
});

test("pressing Enter twice starts one scan, not two", async () => {
  const user = userEvent.setup();
  await renderWithApp(<ScanComposer settings={settingsFixture} />);

  await user.type(urlInput(), "example.com{Enter}{Enter}");

  await screen.findByText("scan screen");
  expect((await api.listScans({})).total).toBe(1);
});

const openOptions = async (user: ReturnType<typeof userEvent.setup>) =>
  user.click(screen.getByRole("button", { name: "Options" }));
const startedOptions = async () => {
  await waitFor(async () => expect((await api.listScans({})).total).toBe(1));
  const [scan] = (await api.listScans({})).items;
  return (await api.getScan(scan?.id ?? "")).options;
};

test("the visitor choice replaces auto-scroll: four behaviours, scroll by default, each described", async () => {
  const user = userEvent.setup();
  await renderWithApp(<ScanComposer settings={settingsFixture} />);
  // What Scan will do is said in one line before any option is opened.
  expect(scanButton()).toHaveAccessibleDescription("Scroll · browser hidden · system proxy · AI on, about $0.36");
  await openOptions(user);

  expect(screen.queryByRole("switch", { name: "Auto-scroll" })).toBeNull();
  const visitor = screen.getByRole("radiogroup", { name: "Visitor" });
  expect(
    within(visitor)
      .getAllByRole("radio")
      .map((radio) => radio.textContent),
  ).toEqual(["Observe only", "Scroll", "Browse like a visitor", "I'll browse myself"]);
  expect(within(visitor).getByRole("radio", { name: "Scroll" })).toBeChecked();
  expect(
    screen.getByText("Scrolls to the bottom in steps, so lazy-loaded slots get their chance."),
  ).toBeInTheDocument();
  // A scripted visit never claims to be a person, and says what the page can tell: Chrome reports
  // navigator.webdriver false in every mode, so only a hidden browser is recognisable.
  expect(
    screen.getByText(
      "This visit is automated. Chrome does not tell the page (navigator.webdriver is false), but a hidden (headless) browser is recognisable.",
    ),
  ).toBeInTheDocument();
  expect(screen.queryByText(/webdriver is true|says so to the page/)).toBeNull();
  // Scroll and observe have no page count, and say where one is; the settings run them headless.
  expect(screen.queryByRole("button", { name: "More pages" })).toBeNull();
  expect(visit().getByText("1 page")).toBeInTheDocument();
  expect(visit().getByText("Only “Browse like a visitor” follows links to further pages.")).toBeInTheDocument();
  expect(screen.getByRole("switch", { name: "Show the browser" })).not.toBeChecked();
  expect(screen.getByText(/Headless Chrome is easier for ad partners to recognise as automated/)).toBeInTheDocument();
});

test("browse like a visitor: pages to visit, the browser shown by default, and a cost that grows per page", async () => {
  const user = userEvent.setup();
  await renderWithApp(<ScanComposer settings={{ ...settingsFixture, budgetUsd: 5 }} />);
  await openOptions(user);
  expect(screen.getByText(/Estimated/)).toHaveTextContent(
    "Estimated $0.36 with claude-sonnet-5-5, hard cap $5.00 per scan.",
  );

  await user.click(screen.getByRole("radio", { name: "Browse like a visitor" }));
  expect(screen.getByRole("switch", { name: "Show the browser" })).toBeChecked();
  expect(visit().getByText("3 pages")).toBeInTheDocument();
  expect(
    screen.getByText(
      /The entry page, then up to 2 links on the same site\. Ads, sign-in and checkout links are never clicked\./,
    ),
  ).toBeInTheDocument();
  expect(screen.getByText(/Estimated/)).toHaveTextContent(
    "Estimated $0.37 with claude-sonnet-5-5 for 3 pages, hard cap $5.00 per scan.",
  );
  expect(scanButton()).toHaveAccessibleDescription(
    "Browse like a visitor · 3 pages · browser shown · system proxy · AI on, about $0.37",
  );

  for (let press = 0; press < 3; press += 1) await user.click(screen.getByRole("button", { name: "More pages" }));
  expect(visit().getByText("6 pages")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "More pages" })).toBeDisabled();
  expect(screen.getByText(/Estimated/)).toHaveTextContent("Estimated $0.38 with claude-sonnet-5-5 for 6 pages");
  await user.click(screen.getByRole("button", { name: "Fewer pages" }));
  await user.click(screen.getByRole("switch", { name: "Show the browser" }));

  await user.type(urlInput(), "example.com{Enter}");
  expect(await startedOptions()).toMatchObject({ visitor: "browse", extraPages: 4, headless: true, autoScroll: true });
});

test("I'll browse myself: no observe time, no consent click, no hidden browser, and an honest explanation", async () => {
  const user = userEvent.setup();
  await renderWithApp(<ScanComposer settings={settingsFixture} />);
  await openOptions(user);
  await user.click(screen.getByRole("radio", { name: "I'll browse myself" }));

  expect(screen.queryByRole("slider")).toBeNull();
  expect(screen.queryByRole("switch", { name: "Accept consent banner" })).toBeNull();
  expect(screen.queryByRole("switch", { name: "Show the browser" })).toBeNull();
  expect(screen.queryByRole("button", { name: "More pages" })).toBeNull();
  // Each option keeps its place and says what holds instead.
  expect(screen.getByText(/Chrome opens; browse the site, then press/)).toHaveTextContent(
    "Chrome opens; browse the site, then press Finish capture.",
  );
  expect(visit().getByText("Up to 15 minutes")).toBeInTheDocument();
  expect(visit().getByText("Every page you open in that window is recorded.")).toBeInTheDocument();
  expect(visit().getByText("There is nobody to browse a hidden window.")).toBeInTheDocument();
  expect(visit().getByText("You handle the consent banner yourself; nothing is clicked for you.")).toBeInTheDocument();
  expect(
    visit().getByText("You drive the browser. The page can still notice the debugger that records it."),
  ).toBeInTheDocument();
  expect(scanButton()).toHaveAccessibleDescription(/^I'll browse myself · browser shown ·/);
  expect(screen.queryByText(/This visit is automated/)).toBeNull();
  expect(document.body).not.toHaveTextContent(/undetect|indistinguishable/i);

  await user.type(urlInput(), "example.com{Enter}");
  expect(await startedOptions()).toMatchObject({
    visitor: "manual",
    headless: false,
    acceptConsent: false,
    extraPages: 0,
  });
});

test("the profile state is shown with a way to change it, and follows the settings", async () => {
  const user = userEvent.setup();
  const { unmount } = await renderWithApp(<ScanComposer settings={settingsFixture} />);
  await openOptions(user);
  expect(screen.getByRole("combobox", { name: "Browser profile" })).toHaveTextContent(
    "Settings default (fresh each scan)",
  );
  expect(screen.getByText(/Starts without cookies, like a first-time visitor\./)).toBeInTheDocument();
  expect(screen.getByRole("link", { name: "Change the default" })).toHaveAttribute("href", "/settings");
  expect(scanButton()).not.toHaveAccessibleDescription(/kept browser profile/);
  unmount();

  await renderWithApp(<ScanComposer settings={{ ...settingsFixture, persistentProfile: true }} />);
  await user.click(screen.getByRole("button", { name: "Options" }));
  expect(screen.getByRole("combobox", { name: "Browser profile" })).toHaveTextContent(
    "Settings default (kept between scans)",
  );
  expect(
    screen.getByText(/Cookies and consent carry over from earlier scans, like a returning visitor's\./),
  ).toBeInTheDocument();
  expect(scanButton()).toHaveAccessibleDescription(/· kept browser profile ·/);
});

test("proxy, browser profile and budget stay out of the way until Options is opened, and then override the settings for this scan", async () => {
  const user = userEvent.setup();
  // Belgium exists in the backend, so a scan through it can start; Netherlands is only listed.
  const stored = await api.saveProxyProfile({
    name: "Belgium",
    scheme: "http",
    host: "be.proxy.example",
    port: 8080,
    username: null,
  });
  const belgium = stored.proxies[0] ?? makeProxy();
  const proxies = [belgium, makeProxy({ id: "proxy-nl", name: "Netherlands" })];
  await renderWithApp(<ScanComposer settings={{ ...settingsFixture, proxies }} />);
  expect(screen.queryByRole("combobox", { name: "Proxy" })).toBeNull();
  expect(screen.queryByRole("combobox", { name: "Browser profile" })).toBeNull();
  expect(screen.queryByRole("textbox", { name: "LLM budget" })).toBeNull();

  await openOptions(user);
  await user.click(screen.getByRole("combobox", { name: "Proxy" }));
  expect((await screen.findAllByRole("option")).map((option) => option.textContent)).toEqual([
    "Settings default (System proxy)",
    "No proxy",
    "System proxy",
    "Belgium",
    "Netherlands",
  ]);
  await user.click(screen.getByRole("option", { name: "Belgium" }));
  await user.click(screen.getByRole("combobox", { name: "Browser profile" }));
  await user.click(await screen.findByRole("option", { name: "Kept between scans" }));
  expect(screen.getByText(/Cookies and consent carry over from earlier scans/)).toBeInTheDocument();
  expect(screen.queryByRole("link", { name: "Change the default" })).toBeNull();
  expect(scanButton()).toHaveAccessibleDescription(
    "Scroll · browser hidden · Belgium proxy · kept browser profile · AI on, about $0.36",
  );

  const budget = screen.getByLabelText("LLM budget");
  await user.type(budget, "0.01");
  await user.type(urlInput(), "example.com{Enter}");
  expect(await screen.findByText("Enter an amount from $0.05 to $20.00.")).toBeInTheDocument();
  expect(budget).toBeInvalid();
  expect((await api.listScans({})).total).toBe(0);

  await user.clear(budget);
  await user.type(budget, "0.2");
  expect(screen.getByText(/Estimated/)).toHaveTextContent(
    "Estimated $0.20 with claude-sonnet-5-5, hard cap $0.20 per scan. Analysis may stop early at the cap.",
  );
  await user.click(scanButton());
  expect(await startedOptions()).toMatchObject({
    proxy: { kind: "profile", id: belgium.id, name: "Belgium" },
    persistentProfile: true,
    budgetUsd: 0.2,
  });
  await api.deleteProxyProfile(belgium.id);
});

test("the options can be saved as a named configuration, which the composer then has open", async () => {
  const user = userEvent.setup();
  const { router } = await renderWithApp(<ScanComposer settings={settingsFixture} />);
  // Saving is offered before any option is opened.
  expect(screen.getByRole("button", { name: "Save as configuration…" })).toBeInTheDocument();
  await openOptions(user);
  await user.click(screen.getByRole("radio", { name: "Browse like a visitor" }));

  // Nothing to save without a page.
  await user.click(screen.getByRole("button", { name: "Save as configuration…" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("Enter the address of a page to scan.");
  expect(screen.queryByRole("dialog")).toBeNull();

  await user.type(urlInput(), "example.com/news");
  await user.click(screen.getByRole("button", { name: "Save as configuration…" }));
  const dialog = await screen.findByRole("dialog", { name: "Save as configuration" });
  await user.click(within(dialog).getByRole("button", { name: "Save configuration" }));
  expect(within(dialog).getByRole("alert")).toHaveTextContent("Give the configuration a name.");
  await user.type(within(dialog).getByLabelText("Name"), "Example, three pages");
  await user.click(within(dialog).getByRole("button", { name: "Save configuration" }));

  await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  const [saved] = await api.listScanConfigurations();
  expect(saved).toMatchObject({
    name: "Example, three pages",
    options: { url: "https://example.com/news", visitor: "browse", extraPages: 2, headless: false },
  });
  expect(router.state.location.search).toEqual({ configuration: saved?.id });
  // Saving started no scan.
  expect((await api.listScans({})).total).toBe(0);

  // The same name again is refused by the backend, in the dialog.
  await user.click(screen.getByRole("button", { name: "Save as configuration…" }));
  const again = await screen.findByRole("dialog");
  await user.type(within(again).getByLabelText("Name"), "example, THREE pages");
  await user.click(within(again).getByRole("button", { name: "Save configuration" }));
  expect(await within(again).findByRole("alert")).toHaveTextContent(
    'A scan configuration named "example, THREE pages" already exists',
  );
});

test("a loaded configuration shows its name, runs as saved, and offers to save edits or copy them", async () => {
  const user = userEvent.setup();
  const configuration = await api.saveScanConfiguration({
    name: "Example on mobile",
    options: { ...validOptions, device: "mobile", visitor: "browse", extraPages: 1, headless: false },
  });
  await renderWithApp(<ScanComposer settings={settingsFixture} configuration={configuration} />);

  // Opened for editing: the options are in view and filled in from the configuration.
  expect(screen.getByText("Example on mobile")).toBeInTheDocument();
  expect(urlInput()).toHaveValue("https://example.com/news");
  expect(screen.getByRole("radio", { name: "Mobile" })).toBeChecked();
  expect(screen.getByRole("radio", { name: "Browse like a visitor" })).toBeChecked();
  expect(visit().getByText("2 pages")).toBeInTheDocument();
  expect(screen.getByText("Scan runs this configuration as saved and records it as its last run.")).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Update configuration" })).toBeNull();
  expect(screen.queryByRole("button", { name: "Revert" })).toBeNull();
  expect(screen.queryByRole("button", { name: "Save as configuration…" })).toBeNull();
  expect(screen.getByRole("link", { name: "Close configuration" })).toHaveAttribute("href", "/");

  // Unchanged, Scan is a run of the configuration.
  await user.click(scanButton());
  await screen.findByText("scan screen");
  const [run] = (await api.listScans({})).items;
  expect(run?.configuration).toEqual({ id: configuration.id, name: "Example on mobile" });
});

test("edits to a loaded configuration are marked, scanned once without it, or saved into it", async () => {
  const user = userEvent.setup();
  const configuration = await api.saveScanConfiguration({
    name: "Example on mobile",
    options: { ...validOptions, device: "mobile", visitor: "browse", extraPages: 1, headless: false },
  });
  await renderWithApp(<ScanComposer settings={settingsFixture} configuration={configuration} />);

  await user.click(screen.getByRole("button", { name: "More pages" }));
  expect(screen.getByText("Edited")).toBeInTheDocument();
  expect(screen.getByText("Scan runs these edited options once; they are not saved yet.")).toBeInTheDocument();
  // Back to the saved value: no longer edited.
  await user.click(screen.getByRole("button", { name: "Fewer pages" }));
  expect(screen.queryByText("Edited")).toBeNull();

  await user.click(screen.getByRole("button", { name: "More pages" }));
  await user.click(screen.getByRole("button", { name: "Update configuration" }));
  expect(await screen.findByText("Updated “Example on mobile”")).toBeInTheDocument();
  await waitFor(() => expect(screen.queryByText("Edited")).toBeNull());
  expect((await api.listScanConfigurations())[0]?.options.extraPages).toBe(2);

  // Edited again and scanned: a scan of its own, not a run of the configuration.
  await user.click(screen.getByRole("radio", { name: "Desktop" }));
  await user.click(screen.getByRole("button", { name: "Save as new…" }));
  expect(await screen.findByRole("dialog", { name: "Save as configuration" })).toBeInTheDocument();
  await user.keyboard("{Escape}");
  await user.click(scanButton());
  await screen.findByText("scan screen");
  const [scan] = (await api.listScans({})).items;
  expect(scan).toMatchObject({ device: "desktop", configuration: null });
  expect((await api.listScanConfigurations())[0]?.options.device).toBe("mobile");
});

test("a configuration is only marked as edited when the scan itself differs, not how a value is typed", async () => {
  const user = userEvent.setup();
  const configuration = await api.saveScanConfiguration({
    name: "Example with a cap",
    options: { ...validOptions, budgetUsd: 0.5 },
  });
  // No key is stored here: the AI switch is off and locked, which is not an edit of the configuration.
  await renderWithApp(
    <ScanComposer settings={{ ...settingsFixture, hasApiKey: false }} configuration={configuration} />,
  );
  const budget = screen.getByRole("textbox", { name: "LLM budget" });
  expect(budget).toHaveValue("0.50");
  expect(screen.queryByText("Edited")).toBeNull();

  await user.clear(budget);
  expect(screen.getByText("Edited")).toBeInTheDocument();
  await user.type(budget, "0.5");
  expect(screen.queryByText("Edited")).toBeNull();
  await user.type(budget, "5");
  expect(screen.getByText("Edited")).toBeInTheDocument();
});

test("updating a configuration while no key is stored keeps its AI choice", async () => {
  const user = userEvent.setup();
  const configuration = await api.saveScanConfiguration({ name: "Example with AI", options: validOptions });
  await renderWithApp(
    <ScanComposer settings={{ ...settingsFixture, hasApiKey: false }} configuration={configuration} />,
  );
  expect(screen.getByRole("switch", { name: "AI logic analysis" })).toBeDisabled();

  await user.click(screen.getByRole("radio", { name: "Mobile" }));
  await user.click(screen.getByRole("button", { name: "Update configuration" }));
  expect(await screen.findByText("Updated “Example with AI”")).toBeInTheDocument();
  // The switch only shows what a scan started now would do; the configuration still asks for the analysis.
  expect((await api.listScanConfigurations())[0]?.options).toMatchObject({ device: "mobile", aiEnabled: true });
});

test("leaving a configuration with unsaved edits asks first; unchanged it just leaves", async () => {
  const user = userEvent.setup();
  const configuration = await api.saveScanConfiguration({
    name: "Example on mobile",
    options: { ...validOptions, device: "mobile" },
  });
  const { router } = await renderWithApp(<ScanComposer settings={settingsFixture} configuration={configuration} />);
  await user.click(screen.getByRole("radio", { name: "Desktop" }));

  void router.navigate({ to: "/history" });
  const question = await screen.findByRole("alertdialog", { name: "Discard the changes to “Example on mobile”?" });
  await user.click(within(question).getByRole("button", { name: "Keep editing" }));
  await waitFor(() => expect(screen.queryByRole("alertdialog")).toBeNull());
  expect(router.state.location.pathname).toBe("/");
  expect(screen.getByRole("radio", { name: "Desktop" })).toBeChecked();

  void router.navigate({ to: "/history" });
  await user.click(within(await screen.findByRole("alertdialog")).getByRole("button", { name: "Discard changes" }));
  expect(await screen.findByText("history screen")).toBeInTheDocument();
  expect((await api.listScanConfigurations())[0]?.options.device).toBe("mobile");
});

test("an unchanged configuration, and a scan of the edited options, leave without a question", async () => {
  const user = userEvent.setup();
  const configuration = await api.saveScanConfiguration({
    name: "Example on mobile",
    options: { ...validOptions, device: "mobile" },
  });
  const first = await renderWithApp(<ScanComposer settings={settingsFixture} configuration={configuration} />);
  void first.router.navigate({ to: "/history" });
  expect(await screen.findByText("history screen")).toBeInTheDocument();
  first.unmount();

  await renderWithApp(<ScanComposer settings={settingsFixture} configuration={configuration} />);
  await user.click(screen.getByRole("radio", { name: "Desktop" }));
  await user.click(scanButton());
  expect(await screen.findByText("scan screen")).toBeInTheDocument();
  expect(screen.queryByRole("alertdialog")).toBeNull();
});

test("a configuration whose proxy was deleted says so and cannot be scanned until another proxy is chosen", async () => {
  const user = userEvent.setup();
  const configuration = makeConfiguration({
    options: {
      ...validOptions,
      proxy: { kind: "profile", id: "proxy-gone", name: "Germany", host: "de.proxy.example" },
    },
  });
  await renderWithApp(<ScanComposer settings={settingsFixture} configuration={configuration} />);

  // The options are open, because that is where the problem is; the summary names it too.
  const proxy = screen.getByRole("combobox", { name: "Proxy" });
  expect(proxy).toBeInvalid();
  expect(scanButton()).toHaveAccessibleDescription(/· deleted proxy ·/);
  const blocked = "This configuration cannot run until another proxy is chosen under Origin.";
  expect(screen.getByText(blocked)).toBeInTheDocument();
  expect(screen.getByText("This proxy was deleted. Choose another one.")).toBeInTheDocument();
  await user.click(scanButton());
  expect((await api.listScans({})).total).toBe(0);

  await user.click(proxy);
  await user.click(await screen.findByRole("option", { name: "No proxy" }));
  expect(screen.queryByText("This proxy was deleted. Choose another one.")).toBeNull();
  expect(screen.queryByText(blocked)).toBeNull();
  await user.click(scanButton());
  expect(await startedOptions()).toMatchObject({ proxy: { kind: "none" } });
});

test("revert puts the saved options back, without a question and without saving anything", async () => {
  const user = userEvent.setup();
  const configuration = await api.saveScanConfiguration({
    name: "Example on mobile",
    options: { ...validOptions, device: "mobile", visitor: "browse", extraPages: 1, headless: false },
  });
  await renderWithApp(<ScanComposer settings={settingsFixture} configuration={configuration} />);

  await user.click(screen.getByRole("radio", { name: "Desktop" }));
  await user.click(screen.getByRole("radio", { name: "I'll browse myself" }));
  expect(screen.getByText("Edited")).toBeInTheDocument();

  await user.click(screen.getByRole("button", { name: "Revert" }));
  expect(screen.queryByText("Edited")).toBeNull();
  expect(screen.getByRole("radio", { name: "Mobile" })).toBeChecked();
  expect(screen.getByRole("radio", { name: "Browse like a visitor" })).toBeChecked();
  expect(visit().getByText("2 pages")).toBeInTheDocument();
  expect(screen.queryByRole("alertdialog")).toBeNull();
  expect((await api.listScanConfigurations())[0]?.options).toMatchObject({ device: "mobile", visitor: "browse" });
});

test("the options keep their places when the visitor changes: every behaviour shows the same four", async () => {
  const user = userEvent.setup();
  await renderWithApp(<ScanComposer settings={settingsFixture} />);
  await openOptions(user);

  const labels = () =>
    ["Pages to visit", "Show the browser", "Accept consent banner"].map((label) => visit().getByText(label));
  for (const visitor of ["Observe only", "Scroll", "Browse like a visitor", "I'll browse myself"]) {
    await user.click(screen.getByRole("radio", { name: visitor }));
    expect(labels()).toHaveLength(3);
  }
  // The first place is the time: watched per page, or as long as the person takes.
  expect(visit().getByText("Session length")).toBeInTheDocument();
  await user.click(screen.getByRole("radio", { name: "Browse like a visitor" }));
  expect(visit().getByText("Time on each page")).toBeInTheDocument();
  await user.click(screen.getByRole("radio", { name: "Scroll" }));
  expect(visit().getByText("Observe after load")).toBeInTheDocument();
});

test("the options are grouped by the question they answer", async () => {
  const user = userEvent.setup();
  await renderWithApp(<ScanComposer settings={settingsFixture} />);
  await openOptions(user);

  const origin = within(screen.getByRole("region", { name: "Origin" }));
  expect(origin.getByRole("combobox", { name: "Proxy" })).toBeInTheDocument();
  expect(origin.getByRole("combobox", { name: "Browser profile" })).toBeInTheDocument();
  const analysis = within(screen.getByRole("region", { name: "Analysis" }));
  expect(analysis.getByRole("switch", { name: "AI logic analysis" })).toBeInTheDocument();
  expect(analysis.getByRole("textbox", { name: "LLM budget" })).toBeInTheDocument();
  expect(analysis.getByText(/Estimated/)).toBeInTheDocument();
  expect(visit().getByRole("radiogroup", { name: "Visitor" })).toBeInTheDocument();
  expect(visit().getByRole("slider")).toBeInTheDocument();
});

test("the keyboard scans from any field with Mod+Enter and saves with Mod+S", async () => {
  const user = userEvent.setup();
  await renderWithApp(<ScanComposer settings={settingsFixture} />);
  await openOptions(user);
  await user.type(urlInput(), "example.com/news");

  // Mod+S with no configuration open: the save dialog.
  screen.getByRole("switch", { name: "Accept consent banner" }).focus();
  await user.keyboard("{Control>}s{/Control}");
  const dialog = await screen.findByRole("dialog", { name: "Save as configuration" });
  // Keys pressed inside the dialog are the dialog's own: nothing is scanned behind it.
  await user.type(within(dialog).getByLabelText("Name"), "Example{Control>}{Enter}{/Control}");
  expect((await api.listScans({})).total).toBe(0);
  await user.keyboard("{Escape}");
  await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());

  screen.getByRole("switch", { name: "Accept consent banner" }).focus();
  await user.keyboard("{Control>}{Enter}{/Control}");
  expect(await startedOptions()).toMatchObject({ url: "https://example.com/news" });
});

test("Mod+Enter and Mod+S work from anywhere on the screen, but not from a field that is not the composer's", async () => {
  const user = userEvent.setup();
  await renderWithApp(
    <>
      <ScanComposer settings={settingsFixture} />
      <input aria-label="Another field" />
      <button type="button">Elsewhere</button>
    </>,
  );
  await user.type(urlInput(), "example.com/news");

  // Typing in a field outside the composer is not a wish to scan or save.
  screen.getByRole("textbox", { name: "Another field" }).focus();
  await user.keyboard("{Control>}{Enter}{/Control}{Control>}s{/Control}");
  expect(screen.queryByRole("dialog")).toBeNull();
  expect((await api.listScans({})).total).toBe(0);

  // From a button elsewhere on the screen, and with nothing focused at all, the keys reach the composer.
  screen.getByRole("button", { name: "Elsewhere" }).focus();
  await user.keyboard("{Control>}s{/Control}");
  expect(await screen.findByRole("dialog", { name: "Save as configuration" })).toBeInTheDocument();
  await user.keyboard("{Escape}");
  await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  (document.activeElement as HTMLElement | null)?.blur();
  await user.keyboard("{Control>}{Enter}{/Control}");
  expect(await startedOptions()).toMatchObject({ url: "https://example.com/news" });
});

test("Mod+S updates the open configuration once it is edited", async () => {
  const user = userEvent.setup();
  const configuration = await api.saveScanConfiguration({ name: "Example", options: validOptions });
  await renderWithApp(<ScanComposer settings={settingsFixture} configuration={configuration} />);

  await user.click(screen.getByRole("radio", { name: "Mobile" }));
  await user.keyboard("{Control>}s{/Control}");
  expect(await screen.findByText("Updated “Example”")).toBeInTheDocument();
  expect((await api.listScanConfigurations())[0]?.options.device).toBe("mobile");
});

/** The options that answer "Where does the visit come from?". */
const origin = () => within(screen.getByRole("region", { name: "Origin" }));
const analysis = () => within(screen.getByRole("region", { name: "Analysis" }));
const summaryLine = () => scanButton().getAttribute("aria-describedby") ?? "";
const summaryText = () => document.getElementById(summaryLine())?.textContent ?? "";
const chooseOption = async (user: ReturnType<typeof userEvent.setup>, combobox: string, option: string | RegExp) => {
  await user.click(origin().getByRole("combobox", { name: combobox }));
  await user.click(await screen.findByRole("option", { name: option }));
};

test("without Browserbase set up, the choice is there but disabled, with the reason and the way to Settings", async () => {
  const user = userEvent.setup();
  await renderWithApp(<ScanComposer settings={settingsFixture} />);
  await user.type(urlInput(), "example.com");
  await user.click(screen.getByRole("button", { name: "Options" }));

  expect(origin().getByRole("radio", { name: "This Mac" })).toBeChecked();
  const browserbase = origin().getByRole("radio", { name: "Browserbase" });
  expect(browserbase).toBeDisabled();
  expect(browserbase).toHaveAccessibleDescription("Browserbase, a cloud browser, is not set up. Set it up in Settings");
  expect(origin().getByRole("link", { name: "Set it up in Settings" })).toHaveAttribute("href", "/settings");
  // What only a Browserbase browser has is not there on this Mac, not even as a placeholder.
  expect(origin().queryByText("Region")).toBeNull();
  expect(origin().queryByRole("combobox", { name: "Region" })).toBeNull();
  // Nothing about browser cost for someone who has no Browserbase.
  expect(analysis().queryByRole("note", { name: "Browser cost" })).toBeNull();
});

test("on Browserbase the options that depend on it change in place: region, location, window, profile and cost", async () => {
  const user = userEvent.setup();
  await renderWithApp(<ScanComposer settings={{ ...browserbaseSettingsFixture, proxies: [makeProxy()] }} />);
  await user.type(urlInput(), "example.com");
  await user.click(screen.getByRole("button", { name: "Options" }));
  // On this Mac there is no region and no browser cost to speak of.
  expect(origin().queryByText("Region")).toBeNull();
  expect(analysis().queryByRole("note", { name: "Browser cost" })).toBeNull();
  expect(visit().getByRole("switch", { name: "Show the browser" })).toBeInTheDocument();

  await user.click(origin().getByRole("radio", { name: "Browserbase" }));
  expect(origin().getByRole("radio", { name: "Browserbase" })).toBeChecked();
  expect(summaryText()).toBe("Scroll · Browserbase, Oregon (us-west-2) · no proxy · AI on, about $0.36");

  // A cloud browser has no window: the option keeps its place and says so.
  expect(visit().queryByRole("switch", { name: "Show the browser" })).toBeNull();
  expect(visit().getByText("No window")).toBeInTheDocument();
  expect(visit().getByText("A cloud browser has no window. The report shows what it loaded.")).toBeInTheDocument();
  expect(
    visit().getByText(/Browserbase's browser does not tell the page \(navigator\.webdriver is false\)/),
  ).toBeInTheDocument();

  expect(origin().getByRole("combobox", { name: "Region" })).toHaveTextContent("Settings default (Oregon)");
  expect(
    origin().getByText("A region near you is faster. Without a proxy, it is also where the visit comes from."),
  ).toBeInTheDocument();
  await chooseOption(user, "Region", /Frankfurt/);

  // The proxy control is now about where the visit leaves from; this Mac's own proxy is not on offer.
  expect(origin().queryByRole("combobox", { name: "Proxy" })).toBeNull();
  await user.click(origin().getByRole("combobox", { name: "Location" }));
  expect((await screen.findAllByRole("option")).map((option) => option.textContent)).toEqual([
    "Settings default (No proxy)",
    "No proxy: leaves from Frankfurt (eu-central-1)",
    "Belgium",
    "Browserbase location…",
  ]);
  await user.click(screen.getByRole("option", { name: "Belgium" }));
  expect(
    origin().getByText(
      "Browserbase connects through your proxy. Its address and password are sent to Browserbase for that.",
    ),
  ).toBeInTheDocument();
  expect(summaryText()).toBe("Scroll · Browserbase, Frankfurt (eu-central-1) · Belgium proxy · AI on, about $0.36");

  // Keeping a profile at Browserbase is its own switch, and it is off.
  await user.click(origin().getByRole("combobox", { name: "Browser profile" }));
  expect(await screen.findByRole("option", { name: "Kept between scans" })).toHaveAttribute("aria-disabled", "true");
  await user.keyboard("{Escape}");
  expect(origin().getByText(/Starts fresh: keeping a profile at Browserbase is off\./)).toBeInTheDocument();

  // Browserbase is estimated apart from the models, and the model estimate is what it was.
  expect(analysis().getByRole("note", { name: "Browser cost" })).toHaveTextContent(
    "Browserbase bills apart from the LLM budget: about 1 browser minute, no proxy traffic. An estimated $0.002 at Browserbase's list price of $0.12 per browser hour and $12 per GB (2026-11-12); your plan's included usage is not counted.",
  );
  expect(analysis().getByRole("note", { name: "Browser cost" })).not.toHaveTextContent(/Developer|Startup|overage/);
  expect(analysis().getByText(/hard cap/)).toHaveTextContent(
    "Estimated $0.36 with claude-sonnet-5-5, hard cap $0.50 per scan.",
  );

  // Back on this Mac everything is as it was.
  await user.click(origin().getByRole("radio", { name: "This Mac" }));
  expect(visit().getByRole("switch", { name: "Show the browser" })).toBeInTheDocument();
  expect(origin().getByRole("combobox", { name: "Proxy" })).toHaveTextContent("Belgium");
  expect(summaryText()).toBe("Scroll · browser hidden · Belgium proxy · AI on, about $0.36");
  expect(origin().queryByText("Region")).toBeNull();
  expect(analysis().queryByRole("note", { name: "Browser cost" })).toBeNull();
});

test("the note about automation follows the browser window: only a hidden local browser is recognisable", async () => {
  const user = userEvent.setup();
  await renderWithApp(<ScanComposer settings={settingsFixture} />);
  await user.click(screen.getByRole("button", { name: "Options" }));
  expect(visit().getByRole("switch", { name: "Show the browser" })).not.toBeChecked();
  expect(visit().getByText(/but a hidden \(headless\) browser is recognisable\./)).toBeInTheDocument();

  await user.click(visit().getByRole("switch", { name: "Show the browser" }));
  expect(
    visit().getByText(
      "This visit is automated. Chrome does not tell the page (navigator.webdriver is false), and it is not hidden (headless) either.",
    ),
  ).toBeInTheDocument();
  expect(visit().queryByText(/recognisable/)).toBeNull();

  // A browse visit is shown by default, and says the same.
  await user.click(visit().getByRole("radio", { name: "Browse like a visitor" }));
  expect(visit().getByRole("switch", { name: "Show the browser" })).toBeChecked();
  expect(visit().getByText(/and it is not hidden \(headless\) either\./)).toBeInTheDocument();
});

test("looking at Browserbase and coming back leaves the proxy choice as it was", async () => {
  const user = userEvent.setup();
  await api.setBrowserbaseCredentials("bb_live_0123456789abcdef");
  await renderWithApp(<ScanComposer settings={{ ...browserbaseSettingsFixture, defaultProxy: { kind: "none" } }} />);
  await user.type(urlInput(), "example.com");
  await user.click(screen.getByRole("button", { name: "Options" }));
  await chooseOption(user, "Proxy", "System proxy");

  // This Mac's own proxy means nothing to a cloud browser: there the settings hold.
  await user.click(origin().getByRole("radio", { name: "Browserbase" }));
  expect(origin().getByRole("combobox", { name: "Location" })).toHaveTextContent("Settings default (No proxy)");
  await user.click(origin().getByRole("radio", { name: "This Mac" }));
  expect(origin().getByRole("combobox", { name: "Proxy" })).toHaveTextContent("System proxy");
  expect(summaryText()).toBe("Scroll · browser hidden · system proxy · AI on, about $0.36");

  // The same the other way: a location typed for Browserbase is still there after a look at this Mac.
  await user.click(origin().getByRole("radio", { name: "Browserbase" }));
  await chooseOption(user, "Location", "Browserbase location…");
  await user.type(origin().getByLabelText("Country code"), "be");
  await user.click(origin().getByRole("radio", { name: "This Mac" }));
  expect(origin().getByRole("combobox", { name: "Proxy" })).toHaveTextContent("Settings default (No proxy)");
  await user.click(origin().getByRole("radio", { name: "Browserbase" }));
  expect(origin().getByRole("combobox", { name: "Location" })).toHaveTextContent("Browserbase location…");
  expect(origin().getByLabelText("Country code")).toHaveValue("be");

  await user.click(origin().getByRole("radio", { name: "This Mac" }));
  await chooseOption(user, "Proxy", "System proxy");
  await user.click(scanButton());
  expect(await screen.findByText("scan screen")).toBeInTheDocument();
  const [scan] = (await api.listScans({})).items;
  expect((await api.getScan(scan?.id ?? "")).options).toMatchObject({
    browser: { kind: "local" },
    proxy: { kind: "system" },
  });
});

test("a Browserbase location needs its country, is labelled for what it is, and shows what the proxy will cost", async () => {
  const user = userEvent.setup();
  await api.setBrowserbaseCredentials("bb_live_0123456789abcdef");
  const { router } = await renderWithApp(<ScanComposer settings={browserbaseSettingsFixture} />);
  await user.type(urlInput(), "example.com");
  await user.click(screen.getByRole("button", { name: "Options" }));
  await user.click(origin().getByRole("radio", { name: "Browserbase" }));
  await user.click(visit().getByRole("radio", { name: "Browse like a visitor" }));
  await chooseOption(user, "Region", /Frankfurt/);
  await chooseOption(user, "Location", "Browserbase location…");
  expect(summaryText()).toMatch(/· Browserbase location not set ·/);

  await user.click(scanButton());
  expect(await origin().findByRole("alert")).toHaveTextContent(
    "Enter the country as a two-letter code, for example BE.",
  );
  // The field that is wrong is the one marked and described, not the select above it.
  const alert = origin().getByRole("alert");
  expect(origin().getByLabelText("Country code")).toBeInvalid();
  expect(origin().getByLabelText("Country code")).toHaveAttribute("aria-describedby", alert.id);
  expect(origin().getByLabelText("State code, optional")).toBeValid();
  expect(origin().getByRole("combobox", { name: "Location" })).not.toHaveAttribute("aria-describedby");
  expect((await api.listScans({})).total).toBe(0);

  await user.type(origin().getByLabelText("Country code"), "us");
  await user.type(origin().getByLabelText("State code, optional"), "n");
  await user.click(scanButton());
  expect(await origin().findByRole("alert")).toHaveTextContent("Enter the state as a two-letter code, for example NY.");
  expect(origin().getByLabelText("State code, optional")).toBeInvalid();
  expect(origin().getByLabelText("State code, optional")).toHaveAttribute(
    "aria-describedby",
    origin().getByRole("alert").id,
  );
  expect(origin().getByLabelText("Country code")).toBeValid();
  await user.clear(origin().getByLabelText("Country code"));
  await user.clear(origin().getByLabelText("State code, optional"));

  await user.type(origin().getByLabelText("Country code"), "be");
  expect(origin().queryByRole("alert")).toBeNull();
  // Three pages of 18 MB each at $12 per GB.
  expect(
    origin().getByText(
      "A residential connection; the country is best effort. About 54 MB go through it, an estimated $0.65 at list price.",
    ),
  ).toBeInTheDocument();
  expect(summaryText()).toBe(
    "Browse like a visitor · 3 pages · Browserbase, Frankfurt (eu-central-1) · Belgium (Browserbase proxy) · AI on, about $0.37",
  );
  expect(analysis().getByRole("note", { name: "Browser cost" })).toHaveTextContent(
    "about 2 browser minutes and 54 MB of proxy traffic. An estimated $0.65 at Browserbase's list price of $0.12 per browser hour and $12 per GB (2026-11-12); your plan's included usage is not counted.",
  );
  // The LLM figure is the one a three-page visit has on this Mac: no Browserbase amount is ever added into it.
  expect(analysis().getByText(/hard cap/)).toHaveTextContent(
    "Estimated $0.37 with claude-sonnet-5-5 for 3 pages, hard cap $0.50 per scan.",
  );

  await user.click(scanButton());
  expect(await screen.findByText("scan screen")).toBeInTheDocument();
  const [scan] = (await api.listScans({})).items;
  expect(router.state.location.pathname).toBe(`/scans/${scan?.id}`);
  expect((await api.getScan(scan?.id ?? "")).options).toMatchObject({
    browser: { kind: "browserbase", region: "eu-central-1" },
    proxy: { kind: "browserbaseLocation", country: "BE" },
    headless: true,
    visitor: "browse",
    extraPages: 2,
  });
});

test.each([
  "localhost:3000/article",
  "http://127.0.0.1:47615/demo-publisher/",
  "192.168.1.20",
  "http://printer.local/",
])("%s cannot be reached by a cloud browser: the choice is disabled with a plain reason", async (url) => {
  const user = userEvent.setup();
  const cloudDefault = { ...browserbaseSettingsFixture, defaultBrowser: "browserbase" as const };
  await renderWithApp(<ScanComposer settings={cloudDefault} />);
  await user.click(screen.getByRole("button", { name: "Options" }));
  // Browserbase is the default, and chosen until the address says otherwise.
  expect(origin().getByRole("radio", { name: "Browserbase" })).toBeChecked();

  await user.type(urlInput(), url);
  expect(origin().getByRole("radio", { name: "This Mac" })).toBeChecked();
  const browserbase = origin().getByRole("radio", { name: "Browserbase" });
  expect(browserbase).toBeDisabled();
  expect(browserbase).toHaveAccessibleDescription(
    "This address is on this Mac or your network; a cloud browser cannot reach it.",
  );
  expect(summaryText()).toBe("Scroll · browser hidden · system proxy · AI on, about $0.36");
  expect(visit().getByRole("switch", { name: "Show the browser" })).toBeInTheDocument();

  // A public address brings the choice back as it was.
  await user.clear(urlInput());
  await user.type(urlInput(), "example.com");
  expect(origin().getByRole("radio", { name: "Browserbase" })).toBeChecked();
});

test("a local address is scanned on this Mac even when Browserbase is the default", async () => {
  const user = userEvent.setup();
  await api.setBrowserbaseCredentials("bb_live_0123456789abcdef");
  await renderWithApp(<ScanComposer settings={{ ...browserbaseSettingsFixture, defaultBrowser: "browserbase" }} />);
  await user.type(urlInput(), "localhost:3000/article{Enter}");
  expect(await screen.findByText("scan screen")).toBeInTheDocument();
  const [scan] = (await api.listScans({})).items;
  expect((await api.getScan(scan?.id ?? "")).options.browser).toEqual({ kind: "local" });
});

test("“I'll browse myself” stays available on Browserbase and says how: the live view, billed while open", async () => {
  const user = userEvent.setup();
  await renderWithApp(<ScanComposer settings={browserbaseSettingsFixture} />);
  await user.type(urlInput(), "example.com");
  await user.click(screen.getByRole("button", { name: "Options" }));
  await user.click(origin().getByRole("radio", { name: "Browserbase" }));
  await user.click(visit().getByRole("radio", { name: "I'll browse myself" }));

  expect(
    visit().getByText("You use the site in Browserbase's live view while it records. Nothing is clicked for you."),
  ).toBeInTheDocument();
  expect(
    visit().getByText(/Browserbase's live view opens in your own browser; browse the site, then press/),
  ).toBeInTheDocument();
  expect(visit().getByText("In the live view")).toBeInTheDocument();
  expect(visit().getByText("Browserbase bills the session for as long as it is open.")).toBeInTheDocument();
  expect(analysis().getByRole("note", { name: "Browser cost" })).toHaveTextContent(
    "up to 15 browser minutes, no proxy traffic. An estimated $0.03",
  );
  expect(summaryText()).toBe("I'll browse myself · Browserbase, Oregon (us-west-2) · no proxy · AI on, about $0.36");
});

test("the profile control follows the Browserbase switch, not the one for this Mac", async () => {
  const user = userEvent.setup();
  const settings = {
    ...browserbaseSettingsFixture,
    persistentProfile: false,
    browserbase: { ...browserbaseSettingsFixture.browserbase, persistentProfile: true },
  };
  await renderWithApp(<ScanComposer settings={settings} />);
  await user.type(urlInput(), "example.com");
  await user.click(screen.getByRole("button", { name: "Options" }));
  expect(origin().getByRole("combobox", { name: "Browser profile" })).toHaveTextContent(
    "Settings default (fresh each scan)",
  );
  await user.click(origin().getByRole("radio", { name: "Browserbase" }));
  expect(origin().getByRole("combobox", { name: "Browser profile" })).toHaveTextContent(
    "Settings default (kept between scans)",
  );
  expect(origin().getByText(/Cookies and consent carry over, in a profile kept at Browserbase\./)).toBeInTheDocument();
  expect(summaryText()).toMatch(/· kept browser profile ·/);
});

test("a kept profile that Browserbase does not allow reads as what happens instead, and comes back on this Mac", async () => {
  const user = userEvent.setup();
  await renderWithApp(<ScanComposer settings={browserbaseSettingsFixture} />);
  await user.type(urlInput(), "example.com");
  await user.click(screen.getByRole("button", { name: "Options" }));
  await chooseOption(user, "Browser profile", "Kept between scans");
  expect(summaryText()).toMatch(/· kept browser profile ·/);

  // The Browserbase switch is off: the control must not say "Kept" above help that says "Starts fresh".
  await user.click(origin().getByRole("radio", { name: "Browserbase" }));
  expect(origin().getByRole("combobox", { name: "Browser profile" })).toHaveTextContent("Fresh each scan");
  expect(origin().getByText(/Starts fresh: keeping a profile at Browserbase is off\./)).toBeInTheDocument();
  expect(summaryText()).not.toMatch(/kept browser profile/);

  await user.click(origin().getByRole("radio", { name: "This Mac" }));
  expect(origin().getByRole("combobox", { name: "Browser profile" })).toHaveTextContent("Kept between scans");
});

test("a configuration that names Browserbase after its key was removed cannot run until it is changed", async () => {
  const user = userEvent.setup();
  const configuration = makeConfiguration({
    name: "Example on Browserbase",
    options: {
      ...validOptions,
      headless: true,
      browser: { kind: "browserbase", region: "eu-central-1" },
      proxy: { kind: "browserbaseLocation", country: "BE" },
    },
  });
  await renderWithApp(<ScanComposer settings={settingsFixture} configuration={configuration} />);

  expect(
    screen.getByText(
      "This configuration cannot run until Browserbase is set up again, or This Mac is chosen under Origin.",
    ),
  ).toBeInTheDocument();
  expect(summaryText()).toMatch(/· Browserbase not set up ·/);
  // The choice is still shown, so that it can be seen and changed.
  expect(origin().getByRole("radio", { name: "Browserbase" })).toBeChecked();
  expect(origin().getByRole("alert")).toHaveTextContent(
    "Browserbase is not set up any more. Set it up in Settings, or choose This Mac.",
  );
  await user.click(scanButton());
  expect((await api.listScans({})).total).toBe(0);

  await user.click(origin().getByRole("radio", { name: "This Mac" }));
  expect(origin().queryByRole("alert")).toBeNull();
  expect(origin().getByRole("radio", { name: "Browserbase" })).toBeDisabled();
  // The Browserbase location cannot come along to this Mac: the proxy is left to the settings.
  expect(origin().getByRole("combobox", { name: "Proxy" })).toHaveTextContent("Settings default (System proxy)");
  expect(screen.getByText("Edited")).toBeInTheDocument();
  await user.click(scanButton());
  expect(await screen.findByText("scan screen")).toBeInTheDocument();
});

test.each([
  ["browserbaseAuth", "Browserbase is not set up. Add the API key in Settings.", "Browserbase is not ready", true],
  [
    "browserbaseLimit",
    "Browserbase is at its limit of browsers running at once (3 of 3, shared by everything that uses this project). Try again when one has finished.",
    "Browserbase is at its limit",
    false,
  ],
  [
    "browserbaseFailed",
    "Browserbase could not start a browser in eu-central-1 right now (HTTP 503).",
    "Browserbase could not run the browser",
    false,
  ],
  [
    "invalidInput",
    "This address is on this Mac or your network; a cloud browser cannot reach it. Scan it with this Mac's Chrome.",
    "Could not start the scan",
    false,
  ],
] as const)(
  "a start refused with %s shows the backend's sentence under a plain title",
  async (code, message, title, settingsAction) => {
    const user = userEvent.setup();
    const startScan = vi.spyOn(api, "startScan").mockRejectedValue({ code, message });
    await renderWithApp(<ScanComposer settings={browserbaseSettingsFixture} />);
    await user.type(urlInput(), "example.com{Enter}");
    // Toasts of earlier scans may still be on screen: this one is found by its title.
    const toast = within((await screen.findByText(title)).closest("li") as HTMLElement);
    expect(toast.getByText(message)).toBeInTheDocument();
    expect(toast.queryByRole("button", { name: "Open settings" }) !== null).toBe(settingsAction);
    startScan.mockRestore();
  },
);
