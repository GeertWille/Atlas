import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, test, vi } from "vitest";
import { api } from "@/lib/api";
import { useBrowser, useSettings } from "@/lib/query/hooks";
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

const SCAN = {
  url: "https://www.example.com/",
  device: "desktop",
  observeSeconds: 5,
  autoScroll: true,
  acceptConsent: true,
  aiEnabled: false,
} as const;

beforeEach(async () => {
  await api.deleteAllScans();
  for (const configuration of await api.listScanConfigurations()) await api.deleteScanConfiguration(configuration.id);
  for (const proxy of (await api.getSettings()).proxies) await api.deleteProxyProfile(proxy.id);
  await api.updateSettings({
    defaultVisitor: "scroll",
    defaultExtraPages: 2,
    persistentProfile: false,
    defaultProxy: { kind: "system" },
  });
  if ((await api.getScanProfile()).exists) await api.clearScanProfile();
});

const section = (name: string) => screen.findByRole("region", { name });
const dialog = () => screen.findByRole("dialog");

test("scan defaults: the visitor behaviour and the pages of a browse visit are saved", async () => {
  const user = userEvent.setup();
  await renderWithApp(<ConnectedSettings />);
  const defaults = await section("Scan defaults");

  await user.click(within(defaults).getByRole("combobox", { name: "Default visitor" }));
  expect((await screen.findAllByRole("option")).map((option) => option.textContent)).toEqual([
    "Observe only",
    "Scroll",
    "Browse like a visitor",
    "I'll browse myself",
  ]);
  await user.click(screen.getByRole("option", { name: "Browse like a visitor" }));
  await waitFor(async () => expect((await api.getSettings()).defaultVisitor).toBe("browse"));
  expect(within(defaults).getByText(/Scripted visits are automated and say so to the page/)).toBeInTheDocument();

  expect(within(defaults).getByText("3 pages")).toBeInTheDocument();
  await user.click(within(defaults).getByRole("button", { name: "More pages" }));
  await waitFor(async () => expect((await api.getSettings()).defaultExtraPages).toBe(3));
  expect(await within(defaults).findByText("4 pages")).toBeInTheDocument();
  for (let press = 0; press < 3; press += 1)
    await user.click(within(defaults).getByRole("button", { name: "Fewer pages" }));
  expect(await within(defaults).findByText("1 page")).toBeInTheDocument();
  expect(within(defaults).getByRole("button", { name: "Fewer pages" })).toBeDisabled();
  await waitFor(async () => expect((await api.getSettings()).defaultExtraPages).toBe(0));
});

test("the browser profile is explained, switched on, shown with its size, and cleared after confirming", async () => {
  const user = userEvent.setup();
  const { queryClient } = await renderWithApp(<ConnectedSettings />);
  const browser = await section("Browser");
  const keep = within(browser).getByRole("switch", { name: "Keep the browser profile between scans" });
  expect(keep).not.toBeChecked();
  expect(keep).toHaveAccessibleDescription(
    "Cookies and consent persist like a returning visitor's, so a banner accepted once does not come back. Off: every scan starts with a fresh profile. Stored only on this Mac, and separate from your own Chrome.",
  );
  expect(await within(browser).findByText("No browser profile is stored yet.")).toBeInTheDocument();

  await user.click(keep);
  await waitFor(async () => expect((await api.getSettings()).persistentProfile).toBe(true));
  const scan = await api.startScan(SCAN);
  await waitFor(async () => expect((await api.getScan(scan.id)).summary.status).toBe("completed"), { timeout: 5000 });
  // In the app the scan's own events refresh the profile; this render has no event subscription.
  await queryClient.invalidateQueries();
  await waitFor(() => expect(within(browser).getByText("20.4 MB")).toBeInTheDocument());
  expect(within(browser).getByText(/on this Mac, last used just now/)).toBeInTheDocument();

  await user.click(within(browser).getByRole("button", { name: "Clear browser profile…" }));
  const confirm = await screen.findByRole("alertdialog", { name: "Clear the browser profile?" });
  expect(confirm).toHaveTextContent("Your own Chrome is not touched.");
  await user.click(within(confirm).getByRole("button", { name: "Clear profile" }));

  expect(await within(browser).findByText("No browser profile is stored yet.")).toBeInTheDocument();
  expect(await screen.findByText("Browser profile cleared")).toBeInTheDocument();
  expect((await api.getScanProfile()).exists).toBe(false);
});

