import { render, screen } from "@testing-library/react";
import { expect, test } from "vitest";
import { FilterMenu } from "./filter-menu";

const options = [
  { value: "visitor", label: "Visitor" },
  { value: "message", label: "Message" },
];

/** WCAG 2.5.3: what a person reads on a control has to be part of its accessible name. */
function expectNameContainsVisibleText(button: HTMLElement) {
  const visible = (button.textContent ?? "").trim().toLowerCase();
  expect((button.getAttribute("aria-label") ?? "").toLowerCase()).toContain(visible);
}

test("the trigger's name contains everything the trigger shows, with and without a selection", () => {
  const { rerender } = render(<FilterMenu label="Kind" options={options} selected={[]} onChange={() => {}} />);
  const trigger = screen.getByRole("button", { name: "Filter by kind" });
  expect(trigger).toHaveTextContent(/^Kind$/);
  expectNameContainsVisibleText(trigger);

  rerender(<FilterMenu label="Kind" options={options} selected={["visitor", "message"]} onChange={() => {}} />);
  const filtered = screen.getByRole("button", { name: "Filter by kind 2 selected" });
  expect(filtered).toHaveTextContent(/^Kind 2$/);
  expectNameContainsVisibleText(filtered);
});
