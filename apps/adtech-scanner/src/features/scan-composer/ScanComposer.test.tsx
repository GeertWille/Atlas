import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, test, vi } from "vitest";
import { api } from "@/lib/api";
import { settingsFixture } from "@/test/fixtures";
import { renderWithApp } from "@/test/render";
import { ScanComposer } from "./ScanComposer";

// The composer talks to the real api module, which resolves to the fake backend outside Tauri.
vi.hoisted(() => {
  localStorage.setItem("adscan.fake.profile", "fresh");
  localStorage.setItem("adscan.fake.speed", "0.001");
});

beforeEach(async () => {
  await api.deleteAllScans();
});

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
  await user.click(screen.getByRole("switch", { name: "Auto-scroll" }));
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
