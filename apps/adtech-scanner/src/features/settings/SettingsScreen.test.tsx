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
});

const VALID_KEY = "sk-ant-api03-abcdefghijklmnopqrstuv";

function ConnectedSettings() {
  const settings = useSettings();
  const browser = useBrowser();
  return settings.data ? <SettingsScreen settings={settings.data} browser={browser.data} /> : null;
}

beforeEach(async () => {
  await api.clearApiKey();
  await api.clearApiKey("typesafe");
  await api.updateSettings({
    budgetUsd: 0.5,
    theme: "system",
    filterModel: "claude-haiku-5-5",
    chromePath: null,
    checkSellersJson: true,
  });
});

const keyInput = () => screen.findByLabelText("Anthropic API key");
const saveKey = () => screen.getByRole("button", { name: "Validate and save" });

test("API key: empty is caught locally, a rejected key shows the reason, a valid key is stored, and it can be removed", async () => {
  const user = userEvent.setup();
  await renderWithApp(<ConnectedSettings />);
  const input = await keyInput();
  expect(input).toHaveAttribute("type", "password");

  await user.click(saveKey());
  expect(await screen.findByRole("alert")).toHaveTextContent("Paste your API key.");

  await user.type(input, "definitely-not-a-key");
  expect(screen.queryByRole("alert")).toBeNull();
  await user.click(saveKey());
  expect(screen.getByRole("button", { name: "Validating…" })).toBeDisabled();
  expect(await screen.findByRole("alert")).toHaveTextContent("Anthropic rejected this key");
  expect(input).toBeInvalid();
  expect((await api.getSettings()).hasApiKey).toBe(false);

  await user.clear(input);
  await user.type(input, VALID_KEY);
  await user.click(saveKey());
  expect(await screen.findByText("A validated key is stored in your system keychain.")).toBeInTheDocument();
  expect(screen.queryByLabelText("Anthropic API key")).toBeNull();
  expect((await api.getSettings()).hasApiKey).toBe(true);

  await user.click(screen.getByRole("button", { name: "Remove key" }));
  const dialog = await screen.findByRole("alertdialog", { name: "Remove the API key?" });
  await user.click(within(dialog).getByRole("button", { name: "Remove key" }));
  expect(await keyInput()).toHaveValue("");
  expect((await api.getSettings()).hasApiKey).toBe(false);
});

const jevSwitch = () => screen.findByRole("switch", { name: "Classify scripts with Jev" });
const triageStatus = () => screen.getByText("Script triage").closest("[data-slot=field]");

