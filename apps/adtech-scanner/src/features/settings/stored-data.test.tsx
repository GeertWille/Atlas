import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, test, vi } from "vitest";
import { api } from "@/lib/api";
import { useBrowser, useSettings } from "@/lib/query/hooks";
import { renderWithApp } from "@/test/render";
import { SettingsScreen } from "./SettingsScreen";

// Runs against the real api module: outside Tauri that is the fake backend, here with its seeded history.
vi.hoisted(() => {
  localStorage.setItem("adscan.fake.profile", "seeded");
});

function ConnectedSettings() {
  const settings = useSettings();
  const browser = useBrowser();
  return settings.data ? <SettingsScreen settings={settings.data} browser={browser.data} /> : null;
}

const section = async () =>
  (await screen.findByRole("heading", { name: "Stored scan data" })).closest("section") as HTMLElement;

beforeEach(async () => {
  await api.updateSettings({ keepNetworkData: true, storeFullHeaders: false, archiveLimitBytes: 5 * 1024 ** 3 });
});

test("network data is kept by default, with cookie and authorization values redacted", async () => {
  await renderWithApp(<ConnectedSettings />);
  const stored = within(await section());
  expect(stored.getByRole("switch", { name: "Keep network data with reports" })).toBeChecked();
  expect(stored.getByRole("switch", { name: "Store full headers" })).not.toBeChecked();
  expect(stored.getByRole("combobox", { name: "Storage limit" })).toHaveTextContent("5 GB");
});

test("the text says what is stored and what the redaction does not cover", async () => {
  await renderWithApp(<ConnectedSettings />);
  const stored = await section();
  expect(stored).toHaveTextContent("every request with its address and headers");
  expect(stored).toHaveTextContent("bid requests and responses included");
  expect(stored).toHaveTextContent("Images, video, audio and fonts are listed but their content is not stored");
  expect(stored).toHaveTextContent("can contain details of that account");
  // What the switch changes, what is replaced whatever it says, and what is never touched.
  expect(stored).toHaveTextContent(
    "Cookie, Set-Cookie and Authorization headers, and of every other header with a credential name",
  );
  expect(stored).toHaveTextContent("On, new scans keep those headers as sent");
  expect(stored).toHaveTextContent(
    "Replaced whatever this switch says: the Proxy-Authorization header, parameters with a credential name in every stored address",
  );
  expect(stored).toHaveTextContent("fields with such a name in request bodies and JSON responses");
  expect(stored).toHaveTextContent("Everything else is kept exactly as captured and can hold ids and tokens too");
  expect(stored).toHaveTextContent("consent strings such as gdpr_consent");
});

test("switching recording off also disables the header choice; both are saved", async () => {
  const user = userEvent.setup();
  await renderWithApp(<ConnectedSettings />);
  const stored = within(await section());
  await user.click(stored.getByRole("switch", { name: "Store full headers" }));
  await waitFor(async () => expect((await api.getSettings()).storeFullHeaders).toBe(true));
  await user.click(stored.getByRole("switch", { name: "Keep network data with reports" }));
  await waitFor(async () => expect((await api.getSettings()).keepNetworkData).toBe(false));
  expect(stored.getByRole("switch", { name: "Store full headers" })).toBeDisabled();
});

test("usage is shown, and clearing asks first and leaves the reports", async () => {
  const user = userEvent.setup();
  await renderWithApp(<ConnectedSettings />);
  const stored = within(await section());
  const meter = await stored.findByRole("meter", { name: "Storage used by scan data" });
  expect(meter).toHaveAttribute("aria-valuetext", expect.stringMatching(/ MB of 5 GB$/));
  expect(stored.getByText(/in 4 scans/)).toHaveTextContent(
    "1 scan no longer has its network data: it was cleared here, or removed to stay under the limit.",
  );

  await user.click(stored.getByRole("button", { name: "Clear stored scan data…" }));
  const dialog = await screen.findByRole("alertdialog", { name: "Clear stored scan data?" });
  expect(dialog).toHaveTextContent("The reports stay, with their findings and audit trails");
  await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
  expect((await api.getArchiveStorage()).scans).toBe(4);

  await user.click(stored.getByRole("button", { name: "Clear stored scan data…" }));
  await user.click(within(await screen.findByRole("alertdialog")).getByRole("button", { name: "Clear scan data" }));
  expect(await screen.findByText("Stored scan data cleared")).toBeInTheDocument();
  await waitFor(() => expect(stored.getByText(/in 0 scans/)).toBeInTheDocument());
  expect(stored.getByRole("button", { name: "Clear stored scan data…" })).toBeDisabled();
  expect((await api.getScan("seed-sportsdesk-desktop")).findings.length).toBeGreaterThan(0);
});
