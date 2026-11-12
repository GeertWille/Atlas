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

function Connected({ panel = false }: { panel?: boolean }) {
  const settings = useSettings();
  const configurations = useScanConfigurations();
  if (!settings.data || !configurations.data) return null;
  const View = panel ? ConfigurationsPanel : ConfigurationsScreen;
  return <View configurations={configurations.data} settings={settings.data} />;
}

beforeEach(async () => {
  await api.deleteAllScans();
  for (const configuration of await api.listScanConfigurations()) await api.deleteScanConfiguration(configuration.id);
  for (const proxy of (await api.getSettings()).proxies) await api.deleteProxyProfile(proxy.id);
});

// A row's name starts with the configuration's name, followed by its site.
const rowOf = (name: string) => screen.findByRole("row", { name: new RegExp(`^${name} ?www\\.`) });
const choose = async (user: ReturnType<typeof userEvent.setup>, name: string, action: string) => {
  await user.click(within(await rowOf(name)).getByRole("button", { name: `Actions for ${name}` }));
  await user.click(await screen.findByRole("menuitem", { name: action }));
};

test("without configurations the screen explains what they are for and where to make one", async () => {
  await renderWithApp(<Connected />);
  expect(await screen.findByText("No configurations yet")).toBeInTheDocument();
  expect(screen.getByText(/the same site from Belgium on mobile and from Germany on desktop/)).toBeInTheDocument();
  expect(screen.getByRole("link", { name: "Set up a scan" })).toHaveAttribute("href", "/");
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
  expect(row).toHaveTextContent("www.example.com/news");
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
      configurations={[broken, makeConfiguration({ name: "Example direct" })]}
      settings={settingsFixture}
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
  expect(await screen.findByText("3 configurations")).toBeInTheDocument();

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
  await waitFor(() => expect(screen.queryByRole("row", { name: /^Example ?www\./ })).toBeNull());
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
  expect(within(panel).getByRole("link", { name: "View all 5" })).toHaveAttribute("href", "/configurations");
});
