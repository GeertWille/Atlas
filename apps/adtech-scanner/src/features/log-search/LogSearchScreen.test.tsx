import { act, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { expect, test, vi } from "vitest";
import { renderWithApp } from "@/test/render";
import { LogSearchScreen } from "./LogSearchScreen";

const input = () => screen.getByRole("searchbox", { name: "Search all audit trails" });
const pastDebounce = () => act(() => new Promise((resolve) => setTimeout(resolve, 400)));

test("typing commits the trimmed query once the field settles", async () => {
  const user = userEvent.setup();
  const onQueryChange = vi.fn();
  await renderWithApp(<LogSearchScreen query="" onQueryChange={onQueryChange} />);

  await user.type(input(), " timeout ");
  await pastDebounce();
  expect(onQueryChange).toHaveBeenCalledExactlyOnceWith("timeout");
});

let setRouteQuery: (query: string) => void = () => {};

/** Owns the query like the real route does, and lets the test change it from outside the screen. */
function Route({ onQueryChange }: { onQueryChange: (query: string) => void }) {
  const [query, setQuery] = useState("failsafe");
  setRouteQuery = setQuery;
  return (
    <LogSearchScreen
      query={query}
      onQueryChange={(next) => {
        onQueryChange(next);
        setQuery(next);
      }}
    />
  );
}

test("a query changed by the route replaces the field instead of being overwritten by it", async () => {
  const onQueryChange = vi.fn();
  await renderWithApp(<Route onQueryChange={onQueryChange} />);
  expect(input()).toHaveValue("failsafe");

  // What the command palette does while this screen is already open.
  act(() => setRouteQuery("IS_MOBILE"));
  expect(input()).toHaveValue("IS_MOBILE");
  await pastDebounce();
  expect(input()).toHaveValue("IS_MOBILE");
  expect(onQueryChange).not.toHaveBeenCalled();
});