test("the browser profile cannot be cleared while a scan is using it", async () => {
  localStorage.setItem("adscan.fake.speed", "5000");
  await api.updateSettings({ persistentProfile: true });
  // A manual session holds the profile until it is finished.
  const scan = await api.startScan({ ...SCAN, visitor: "manual" });
  await renderWithApp(<ConnectedSettings />);
  const browser = await section("Browser");

  expect(await within(browser).findByText(/A running scan is using it\./)).toBeInTheDocument();
  expect(within(browser).getByRole("button", { name: "Clear browser profile…" })).toBeDisabled();
  await api.cancelScan(scan.id);
});

test("a proxy is added with its password, listed without it, edited and deleted after confirming", async () => {
  const user = userEvent.setup();
  await renderWithApp(<ConnectedSettings />);
  const proxies = await section("Proxies");
  expect(
    within(proxies).getByText(/Behind a proxy the app cannot fully verify where a request lands/),
  ).toBeInTheDocument();
  expect(within(proxies).queryByRole("list", { name: "Saved proxies" })).toBeNull();

  await user.click(within(proxies).getByRole("button", { name: "Add proxy…" }));
  let form = await dialog();
  await user.click(within(form).getByRole("button", { name: "Add proxy" }));
  expect(within(form).getByLabelText("Name")).toBeInvalid();
  expect(within(form).getByLabelText("Host")).toHaveAccessibleDescription("Enter the proxy's host name or IP address.");
  expect((await api.getSettings()).proxies).toEqual([]);

  await user.type(within(form).getByLabelText("Name"), "Belgium");
  await user.type(within(form).getByLabelText("Host"), "be.proxy.example");
  await user.type(within(form).getByLabelText("User name (optional)"), "scanner");
  const password = within(form).getByLabelText("Password (optional)");
  expect(password).toHaveAttribute("type", "password");
  await user.type(password, "s3cret");
  await user.click(within(form).getByRole("button", { name: "Add proxy" }));

  const row = await within(proxies).findByRole("listitem");
  expect(row).toHaveTextContent("Belgium");
  expect(row).toHaveTextContent("http://be.proxy.example:8080");
  expect(row).toHaveTextContent("Password stored");
  expect(document.body).not.toHaveTextContent("s3cret");
  expect((await api.getSettings()).proxies).toMatchObject([
    { name: "Belgium", username: "scanner", hasPassword: true },
  ]);

  // Edit: the password field is empty, the stored one can be removed, and the port can change.
  await user.click(within(row).getByRole("button", { name: "Edit Belgium" }));
  form = await dialog();
  expect(within(form).getByLabelText("Password (optional)")).toHaveValue("");
  await user.click(within(form).getByRole("button", { name: "Remove password" }));
  await waitFor(() => expect(within(form).queryByRole("button", { name: "Remove password" })).toBeNull());
  await user.clear(within(form).getByLabelText("Port"));
  await user.type(within(form).getByLabelText("Port"), "3128");
  await user.click(within(form).getByRole("button", { name: "Save proxy" }));
  await waitFor(() => expect(within(proxies).getByRole("listitem")).toHaveTextContent("http://be.proxy.example:3128"));
  expect(within(proxies).getByRole("listitem")).not.toHaveTextContent("Password stored");

  // It can be made the default for scans, and deleting it puts the default back.
  await user.click(within(proxies).getByRole("combobox", { name: "Default proxy" }));
  await user.click(await screen.findByRole("option", { name: "Belgium" }));
  await waitFor(async () => expect((await api.getSettings()).defaultProxy.kind).toBe("profile"));

  await user.click(within(proxies).getByRole("button", { name: "Delete Belgium" }));
  const confirm = await screen.findByRole("alertdialog", { name: "Delete the proxy “Belgium”?" });
  await user.click(within(confirm).getByRole("button", { name: "Delete" }));
  await waitFor(() => expect(within(proxies).queryByRole("listitem")).toBeNull());
  expect(await api.getSettings()).toMatchObject({ proxies: [], defaultProxy: { kind: "system" } });
});

