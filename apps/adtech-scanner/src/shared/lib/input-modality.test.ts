import { fireEvent } from "@testing-library/react";
import { expect, test } from "vitest";
import { trackInputModality } from "./input-modality";

test("the root element says whether the pointer or the keyboard was used last", () => {
  const root = document.createElement("div");
  const control = root.appendChild(document.createElement("button"));
  trackInputModality(root);
  expect(root.dataset.input).toBeUndefined();

  fireEvent.pointerDown(control);
  expect(root.dataset.input).toBe("pointer");
  // Holding a modifier for a click is not keyboard use.
  fireEvent.keyDown(control, { key: "Shift" });
  fireEvent.keyDown(control, { key: "Meta" });
  expect(root.dataset.input).toBe("pointer");

  fireEvent.keyDown(control, { key: "Tab" });
  expect(root.dataset.input).toBe("keyboard");
  fireEvent.pointerDown(control);
  expect(root.dataset.input).toBe("pointer");
  fireEvent.keyDown(control, { key: "k", metaKey: true });
  expect(root.dataset.input).toBe("keyboard");
});
