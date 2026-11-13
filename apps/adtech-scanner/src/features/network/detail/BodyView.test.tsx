import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";
import type { BodyRef, NetworkBodyChunk } from "@/bindings";
import { ApiError, api } from "@/lib/api";
import { renderWithApp } from "@/test/render";
import { BodyView } from "./BodyView";

const BODY: BodyRef = { state: "kept", bytes: 24, storedBytes: 24, mime: "text/plain", scriptId: null };
const chunk = (text: string, offset: number, nextOffset: number | null): NetworkBodyChunk => ({
  state: "kept",
  text,
  offset,
  nextOffset,
  totalBytes: 24,
});

test("a later stretch of a body that fails keeps what was read, and a first one that fails is an error", async () => {
  const user = userEvent.setup();
  const locked = () => new ApiError("storage", "The database is locked");
  const getBody = vi
    .spyOn(api, "getNetworkBody")
    .mockRejectedValueOnce(locked())
    .mockResolvedValueOnce(chunk("first stretch", 0, 13))
    .mockRejectedValueOnce(locked())
    .mockResolvedValueOnce(chunk(" and the rest", 13, null));
  await renderWithApp(<BodyView scanId="s1" requestId="r1" side="response" body={BODY} mime="text/plain" />);

  expect(await screen.findByText("Could not read this body")).toBeInTheDocument();
  expect(screen.queryByRole("alert")).toBeNull();
  await user.click(screen.getByRole("button", { name: "Try again" }));
  const editor = await screen.findByRole("textbox", { name: "Response body" });
  expect(editor).toHaveTextContent("first stretch");

  await user.click(screen.getByRole("button", { name: "Load more" }));
  const notice = within(await screen.findByRole("alert"));
  expect(notice.getByText("Could not read more of this body")).toBeInTheDocument();
  expect(notice.getByText(/The database is locked/)).toBeInTheDocument();
  expect(notice.getByText("The first 13 B of 24 B are shown.")).toBeInTheDocument();
  expect(screen.getByRole("textbox", { name: "Response body" })).toHaveTextContent("first stretch");
  expect(screen.queryByText("Could not read this body")).toBeNull();

  await user.click(notice.getByRole("button", { name: "Try again" }));
  await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
  expect(screen.getByRole("textbox", { name: "Response body" })).toHaveTextContent("first stretch and the rest");
  // The stretch that was read is not read again.
  expect(getBody.mock.calls.map(([, , query]) => query?.offset)).toEqual([0, 0, 13, 13]);
});
