import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, test, vi } from "vitest";
import type { ScanOptions } from "@/bindings";
import { api } from "@/lib/api";
import { useScanConfigurations, useSettings } from "@/lib/query/hooks";
import { makeConfiguration, settingsFixture } from "@/test/fixtures";
import { renderWithApp } from "@/test/render";
import { ConfigurationsPanel } from "./ConfigurationsPanel";
import { ConfigurationsScreen } from "./ConfigurationsScreen";

// Runs against the real api module: outside Tauri that is the fake backend.
vi.hoisted(() => {
  localStorage.setItem("adscan.fake.profile", "fresh");
  localStorage.setItem("adscan.fake.speed", "5000");
});

const options: ScanOptions = {
  url: "https://www.example.com/news",
  device: "mobile",
  observeSeconds: 5,
  autoScroll: true,
  acceptConsent: true,
  aiEnabled: false,
  visitor: "browse",
  extraPages: 2,
};

const onNew = vi.fn();

function Connected({ panel = false }: { panel?: boolean }) {
  const settings = useSettings();
  const configurations = useScanConfigurations();
  if (!settings.data || !configurations.data) return null;
  if (panel) return <ConfigurationsPanel configurations={configurations.data} settings={settings.data} />;
  return <ConfigurationsScreen configurations={configurations.data} settings={settings.data} onNew={onNew} />;
}

beforeEach(async () => {
  onNew.mockClear();
  await api.deleteAllScans();
  for (const configuration of await api.listScanConfigurations()) await api.deleteScanConfiguration(configuration.id);
  for (const proxy of (await api.getSettings()).proxies) await api.deleteProxyProfile(proxy.id);
});

// A row's name starts with the configuration's name, followed by the page it scans: the path under its
// site's heading, the whole address in the home panel.
const rowOf = (name: string) => screen.findByRole("row", { name: new RegExp(`^${name} ?(/|www\\.)`) });
const choose = async (user: ReturnType<typeof userEvent.setup>, name: string, action: string) => {
  await user.click(within(await rowOf(name)).getByRole("button", { name: `Actions for ${name}` }));
  await user.click(await screen.findByRole("menuitem", { name: action }));
};

test("without configurations the screen explains what they are for and where to make one", async () => {
  await renderWithApp(<Connected />);
  expect(await screen.findByText("No configurations yet")).toBeInTheDocument();
  expect(screen.getByText(/the same site from Belgium on mobile and from Germany on desktop/)).toBeInTheDocument();
  expect(screen.getByText(/Set up a scan on Home and choose “Save as configuration…”/)).toBeInTheDocument();
  // Nothing to search or sort yet.
  expect(screen.queryByRole("searchbox")).toBeNull();
  await userEvent.setup().click(screen.getByRole("button", { name: "Set up a scan" }));
  expect(onNew).toHaveBeenCalledOnce();
});

test("a configuration is listed with its site, setup and last run, and one click runs it", async () => {
  const user = userEvent.setup();
  const proxyId = (
    await api.saveProxyProfile({
      name: "Belgium",
      scheme: "http",
      host: "be.proxy.example",
      port: 8080,
      username: null,
    })
  ).proxies[0]?.id as string;
  await api.saveScanConfiguration({
    name: "Example from Belgium",
    options: { ...options, proxy: { kind: "profile", id: proxyId } },
  });
  const { router } = await renderWithApp(<Connected />);

  const row = await rowOf("Example from Belgium");
  // The site heads its group; the row adds the page.
  expect(screen.getByRole("rowheader", { name: "example.com 1 configuration" })).toBeInTheDocument();
  expect(row).toHaveTextContent("/news");
  expect(within(row).getByRole("link", { name: "Example from Belgium" })).toHaveAttribute(
    "href",
    expect.stringMatching(/^\/\?configuration=config-/),
  );
  expect(row).toHaveTextContent("Mobile");
  expect(row).toHaveTextContent("Belgium");
  expect(row).toHaveTextContent("Browse like a visitor, 3 pages");
  expect(row).toHaveTextContent("Never run");

  await user.click(within(row).getByRole("button", { name: "Run Example from Belgium" }));
  await screen.findByText("scan screen");
  const [scan] = (await api.listScans({})).items;
  expect(router.state.location.pathname).toBe(`/scans/${scan?.id}`);
  expect(scan).toMatchObject({
    device: "mobile",
    configuration: { name: "Example from Belgium" },
    proxy: { name: "Belgium" },
  });
});

test("the last run links to its scan with the health score once it has one", async () => {
  const saved = await api.saveScanConfiguration({ name: "Example", options });
  const scan = await api.runScanConfiguration(saved.id);
  await waitFor(async () => expect((await api.getScan(scan.id)).summary.status).toBe("completed"), { timeout: 5000 });
  await renderWithApp(<Connected />);

  const lastRun = await within(await rowOf("Example")).findByRole("link", { name: /Health score \d+ of 100/ });
  expect(lastRun).toHaveAttribute("href", `/scans/${scan.id}`);
  expect(lastRun).toHaveTextContent("just now");
});

