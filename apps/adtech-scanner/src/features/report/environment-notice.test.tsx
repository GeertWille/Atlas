import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, test, vi } from "vitest";
import { api } from "@/lib/api";
import { makeDetail, makeEnvironment, makeFinding } from "@/test/fixtures";
import { renderWithApp } from "@/test/render";
import { EnvironmentNotice } from "./EnvironmentNotice";
import { environmentNotice, isAutomatedTrafficFinding } from "./environment-notice";

vi.hoisted(() => {
  localStorage.setItem("adscan.fake.profile", "fresh");
  localStorage.setItem("adscan.fake.speed", "0.001");
});

beforeEach(async () => {
  await api.deleteAllScans();
});

const automatedFinding = makeFinding({
  id: "r6",
  severity: "info",
  category: "configuration",
  source: "heuristic",
  title: "This scan was identifiable as automated traffic",
});

test("the automated-traffic finding is recognised by what it is, not by a similar-looking one", () => {
  expect(isAutomatedTrafficFinding(automatedFinding)).toBe(true);
  expect(isAutomatedTrafficFinding({ ...automatedFinding, source: "ai" })).toBe(false);
  expect(isAutomatedTrafficFinding({ ...automatedFinding, severity: "medium" })).toBe(false);
  expect(isAutomatedTrafficFinding({ ...automatedFinding, category: "consent" })).toBe(false);
  expect(isAutomatedTrafficFinding({ ...automatedFinding, title: "Prebid debug mode is on" })).toBe(false);
});

test("no notice for a scan without a recorded environment, a manual visit, or an unreadable webdriver flag", () => {
  expect(environmentNotice(makeDetail())).toBeNull();
  expect(
    environmentNotice(
      makeDetail({ environment: makeEnvironment({ visitor: "manual", webdriver: false, headless: false }) }),
    ),
  ).toBeNull();
  expect(environmentNotice(makeDetail({ environment: makeEnvironment({ webdriver: null }) }))).toBeNull();
});

test("a scripted visit is reported with the facts the page could see", () => {
  const headless = environmentNotice(makeDetail({ environment: makeEnvironment() }));
  expect(headless).toEqual({
    title: "The pages could tell that this visit was automated",
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

test("the automated-traffic finding raises the notice even when webdriver was false, with a stronger title", () => {
  const notice = environmentNotice(
    makeDetail({
      findings: [automatedFinding],
      environment: makeEnvironment({ visitor: "manual", webdriver: false, headless: false }),
    }),
  );
  expect(notice).toMatchObject({ title: "Bidders may have treated this scan as automated traffic", signals: [] });
});

test("nothing is rendered when there is nothing to say", async () => {
  await renderWithApp(<EnvironmentNotice detail={makeDetail()} />);
  expect(screen.queryByRole("note")).toBeNull();
});

test("the notice states the facts, what they mean and what to try, without promising anything", async () => {
  await renderWithApp(<EnvironmentNotice detail={makeDetail({ environment: makeEnvironment() })} />);
  const notice = screen.getByRole("note", { name: "Scan environment" });
  expect(notice).toHaveTextContent(
    "The pages could tell that this visit was automated. navigator.webdriver was true, Chrome ran headless. Ad partners can bid less, or not at all, for automated visits, so low fill or missing bids in this report may not match your visitors'. Before concluding that a bidder is broken, repeat the scan as “I'll browse myself” or with the browser shown.",
  );
  expect(notice).not.toHaveTextContent(/undetect|indistinguishable|like a (real|human)/i);
  expect(
    within(notice)
      .getAllByRole("button")
      .map((button) => button.textContent),
  ).toEqual(["as “I'll browse myself”", "with the browser shown"]);
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
