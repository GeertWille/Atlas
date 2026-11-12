import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, test, vi } from "vitest";
import { api } from "@/lib/api";
import { makeDetail, makeEnvironment, makeFinding } from "@/test/fixtures";
import { renderWithApp } from "@/test/render";
import { EnvironmentNotice } from "./EnvironmentNotice";
import { environmentNotice } from "./environment-notice";
import { ReportOverview } from "./ReportOverview";

vi.hoisted(() => {
  localStorage.setItem("adscan.fake.profile", "fresh");
  localStorage.setItem("adscan.fake.speed", "0.001");
});

beforeEach(async () => {
  await api.deleteAllScans();
});

test("no notice for a scan without a recorded environment, or for a visit a person drove in a window", () => {
  expect(environmentNotice(makeDetail())).toBeNull();
  expect(
    environmentNotice(
      makeDetail({ environment: makeEnvironment({ visitor: "manual", webdriver: false, headless: false }) }),
    ),
  ).toBeNull();
  // Nothing was seen of the flag and the browser had a window: there is no sign to report.
  expect(
    environmentNotice(makeDetail({ environment: makeEnvironment({ webdriver: null, headless: false }) })),
  ).toBeNull();
});

test("a scripted visit is reported with the facts the page could see", () => {
  const headless = environmentNotice(makeDetail({ environment: makeEnvironment() }));
  expect(headless).toEqual({
    signals: ["navigator.webdriver was true", "Chrome ran headless"],
    canShowBrowser: true,
  });
  const shown = environmentNotice(
    makeDetail({
      environment: makeEnvironment({
        visitor: "browse",
        headless: false,
        detectors: [
          { vendor: "DoubleVerify", kind: "invalidTrafficMeasurement", host: "cdn.doubleverify.com" },
          { vendor: "DataDome", kind: "botProtection", host: "js.datadome.co" },
        ],
      }),
    }),
  );
  expect(shown?.signals).toEqual([
    "navigator.webdriver was true",
    "DoubleVerify (invalid-traffic measurement) was on the page",
    "DataDome (bot protection) was on the page",
  ]);
  // The browser was already shown: there is no second thing to try.
  expect(shown?.canShowBrowser).toBe(false);
});

test("headless Chrome alone is a sign, as it is for the backend's own rule", () => {
  expect(environmentNotice(makeDetail({ environment: makeEnvironment({ webdriver: null }) }))).toEqual({
    signals: ["Chrome ran headless"],
    canShowBrowser: true,
  });
});

test("the notice follows the recorded environment, never the wording of a finding", () => {
  const lookalike = makeFinding({
    severity: "info",
    category: "configuration",
    source: "heuristic",
    title: "This scan was identifiable as automated traffic",
  });
  const person = makeEnvironment({ visitor: "manual", webdriver: false, headless: false });
  expect(environmentNotice(makeDetail({ findings: [lookalike], environment: person }))).toBeNull();
  // Without that finding, or with its title reworded, a scripted visit is still reported.
  expect(environmentNotice(makeDetail({ findings: [], environment: makeEnvironment() }))).not.toBeNull();
});

test("nothing is rendered when there is nothing to say", async () => {
  await renderWithApp(<EnvironmentNotice detail={makeDetail()} />);
  expect(screen.queryByRole("note")).toBeNull();
});

test("the notice states the facts, what they mean and what to try, without promising anything", async () => {
  await renderWithApp(<EnvironmentNotice detail={makeDetail({ environment: makeEnvironment() })} />);
  const notice = screen.getByRole("note", { name: "Scan environment" });
  expect(notice).toHaveTextContent(
    "This visit was automated and the pages could tell: navigator.webdriver was true, Chrome ran headless. Ad partners can bid less, or not at all, for automated visits. To compare, repeat the scan as “I'll browse myself” or with the browser shown.",
  );
  expect(notice).not.toHaveTextContent(/undetect|indistinguishable|like a (real|human)/i);
  expect(
    within(notice)
      .getAllByRole("button")
      .map((button) => button.textContent),
  ).toEqual(["as “I'll browse myself”", "with the browser shown"]);
});

test("the report says it once, as a line of the visit summary", async () => {
  const detail = makeDetail({ overview: "Two auctions ran.", environment: makeEnvironment() });
  await renderWithApp(<ReportOverview detail={detail} />);
  const overview = screen.getByRole("region", { name: "Overview" });
  expect(within(overview).getAllByRole("note", { name: "Scan environment" })).toHaveLength(1);
  // A remark every scripted scan carries must not look like an alarm.
  expect(within(overview).queryByRole("alert")).toBeNull();
});

test("repeating as a manual session starts a headed visit that clicks nothing, without the old seed", async () => {
  const user = userEvent.setup();
  const detail = makeDetail({
    environment: makeEnvironment(),
    options: { ...makeDetail().options, visitor: "browse", extraPages: 2, headless: true, seed: 99 },
  });
  await renderWithApp(<EnvironmentNotice detail={detail} />);
  await user.click(screen.getByRole("button", { name: "as “I'll browse myself”" }));

  await screen.findByText("scan screen");
  const [scan] = (await api.listScans({})).items;
  const started = await api.getScan(scan?.id ?? "");
  expect(started.options).toMatchObject({ visitor: "manual", headless: false, acceptConsent: false, extraPages: 0 });
  expect(started.options.seed).not.toBe(99);
});

test("repeating with the browser shown changes nothing else about the scan", async () => {
  const user = userEvent.setup();
  const detail = makeDetail({
    environment: makeEnvironment(),
    options: { ...makeDetail().options, visitor: "browse", extraPages: 2, headless: true },
  });
  await renderWithApp(<EnvironmentNotice detail={detail} />);
  await user.click(screen.getByRole("button", { name: "with the browser shown" }));

  await waitFor(async () => expect((await api.listScans({})).total).toBe(1));
  const [scan] = (await api.listScans({})).items;
  expect((await api.getScan(scan?.id ?? "")).options).toMatchObject({
    visitor: "browse",
    extraPages: 2,
    headless: false,
  });
});
