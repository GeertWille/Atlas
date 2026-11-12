import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, test, vi } from "vitest";
import { api } from "@/lib/api";
import { makeConfiguration, makeProxy, settingsFixture } from "@/test/fixtures";
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
  });
});

test("without an API key the AI option is off and locked, and the scan runs deterministic checks only", async () => {
  const user = userEvent.setup();
  await renderWithApp(<ScanComposer settings={{ ...settingsFixture, hasApiKey: false }} />);

  expect(screen.getByText(/this scan runs the deterministic checks only/)).toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "Options" }));
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

  expect(screen.getByText(/Estimated/)).toHaveTextContent(
    "Estimated $0.001 for Jev script triage. Add an Anthropic key to analyse wrapper logic.",
  );
  expect(screen.getByRole("link", { name: "Add an Anthropic key" })).toHaveAttribute("href", "/settings");
  await user.click(screen.getByRole("button", { name: "Options" }));
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
  expect(screen.getByText(/this scan runs the deterministic checks only/)).toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "Options" }));
  expect(screen.getByRole("switch", { name: "AI logic analysis" })).toBeDisabled();
});

test("with both keys and Jev on, the cost line names both models", async () => {
  await renderWithApp(<ScanComposer settings={{ ...settingsFixture, hasTypesafeKey: true, jevEnabled: true }} />);
  expect(screen.getByText(/Estimated/)).toHaveTextContent(
    "Estimated $0.37 with claude-sonnet-5-5 and Jev script triage, hard cap $0.50 per scan.",
  );
});

test("the cost line reflects the settings and the AI switch", async () => {
  const user = userEvent.setup();
  await renderWithApp(<ScanComposer settings={{ ...settingsFixture, analysisEffort: "high" }} />);

  expect(screen.getByText(/Estimated/)).toHaveTextContent(
    "Estimated $0.50 with claude-sonnet-5-5, hard cap $0.50 per scan. Analysis may stop early at the cap.",
  );
  await user.click(screen.getByRole("button", { name: "Options" }));
  await user.click(screen.getByRole("switch", { name: "AI logic analysis" }));
  expect(screen.getByText("Deterministic checks only. No LLM cost.")).toBeInTheDocument();
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
  expect(screen.getByText("Scroll", { selector: "span" })).toBeInTheDocument();
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
  // A scripted visit never claims to be a person.
  expect(
    screen.getByText(/This visit is automated and says so to the page \(navigator\.webdriver is true\)/),
  ).toBeInTheDocument();
  // Scroll and observe have no page count; the settings run them headless.
  expect(screen.queryByRole("button", { name: "More pages" })).toBeNull();
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
  expect(screen.getByText("3 pages")).toBeInTheDocument();
  expect(
    screen.getByText(
      /The entry page, then up to 2 links on the same site\. Ads, sign-in and checkout links are never clicked\./,
    ),
  ).toBeInTheDocument();
  expect(screen.getByText(/Estimated/)).toHaveTextContent(
    "Estimated $0.37 with claude-sonnet-5-5 for 3 pages, hard cap $5.00 per scan.",
  );
  expect(screen.getByText("Browse like a visitor, 3 pages")).toBeInTheDocument();

  for (let press = 0; press < 3; press += 1) await user.click(screen.getByRole("button", { name: "More pages" }));
  expect(screen.getByText("6 pages")).toBeInTheDocument();
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
  const explanation = screen.getByText(/Chrome opens; browse the site, then press/);
  expect(explanation).toHaveTextContent(
    "Chrome opens; browse the site, then press Finish capture (up to 15 minutes). You drive the browser and handle the consent banner yourself. The page can still notice the debugger that records it.",
  );
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
  expect(
    screen.getByText(/A fresh profile for this scan: no cookies, like a first-time visitor\./),
  ).toBeInTheDocument();
  expect(screen.getByRole("link", { name: "Change the default" })).toHaveAttribute("href", "/settings");
  unmount();

  await renderWithApp(<ScanComposer settings={{ ...settingsFixture, persistentProfile: true }} />);
  await user.click(screen.getByRole("button", { name: "Options" }));
  expect(
    screen.getByText(/Kept between scans: cookies and consent persist, like a returning visitor's\./),
  ).toBeInTheDocument();
});

test("proxy, profile and budget stay out of the way until asked for, and then override the settings for this scan", async () => {
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
  await openOptions(user);
  expect(screen.queryByRole("combobox", { name: "Proxy" })).toBeNull();

  await user.click(screen.getByRole("button", { name: "Proxy, profile and budget" }));
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
  await user.click(await screen.findByRole("option", { name: "Kept scan profile" }));
  expect(screen.getByText(/Kept between scans/)).toBeInTheDocument();
  expect(screen.queryByRole("link", { name: "Change the default" })).toBeNull();

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
  expect(screen.getByText("2 pages")).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Update configuration" })).toBeNull();
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
  expect(
    screen.getByText("Scan runs these edited options once. Update the configuration to keep them."),
  ).toBeInTheDocument();
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

  // The advanced options are open, because that is where the problem is.
  const proxy = screen.getByRole("combobox", { name: "Proxy" });
  expect(proxy).toBeInvalid();
  expect(screen.getByText("This proxy was deleted. Choose another one.")).toBeInTheDocument();
  await user.click(scanButton());
  expect((await api.listScans({})).total).toBe(0);

  await user.click(proxy);
  await user.click(await screen.findByRole("option", { name: "No proxy" }));
  expect(screen.queryByText("This proxy was deleted. Choose another one.")).toBeNull();
  await user.click(scanButton());
  expect(await startedOptions()).toMatchObject({ proxy: { kind: "none" } });
});
