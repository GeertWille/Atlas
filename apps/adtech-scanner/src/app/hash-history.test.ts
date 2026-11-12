import { afterEach, expect, test } from "vitest";
import { createFragmentHistory } from "./hash-history";

afterEach(() => window.history.replaceState(null, "", "/"));

test("an empty fragment is the home route", () => {
  window.history.replaceState(null, "", "/");
  const history = createFragmentHistory();
  expect(history.location.pathname).toBe("/");
  expect(history.location.search).toBe("");
  history.destroy();
});

test("route search params survive a real query string in front of the fragment", () => {
  window.history.replaceState(null, "", "/?ref=mail&tab=findings#/scans/scan-1?tab=audit&log=l7");
  const history = createFragmentHistory();
  expect(history.location.pathname).toBe("/scans/scan-1");
  expect(history.location.search).toBe("?tab=audit&log=l7");
  history.destroy();
});

test("navigation rewrites the fragment only and keeps the document's own query string", () => {
  window.history.replaceState(null, "", "/?ref=mail#/history");
  const history = createFragmentHistory();
  history.push("/logs?q=timeout");
  history.flush();
  expect(`${window.location.pathname}${window.location.search}${window.location.hash}`).toBe(
    "/?ref=mail#/logs?q=timeout",
  );
  expect(history.location.pathname).toBe("/logs");
  expect(history.location.search).toBe("?q=timeout");
  history.destroy();
});