test("when the password cannot be stored, trying again finishes the same proxy instead of adding a second", async () => {
  const user = userEvent.setup();
  await renderWithApp(<ConnectedSettings />);
  const proxies = await section("Proxies");
  await user.click(within(proxies).getByRole("button", { name: "Add proxy…" }));
  const add = await dialog();
  await user.type(within(add).getByLabelText("Name"), "Belgium");
  await user.type(within(add).getByLabelText("Host"), "be.proxy.example");
  await user.type(within(add).getByLabelText("User name (optional)"), "scanner");
  await user.type(within(add).getByLabelText("Password (optional)"), "s3cret");

  const keychain = vi
    .spyOn(api, "setProxyPassword")
    .mockRejectedValueOnce({ code: "internal", message: "The keychain is locked" });
  await user.click(within(add).getByRole("button", { name: "Add proxy" }));
  expect(await within(add).findByText("The keychain is locked")).toBeInTheDocument();
  // The proxy itself was saved; only its password is missing.
  expect((await api.getSettings()).proxies).toMatchObject([{ name: "Belgium", hasPassword: false }]);

  await user.click(within(add).getByRole("button", { name: "Add proxy" }));
  await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  expect((await api.getSettings()).proxies).toMatchObject([{ name: "Belgium", hasPassword: true }]);
  keychain.mockRestore();
});

test("SOCKS5 with a user name is refused in the dialog, with the reason", async () => {
  const user = userEvent.setup();
  await renderWithApp(<ConnectedSettings />);
  await user.click(within(await section("Proxies")).getByRole("button", { name: "Add proxy…" }));
  const form = await dialog();
  await user.type(within(form).getByLabelText("Name"), "Tunnel");
  await user.type(within(form).getByLabelText("Host"), "socks.example");
  await user.type(within(form).getByLabelText("User name (optional)"), "me");

  await user.click(within(form).getByRole("combobox", { name: "Type" }));
  await user.click(await screen.findByRole("option", { name: "SOCKS5" }));
  // The port follows the type.
  expect(within(form).getByLabelText("Port")).toHaveValue("1080");
  await user.click(within(form).getByRole("button", { name: "Add proxy" }));

  expect(within(form).getByLabelText("User name (optional)")).toHaveAccessibleDescription(
    "Chrome cannot sign in to a SOCKS5 proxy. Remove the user name, or use an HTTP or HTTPS proxy.",
  );
  expect((await api.getSettings()).proxies).toEqual([]);

  await user.clear(within(form).getByLabelText("User name (optional)"));
  await user.click(within(form).getByRole("button", { name: "Add proxy" }));
  await waitFor(async () =>
    expect((await api.getSettings()).proxies).toMatchObject([{ name: "Tunnel", scheme: "socks5" }]),
  );
});

test("a proxy that a configuration uses is not deleted, and the reason names the configuration", async () => {
  const user = userEvent.setup();
  const settings = await api.saveProxyProfile({
    name: "Belgium",
    scheme: "http",
    host: "be.proxy.example",
    port: 8080,
    username: null,
  });
  const id = settings.proxies[0]?.id ?? "";
  await api.saveScanConfiguration({
    name: "Example from Belgium",
    options: { ...SCAN, proxy: { kind: "profile", id } },
  });
  await renderWithApp(<ConnectedSettings />);
  const proxies = await section("Proxies");

  await user.click(await within(proxies).findByRole("button", { name: "Delete Belgium" }));
  await user.click(within(await screen.findByRole("alertdialog")).getByRole("button", { name: "Delete" }));

  expect(await screen.findByText("Could not delete the proxy")).toBeInTheDocument();
  expect(screen.getByText(/is used by the scan configuration "Example from Belgium"/)).toBeInTheDocument();
  expect(within(proxies).getByRole("listitem")).toHaveTextContent("Belgium");
});