test("a configuration whose proxy was deleted states the problem and cannot be run", async () => {
  const broken = makeConfiguration({
    name: "Example from Germany",
    options: { ...options, proxy: { kind: "profile", id: "proxy-de", name: "Germany", host: "de.proxy.example" } },
  });
  await renderWithApp(
    <ConfigurationsScreen
      configurations={[broken, makeConfiguration({ id: "config-2", name: "Example direct" })]}
      settings={settingsFixture}
      onNew={onNew}
    />,
  );

  const row = await rowOf("Example from Germany");
  expect(row).toHaveTextContent("The proxy “Germany” was deleted. Edit the configuration and choose another proxy.");
  expect(row).toHaveTextContent("Germany (deleted)");
  expect(within(row).getByRole("button", { name: "Run Example from Germany" })).toBeDisabled();
  // The way to fix it stays open.
  await userEvent.setup().click(within(row).getByRole("button", { name: "Actions for Example from Germany" }));
  expect(await screen.findByRole("menuitem", { name: "Edit" })).toHaveAttribute("href", "/?configuration=config-1");
  await userEvent.setup().keyboard("{Escape}");
  expect(within(await rowOf("Example direct")).getByRole("button", { name: "Run Example direct" })).toBeEnabled();
});

test("edit opens the composer with the configuration; duplicate, rename and delete act on the list", async () => {
  const user = userEvent.setup();
  const saved = await api.saveScanConfiguration({ name: "Example", options });
  const scan = await api.runScanConfiguration(saved.id);
  await renderWithApp(<Connected />);

  await user.click(within(await rowOf("Example")).getByRole("button", { name: "Actions for Example" }));
  expect(await screen.findByRole("menuitem", { name: "Edit" })).toHaveAttribute("href", `/?configuration=${saved.id}`);
  await user.click(screen.getByRole("menuitem", { name: "Duplicate" }));
  expect(await rowOf("Example copy")).toHaveTextContent("Never run");
  await choose(user, "Example", "Duplicate");
  expect(await rowOf("Example copy 2")).toBeInTheDocument();
  // Three in all, as the line under the list says (the site heading counts its own).
  expect(await screen.findByText("3 configurations", { selector: "p" })).toBeInTheDocument();

  await choose(user, "Example copy 2", "Rename…");
  const rename = await screen.findByRole("dialog", { name: "Rename configuration" });
  const name = within(rename).getByLabelText("Name");
  expect(name).toHaveValue("Example copy 2");
  await user.clear(name);
  await user.type(name, "example COPY");
  await user.click(within(rename).getByRole("button", { name: "Rename" }));
  expect(await within(rename).findByRole("alert")).toHaveTextContent(
    'A scan configuration named "example COPY" already exists',
  );
  await user.clear(name);
  await user.type(name, "Example on a phone{Enter}");
  expect(await rowOf("Example on a phone")).toBeInTheDocument();
  expect(screen.queryByRole("row", { name: /Example copy 2/ })).toBeNull();

  await choose(user, "Example", "Delete…");
  const confirm = await screen.findByRole("alertdialog", { name: "Delete “Example”?" });
  expect(confirm).toHaveTextContent("Scans that were run from it stay in history.");
  await user.click(within(confirm).getByRole("button", { name: "Delete" }));
  await waitFor(() => expect(screen.queryByRole("row", { name: /^Example ?\// })).toBeNull());
  expect((await api.listScanConfigurations()).map((configuration) => configuration.name)).toEqual([
    "Example copy",
    "Example on a phone",
  ]);
  expect((await api.getScan(scan.id)).summary.configuration?.name).toBe("Example");
});

test("the home panel is absent without configurations, and lists the most recently run first", async () => {
  const { unmount } = await renderWithApp(<Connected panel />);
  await waitFor(async () => expect(await api.listScanConfigurations()).toEqual([]));
  expect(screen.queryByRole("region", { name: "Configurations" })).toBeNull();
  unmount();

  for (const name of ["Alpha", "Bravo", "Charlie", "Delta", "Echo"]) await api.saveScanConfiguration({ name, options });
  const echo = (await api.listScanConfigurations()).find((configuration) => configuration.name === "Echo");
  await api.runScanConfiguration(echo?.id ?? "");
  await renderWithApp(<Connected panel />);

  const panel = await screen.findByRole("region", { name: "Configurations" });
  const rows = within(panel).getAllByRole("row").slice(1);
  expect(rows).toHaveLength(4);
  expect(rows[0]).toHaveTextContent("Echo");
  // Flat, without site headings: every row says its whole address, and its name opens it in the composer.
  expect(within(panel).queryByRole("rowheader")).toBeNull();
  expect(rows[0]).toHaveTextContent("www.example.com/news");
  expect(within(rows[0] as HTMLElement).getByRole("link", { name: "Echo" })).toHaveAttribute(
    "href",
    `/?configuration=${echo?.id}`,
  );
  expect(within(panel).getByRole("link", { name: "View all 5" })).toHaveAttribute("href", "/configurations");
});

test("the list is grouped by site, searched by any word, and sorted by last run or by name", async () => {
  const user = userEvent.setup();
  const ledger = { ...options, url: "https://www.ledger.example/markets", device: "desktop" as const };
  await api.saveScanConfiguration({ name: "Zulu news", options });
  await api.saveScanConfiguration({ name: "Alpha news", options });
  await api.saveScanConfiguration({ name: "Ledger markets", options: ledger });
  const zulu = (await api.listScanConfigurations()).find((configuration) => configuration.name === "Zulu news");
  await api.runScanConfiguration(zulu?.id ?? "");
  await renderWithApp(<Connected />);
  await rowOf("Zulu news");

  const table = () => within(screen.getByRole("region", { name: "Configurations" }));
  const listed = () =>
    table()
      .getAllByRole("row")
      .slice(1)
      .map((row) => (within(row).queryByRole("rowheader") ?? within(row).getAllByRole("link")[0])?.textContent);
  // Recently run first: the site of the newest run leads, and its rows follow it.
  expect(listed()[0]).toBe("example.com 2 configurations");
  expect(listed()[1]).toBe("Zulu news");
  expect(listed()).toContain("ledger.example 1 configuration");
  expect(screen.getByText("3 configurations", { selector: "p" })).toBeInTheDocument();

  await user.click(screen.getByRole("combobox", { name: "Sort by" }));
  await user.click(await screen.findByRole("option", { name: "Name" }));
  expect(listed()).toEqual([
    "example.com 2 configurations",
    "Alpha news",
    "Zulu news",
    "ledger.example 1 configuration",
    "Ledger markets",
  ]);

  const search = screen.getByRole("searchbox", { name: "Search configurations" });
  await user.type(search, "ledger desktop");
  expect(listed()).toEqual(["ledger.example 1 configuration", "Ledger markets"]);
  expect(screen.getByText("1 of 3 configurations")).toBeInTheDocument();

  await user.clear(search);
  await user.type(search, "nothing like it");
  expect(screen.getByText("No configurations match")).toBeInTheDocument();
  expect(table().queryByRole("row")).toBeNull();
  await user.click(screen.getByRole("button", { name: "Show all configurations" }));
  expect(listed()).toHaveLength(5);

  await user.click(screen.getByRole("button", { name: "New configuration" }));
  expect(onNew).toHaveBeenCalledOnce();
});

test("the last run says how the health score moved against the run before", async () => {
  const saved = await api.saveScanConfiguration({ name: "Example", options });
  const done = async (scanId: string) =>
    waitFor(async () => expect((await api.getScan(scanId)).summary.status).toBe("completed"), { timeout: 5000 });
  const first = await api.runScanConfiguration(saved.id);
  await done(first.id);
  const second = await api.runScanConfiguration(saved.id);
  await done(second.id);
  const [before, after] = [await api.getScan(first.id), await api.getScan(second.id)].map(
    (detail) => detail.summary.healthScore ?? 0,
  );
  await renderWithApp(<Connected />);

  const lastRun = await within(await rowOf("Example")).findByRole("link", { name: /Health score \d+ of 100/ });
  expect(lastRun).toHaveAttribute("href", `/scans/${second.id}`);
  // The score, how it moved (nothing when it did not), and when.
  const change = (after ?? 0) - (before ?? 0);
  const moved = `${change > 0 ? `+${change}` : `−${-change}`}, ${change > 0 ? "up" : "down"} ${Math.abs(change)} from the run before,`;
  expect(lastRun.textContent).toBe(`${after}${change === 0 ? "" : moved}just now`);
});

test("a configuration that runs on Browserbase carries its mark, and is broken once the credentials are gone", async () => {
  const user = userEvent.setup();
  await api.setBrowserbaseCredentials("bb_live_0123456789abcdef");
  await api.saveScanConfiguration({
    name: "Example via Frankfurt",
    options: {
      ...options,
      headless: true,
      browser: { kind: "browserbase", region: "eu-central-1" },
      proxy: { kind: "browserbaseLocation", country: "BE" },
    },
  });
  const first = await renderWithApp(<Connected />);
  const row = within(await rowOf("Example via Frankfurt"));
  expect(row.getByTitle("Runs in a Browserbase Frankfurt browser")).toHaveTextContent("Browserbase Frankfurt");
  expect(row.getByText("Belgium (Browserbase proxy)")).toBeInTheDocument();
  expect(row.getByRole("button", { name: "Run Example via Frankfurt" })).toBeEnabled();

  // It is found by where it runs.
  await user.type(screen.getByRole("searchbox"), "browserbase frankfurt");
  expect(await rowOf("Example via Frankfurt")).toBeInTheDocument();
  first.unmount();

  await api.clearBrowserbaseCredentials();
  await renderWithApp(<Connected />);
  const broken = within(await rowOf("Example via Frankfurt"));
  expect(
    broken.getByText(
      "Browserbase is not set up any more. Add its key in Settings, or edit the configuration to run on this Mac.",
    ),
  ).toBeInTheDocument();
  expect(broken.getByRole("button", { name: "Run Example via Frankfurt" })).toBeDisabled();
});
