import { render, screen } from "@testing-library/react";
import { expect, test } from "vitest";
import { FilterMenu } from "./filter-menu";

const options = [
  { value: "visitor", label: "Visitor" },
  { value: "message", label: "Message" },
];

test("the trigger's name always contains its visible label; the counter is its description, not part of the name", () => {
  const { rerender } = render(<FilterMenu label="Kind" options={options} selected={[]} onChange={() => {}} />);
  const trigger = screen.getByRole("button", { name: "Filter by kind" });
  expect(trigger).toHaveTextContent(/^Kind$/);
  expect(trigger).not.toHaveAccessibleDescription();

  rerender(<FilterMenu label="Kind" options={options} selected={["visitor", "message"]} onChange={() => {}} />);
  const filtered = screen.getByRole("button", { name: "Filter by kind" });
  expect(filtered).toHaveAccessibleDescription("2 selected");
  // Speech input goes by the label a person can see: "Kind", which the name still contains.
  expect(filtered.querySelector("[aria-hidden='true']")).toHaveTextContent("2");
});
