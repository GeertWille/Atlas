import { screen } from "@testing-library/react";
import { expect, test } from "vitest";
import type { DecodedBidRequest, DecodedBidResponse, Header } from "@/bindings";
import { makeDetail, makeFinding } from "@/test/fixtures";
import { makeAdLink, makeRequestDetail, makeRow } from "@/test/network-fixtures";
import { renderWithApp } from "@/test/render";
import { expectInert, HOSTILE } from "@/test/untrusted";
import { DecodedRequest, DecodedResponse } from "./Decoded";
import { HeadersTab, InitiatorTab, PayloadTab, SummaryTab } from "./tabs";

const header = (name: string, value: string): Header => ({ name, value, redaction: null });
const everything = Object.values(HOSTILE).join(" ");

test("the summary shows a hostile URL, error and finding title as text", async () => {
  const url = `https://rtb.example/bid?next=${encodeURIComponent(HOSTILE.scriptUrl)}&html=${HOSTILE.image}`;
  const row = makeRow({
    url,
    category: "bid",
    bidder: HOSTILE.link,
    vendor: HOSTILE.style,
    outcome: "failed",
    error: HOSTILE.image,
    ad: makeAdLink({ verdict: "failed", adUnits: [HOSTILE.script] }),
  });
  const finding = makeFinding({ id: "f1", title: `${HOSTILE.image} ${HOSTILE.bidi}` });
  const scan = makeDetail({ findings: [finding] });
  const detail = makeRequestDetail({
    row,
    redirectChain: [
      { requestId: "r0", url: HOSTILE.scriptUrl, status: 302 },
      { requestId: row.id, url, status: 200 },
    ],
    adContext: {
      link: makeAdLink(),
      bidder: HOSTILE.link,
      units: [],
      auctionStartedAtMs: 0,
      auctionEndedAtMs: 100,
      timeoutMs: 100,
      slotElementId: null,
      findingIds: ["f1"],
      checkCodes: [],
    },
  });
  const { container } = await renderWithApp(
    <SummaryTab scan={scan} row={row} detail={detail} onOpenRequest={() => undefined} />,
  );
  expectInert(container);
  expect(container.textContent).toContain(HOSTILE.image);
  expect(container.textContent).toContain("user⟨U+202E⟩ // admins only⟨U+2069⟩");
  // The hop of the redirect chain is a button of the app, not a link to the address it names.
  expect(screen.getByRole("button", { name: HOSTILE.scriptUrl })).toBeInTheDocument();
});

test("header names and values, and query parameters, are text whatever they hold", async () => {
  const detail = makeRequestDetail({
    row: makeRow({ url: `https://x.example/a?${encodeURIComponent(HOSTILE.image)}=${encodeURIComponent(everything)}` }),
    requestHeaders: [header("Referer", HOSTILE.scriptUrl), header(HOSTILE.image, everything)],
    responseHeaders: [header("Location", HOSTILE.remoteUrl), header("Link", `<${HOSTILE.remoteUrl}>; rel=preload`)],
  });
  const headers = await renderWithApp(<HeadersTab detail={detail} />);
  expectInert(headers.container);
  expect(headers.container.querySelectorAll("a")).toHaveLength(0);
  expect(headers.container.textContent).toContain(HOSTILE.link);
  expect(headers.container.textContent).toContain("pay⟨U+200B⟩pal.com");
  headers.unmount();

  const payload = await renderWithApp(<PayloadTab scanId="scan-1" detail={detail} />);
  expectInert(payload.container);
  expect(payload.container.querySelectorAll("a")).toHaveLength(0);
  expect(payload.container.textContent).toContain(HOSTILE.image);
});

test("a call stack with hostile function names and script addresses is text", async () => {
  const detail = makeRequestDetail({
    stack: [
      { function: HOSTILE.image, url: HOSTILE.scriptUrl, scriptId: null, line: 1, column: 1 },
      { function: HOSTILE.bidi, url: HOSTILE.remoteUrl, scriptId: null, line: 2, column: 3 },
    ],
  });
  const { container } = await renderWithApp(<InitiatorTab scanId="scan-1" detail={detail} />);
  expectInert(container);
  expect(container.querySelectorAll("a")).toHaveLength(0);
  expect(container.textContent).toContain(HOSTILE.image);
});

test("decoded OpenRTB fields (advertiser domains, creative ids, seats, deals) are text", async () => {
  const request: DecodedBidRequest = {
    id: HOSTILE.image,
    tmaxMs: 250,
    imps: [
      {
        id: HOSTILE.script,
        adUnit: HOSTILE.link,
        tagId: HOSTILE.style,
        mediaTypes: [HOSTILE.image],
        sizes: [HOSTILE.image],
        floor: 0.5,
        floorCurrency: HOSTILE.image,
        bidders: [HOSTILE.link],
      },
    ],
    siteDomain: HOSTILE.bidi,
    sitePage: HOSTILE.scriptUrl,
    currencies: [HOSTILE.image],
    schain: { complete: true, nodes: [{ asi: HOSTILE.link, sid: HOSTILE.zeroWidth, hp: true }] },
    gdpr: true,
    coppa: null,
    consentStringPresent: true,
    usPrivacyPresent: false,
    gppPresent: false,
    eidSources: [HOSTILE.image, HOSTILE.remoteUrl],
    test: false,
  };
  const response: DecodedBidResponse = {
    id: HOSTILE.image,
    currency: HOSTILE.style,
    nbr: null,
    nbrMeaning: null,
    seats: [
      {
        seat: HOSTILE.link,
        bids: [
          {
            impId: HOSTILE.script,
            adUnit: HOSTILE.bidi,
            price: 1.25,
            size: HOSTILE.image,
            advertiserDomains: [HOSTILE.remoteUrl, HOSTILE.zeroWidth, HOSTILE.scriptUrl],
            creativeId: HOSTILE.image,
            dealId: HOSTILE.link,
            // The creative markup itself is never part of the decoded view: only its size.
            markupBytes: 2048,
          },
        ],
      },
    ],
    serverTimings: [{ bidder: HOSTILE.image, ms: 12 }],
    serverErrors: [{ bidder: HOSTILE.image, code: 1, message: HOSTILE.script }],
  };
  const { container } = await renderWithApp(
    <>
      <DecodedRequest request={request} />
      <DecodedResponse response={response} />
    </>,
  );
  expectInert(container);
  expect(container.querySelectorAll("a")).toHaveLength(0);
  expect(container.textContent).toContain(HOSTILE.remoteUrl);
  expect(container.textContent).toContain("pay⟨U+200B⟩pal.com");
  expect(container.textContent).toContain("2.0 KB");
});
