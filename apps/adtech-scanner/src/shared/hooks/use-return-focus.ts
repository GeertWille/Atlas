import { useRef } from "react";

/**
 * Focus handling for a dialog that is opened without a Radix trigger (from a
 * shortcut or a menu item). Radix only returns focus to a trigger, so closing
 * such a dialog would drop keyboard users at the top of the document. Spread
 * the result onto the dialog content.
 */
export function useReturnFocus() {
  const opener = useRef<Element | null>(null);
  return {
    onOpenAutoFocus: () => {
      opener.current = document.activeElement;
    },
    onCloseAutoFocus: (event: Event) => {
      event.preventDefault();
      // Whatever the dialog did may have moved focus on purpose; only fill the gap it left behind.
      const focusWasDropped = document.activeElement === document.body;
      if (focusWasDropped && opener.current instanceof HTMLElement) opener.current.focus();
    },
  };
}
