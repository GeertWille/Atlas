const MODIFIER_KEYS = new Set(["Shift", "Control", "Alt", "Meta"]);

/**
 * Records on the root element whether the person last used the pointer or the keyboard (`data-input`), for the
 * `pointer-input:` variant of the shared controls.
 *
 * WebKit matches `:focus-visible` on an element that a script focuses while nothing else has focus, and a click
 * does not always focus the clicked control there. Radix moves focus from script all the time: to a clicked tab,
 * into a dialog that opens, back to a select or menu trigger when its list closes. Each of those drew a focus
 * ring after a mouse click. The controls therefore hide the ring while the last input was the pointer; the first
 * key press brings it back.
 */
export function trackInputModality(root: HTMLElement = document.documentElement) {
  root.addEventListener(
    "pointerdown",
    () => {
      root.dataset.input = "pointer";
    },
    true,
  );
  root.addEventListener(
    "keydown",
    (event) => {
      if (!MODIFIER_KEYS.has(event.key)) root.dataset.input = "keyboard";
    },
    true,
  );
}
