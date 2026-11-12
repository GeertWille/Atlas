import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { expect, test } from "vitest";
import type { Auction, AuctionAdUnit } from "@/bindings";
import { makeAuction, makeBid, makeDetail } from "@/test/fixtures";
import { renderWithApp } from "@/test/render";
import { BidInspector } from "./BidInspector";

const unit = (code: string, sizes: string[], bids: AuctionAdUnit["bids"] = []): AuctionAdUnit => ({
  code,
  sizes,
  bids,
  winningBidder: null,
  winningCpm: null,
});

// Auctions as a scan stored before ad units were merged by code holds them ("twin" ad units, one entry
// per registered ad unit, every bid under the first), and one as the backend delivers them now.
const twins: Auction = makeAuction([], {
  id: "twins",
  adUnits: [
    unit(
      "gpt-ad-BANNER_ABOVE",
      ["980x150", "970x250"],
      [
        makeBid({ bidder: "gumgum", status: "won", cpm: 10.01, currency: "EUR" }),
        makeBid({ bidder: "rubicon", status: "bid", cpm: 2.05, currency: "USD" }),
        makeBid({ bidder: "appnexus", status: "noBid", cpm: null }),
      ],
    ),
    unit("gpt-ad-BANNER_ABOVE", ["970x250", "840x150"]),
    unit("gpt-ad-BANNER_ABOVE", ["970x250", "728x90"]),
    unit("gpt-ad-RECT_MIDDLE_1", ["300x600", "300x250"], [makeBid({ bidder: "rubicon", status: "noBid", cpm: null })]),
    unit("gpt-ad-BANNER_ABOVE", ["840x250"]),
  ],
});
const moreTwins: Auction = makeAuction([], {
  id: "more-twins",
  adUnits: [
    unit("gpt-ad-RECT_MIDDLE_1", ["300x250"], [makeBid({ bidder: "ix", status: "bid", cpm: 0.4 })]),
    unit("gpt-ad-RECT_MIDDLE_1", ["300x600"]),
  ],
});
const merged: Auction = makeAuction([], {
  id: "merged",
  adUnits: [
    unit("gpt-ad-BANNER_ABOVE", ["980x150", "970x250", "840x150"], [makeBid({ bidder: "gumgum", cpm: 1.2 })]),
    unit("gpt-ad-RECT_MIDDLE_1", ["300x600"], [makeBid({ bidder: "gumgum", status: "noBid", cpm: null })]),
    unit("gpt-ad-FOOTER", ["728x90"]),
  ],
});
const auctions = [twins, moreTwins, merged];
const scan = makeDetail({ auctions });

function inspector(auctionId: string) {
  return (
    <BidInspector
      scan={scan}
      auctions={auctions}
      auctionId={auctionId}
      onAuctionChange={() => {}}
      rows={new Map()}
      selectedId={undefined}
      onOpen={() => {}}
    />
  );
}

/** The header cells of the table, and the number of cells in each bidder row. */
function shape() {
  const table = screen.getByRole("table");
  const [header, ...bidders] = within(table).getAllByRole("row");
  return {
    headers: within(header as HTMLElement)
      .getAllByRole("columnheader")
      .map((cell) => cell.textContent),
    rowCells: bidders.map((row) => row.children.length),
  };
}

/** The inspector with a way to go from one auction to another, as its own menu does. */
function Switchable() {
  const [auctionId, setAuctionId] = useState("twins");
  return (
    <>
      {auctions.map((auction) => (
        <button key={auction.id} type="button" onClick={() => setAuctionId(auction.id)}>
          show {auction.id}
        </button>
      ))}
      {inspector(auctionId)}
    </>
  );
}

test("the header and every bidder row have the same columns, also after switching between auctions", async () => {
  const user = userEvent.setup();
  await renderWithApp(<Switchable />);
  const expectAligned = (auction: Auction) => {
    const { headers, rowCells } = shape();
    expect(headers).toEqual([
      "Bidder",
      ...auction.adUnits.map((entry) => `${entry.code}${entry.sizes.join(", ")}`),
      "Request",
    ]);
    expect(rowCells.length).toBeGreaterThan(0);
    for (const cells of rowCells) expect(cells).toBe(headers.length);
  };

  expectAligned(twins);
  // Switching auctions is what left columns of the auction before in the header.
  for (const auction of [merged, moreTwins, twins, moreTwins, merged, twins]) {
    await user.click(screen.getByRole("button", { name: `show ${auction.id}` }));
    expectAligned(auction);
  }
});

test("a bidder's cell sits under its own ad unit and its request summary under Request", async () => {
  await renderWithApp(inspector("twins"));
  const table = screen.getByRole("table");
  const row = within(table).getByRole("row", { name: /^rubicon/ });
  const cells = [...row.children].map((cell) => cell.textContent ?? "");
  // rubicon bid on the first column and on the rectangle, the fourth of five.
  expect(cells[1]).toContain("2.05 USD");
  expect(cells[2]).toBe("");
  expect(cells[3]).toBe("");
  expect(cells[4]).toContain("no bid");
  expect(cells[5]).toBe("");
  expect(cells).toHaveLength(7);
});

test("prices in two currencies each say which they are in, and the auction says that they are not comparable", async () => {
  await renderWithApp(inspector("twins"));
  const table = screen.getByRole("table");
  expect(within(table).getByText("10.01 EUR")).toBeInTheDocument();
  expect(within(table).getByText("2.05 USD")).toBeInTheDocument();
  expect(within(table).queryByText(/\$/)).toBeNull();
  expect(screen.getByText(/bids in EUR and USD, shown as they were bid/)).toBeInTheDocument();
});

test("with one currency the prices keep their short form and nothing is said about currencies", async () => {
  await renderWithApp(inspector("merged"));
  expect(within(screen.getByRole("table")).getByText("$1.20")).toBeInTheDocument();
  expect(screen.queryByText(/shown as they were bid/)).toBeNull();
  expect(screen.getByText(/1 bidder · 3 ad units/)).toBeInTheDocument();
});
