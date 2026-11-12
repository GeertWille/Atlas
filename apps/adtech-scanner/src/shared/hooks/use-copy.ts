import { useEffect, useState } from "react";
import { toast } from "sonner";

/**
 * Writes text to the clipboard. WebKit allows a write only while the click or key press that asked for
 * it is being handled, so text that has to be fetched first is handed over as a promise: the write
 * starts at once and the browser waits for the text.
 */
async function writeToClipboard(text: string | Promise<string>): Promise<void> {
  if (typeof text === "string") return navigator.clipboard.writeText(text);
  if (typeof ClipboardItem === "undefined") return navigator.clipboard.writeText(await text);
  const blob = text.then((value) => new Blob([value], { type: "text/plain" }));
  return navigator.clipboard.write([new ClipboardItem({ "text/plain": blob })]);
}

/** Clipboard write with a short-lived "copied" flag for button feedback. */
export function useCopy() {
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    if (!copied) return;
    const timer = setTimeout(() => setCopied(false), 1600);
    return () => clearTimeout(timer);
  }, [copied]);

  /**
   * Resolves to whether the text is on the clipboard. A promise that rejects copies nothing and is the
   * caller's to explain; a refusal by the browser is reported here. `done`: what to say once it is copied.
   */
  const copy = async (text: string | Promise<string>, done?: string): Promise<boolean> => {
    let textFailed = false;
    const tracked =
      typeof text === "string"
        ? text
        : text.catch((reason: unknown) => {
            textFailed = true;
            throw reason;
          });
    try {
      await writeToClipboard(tracked);
      setCopied(true);
      if (done) toast.success(done);
      return true;
    } catch {
      if (!textFailed) toast.error("Could not copy to the clipboard");
      return false;
    }
  };

  return { copied, copy };
}