test("TypeSafe key: a rejected key shows the reason; a stored key unlocks the Jev switch without flipping it", async () => {
  const user = userEvent.setup();
  await renderWithApp(<ConnectedSettings />);
  const input = await screen.findByLabelText("TypeSafe API key");
  expect(input).toHaveAttribute("type", "password");
  expect(await jevSwitch()).toBeDisabled();
  expect(await jevSwitch()).not.toBeChecked();
  expect(await jevSwitch()).toHaveAccessibleDescription(/api\.typesafe\.ai.*Add a TypeSafe key first\./);
  expect(triageStatus()).toHaveTextContent("None");

  await user.type(input, "invalid-key");
  await user.click(screen.getByRole("button", { name: "Save TypeSafe key" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("TypeSafe rejected this API key.");
  expect(input).toBeInvalid();
  expect((await api.getSettings()).hasTypesafeKey).toBe(false);

  await user.clear(input);
  await user.type(input, "typesafe-test-key");
  await user.click(screen.getByRole("button", { name: "Save TypeSafe key" }));
  expect(await screen.findByText("A validated TypeSafe key is stored in your system keychain.")).toBeInTheDocument();
  expect(screen.queryByLabelText("TypeSafe API key")).toBeNull();
  // The Anthropic field is untouched by all of this.
  expect(screen.getByLabelText("Anthropic API key")).toHaveValue("");

  await waitFor(async () => expect(await jevSwitch()).toBeEnabled());
  expect(await jevSwitch()).not.toBeChecked();
  expect(await jevSwitch()).toHaveAccessibleDescription(/Jev is sent more of each script than Claude Haiku\.$/);
  expect(await api.getSettings()).toMatchObject({ hasTypesafeKey: true, jevEnabled: false });
});

test("the Jev switch decides who triages scripts, and removing the TypeSafe key switches it back off", async () => {
  const user = userEvent.setup();
  await api.setApiKey(VALID_KEY);
  await api.setApiKey("typesafe-test-key", "typesafe");
  await renderWithApp(<ConnectedSettings />);

  expect(await jevSwitch()).toBeEnabled();
  expect(triageStatus()).toHaveTextContent("Claude Haiku 5.5");

  await user.click(await jevSwitch());
  await waitFor(async () => expect((await api.getSettings()).jevEnabled).toBe(true));
  expect(await jevSwitch()).toBeChecked();
  expect(triageStatus()).toHaveTextContent("Jev System One");
  expect(triageStatus()).toHaveTextContent("Claude Haiku 5.5 takes over if Jev fails.");

  // Without the Anthropic key Jev still triages, but nothing analyses the logic.
  await user.click(screen.getByRole("button", { name: "Remove key" }));
  await user.click(
    within(await screen.findByRole("alertdialog", { name: "Remove the API key?" })).getByRole("button", {
      name: "Remove key",
    }),
  );
  await waitFor(() => expect(triageStatus()).toHaveTextContent("Logic analysis needs an Anthropic key."));
  expect(triageStatus()).toHaveTextContent("Jev System One");

  await user.click(screen.getByRole("button", { name: "Remove TypeSafe key" }));
  const dialog = await screen.findByRole("alertdialog", { name: "Remove the TypeSafe key?" });
  expect(dialog).toHaveTextContent("Jev is turned off");
  await user.click(within(dialog).getByRole("button", { name: "Remove TypeSafe key" }));
  expect(await screen.findByLabelText("TypeSafe API key")).toHaveValue("");
  expect(await jevSwitch()).toBeDisabled();
  expect(await jevSwitch()).not.toBeChecked();
  expect(triageStatus()).toHaveTextContent("None");
  expect(await api.getSettings()).toMatchObject({ hasTypesafeKey: false, jevEnabled: false });
});

test("the sellers.json check is on until switched off, and says whom it contacts and how", async () => {
  const user = userEvent.setup();
  await renderWithApp(<ConnectedSettings />);
  const sellersJson = () => screen.findByRole("switch", { name: "Check sellers.json at ad exchanges" });

  expect(await sellersJson()).toBeChecked();
  expect(await sellersJson()).toHaveAccessibleDescription(
    /^On: a scan asks the ad exchanges that the scanned site's ads\.txt names \(.*at most 12 per scan\) for their sellers\.json.* never through the proxy chosen for a scan\. A page on this Mac, such as the demo site, is never asked about\. Turn it off on a metered or slow connection .*or to contact only the scanned site\.$/,
  );

  await user.click(await sellersJson());
  await waitFor(async () => expect((await api.getSettings()).checkSellersJson).toBe(false));
  expect(await sellersJson()).not.toBeChecked();

  await user.click(await sellersJson());
  await waitFor(async () => expect((await api.getSettings()).checkSellersJson).toBe(true));
});

test("the kept sellers.json files are shown with their size and can be cleared", async () => {
  const user = userEvent.setup();
  expect(await api.getSellersJsonCache()).toEqual({ entries: 0, storedBytes: 0 });
  // A scan with the check on leaves the files of the site's exchanges behind.
  const scan = await api.startScan({
    url: "https://www.sportsdesk.example/football",
    device: "desktop",
    observeSeconds: 20,
    autoScroll: true,
    acceptConsent: true,
    aiEnabled: false,
  });
  await api.cancelScan(scan.id);
  await renderWithApp(<ConnectedSettings />);
  expect(await screen.findByText("from 6 exchanges")).toBeInTheDocument();
  expect(screen.getByText("7.0 MB")).toBeInTheDocument();

  await user.click(screen.getByRole("button", { name: "Clear kept files" }));
  expect(await screen.findByText("None kept")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Clear kept files" })).toBeDisabled();
  expect(await api.getSellersJsonCache()).toEqual({ entries: 0, storedBytes: 0 });
  // The scan itself is not touched.
  expect((await api.listScans({})).items.map((item) => item.id)).toContain(scan.id);
  await api.deleteAllScans();
});

test("About names every host the app talks to", async () => {
  await renderWithApp(<ConnectedSettings />);
  expect(await screen.findByText(/Outbound traffic goes only to/)).toHaveTextContent(
    "the site being scanned (the page you enter, the pages a browse visit follows on that site and its ads.txt), api.anthropic.com, api.typesafe.ai when Jev is enabled and, unless the sellers.json check is switched off, the ad exchanges that ads.txt names. A scan's traffic to the site goes through the proxy you chose for it, if any; the requests to the exchanges and to the model APIs do not.",
  );
});

test("the key can be revealed while typing", async () => {
  const user = userEvent.setup();
  await renderWithApp(<ConnectedSettings />);
  const input = await keyInput();
  await user.click(screen.getByRole("button", { name: "Show key" }));
  expect(input).toHaveAttribute("type", "text");
  await user.click(screen.getByRole("button", { name: "Hide key" }));
  expect(input).toHaveAttribute("type", "password");
});

test("budget: invalid input is explained and not saved; a valid amount is saved on Enter", async () => {
  const user = userEvent.setup();
  await renderWithApp(<ConnectedSettings />);
  const budget = await screen.findByLabelText("Budget per scan");
  expect(budget).toHaveValue("0.50");

  await user.clear(budget);
  await user.type(budget, "0.01");
  await user.tab();
  expect(await screen.findByText("The budget must be at least $0.05.")).toBeInTheDocument();
  expect(budget).toBeInvalid();
  expect((await api.getSettings()).budgetUsd).toBe(0.5);

  await user.clear(budget);
  await user.type(budget, "abc{Enter}");
  expect(await screen.findByText("Enter an amount in dollars.")).toBeInTheDocument();

  await user.clear(budget);
  await user.type(budget, "1.25{Enter}");
  await waitFor(async () => expect((await api.getSettings()).budgetUsd).toBe(1.25));
  expect(screen.queryByText(/The budget must/)).toBeNull();
  expect(budget).toHaveValue("1.25");
});

test("Escape restores the saved value of a text setting", async () => {
  const user = userEvent.setup();
  await renderWithApp(<ConnectedSettings />);
  const model = await screen.findByLabelText("Script filter model");

  await user.clear(model);
  await user.type(model, "bad model id");
  await user.keyboard("{Escape}");
  expect(model).toHaveValue("claude-haiku-5-5");
  expect((await api.getSettings()).filterModel).toBe("claude-haiku-5-5");

  await user.clear(model);
  await user.type(model, "claude-opus-5-5");
  await user.tab();
  await waitFor(async () => expect((await api.getSettings()).filterModel).toBe("claude-opus-5-5"));
});

test("toggles save immediately", async () => {
  const user = userEvent.setup();
  await renderWithApp(<ConnectedSettings />);

  await user.click(await screen.findByRole("radio", { name: "Dark" }));
  await waitFor(async () => expect((await api.getSettings()).theme).toBe("dark"));
  expect(screen.getByRole("radio", { name: "Dark" })).toBeChecked();

  await user.click(screen.getByRole("switch", { name: "Run headless" }));
  await waitFor(async () => expect((await api.getSettings()).headless).toBe(false));
});

test("browser: shows the detected browser, accepts a manual executable and returns to auto-detect", async () => {
  const user = userEvent.setup();
  await renderWithApp(<ConnectedSettings />);

  expect(await screen.findByText("Google Chrome 141.0.7390.55")).toBeInTheDocument();
  expect(screen.getByText(/detected automatically/)).toBeInTheDocument();

  await user.click(screen.getByRole("button", { name: /Choose executable/ }));
  expect(await screen.findByText(/chosen manually/)).toBeInTheDocument();
  expect(await screen.findByText("/Applications/Chromium.app/Contents/MacOS/Chromium")).toBeInTheDocument();

  await user.click(screen.getByRole("button", { name: "Use auto-detect" }));
  expect(await screen.findByText(/detected automatically/)).toBeInTheDocument();
  expect((await api.getSettings()).chromePath).toBeNull();
});
