import { useEffect, useState } from "react";
import { toast } from "sonner";

/** Clipboard write with a short-lived "copied" flag for button feedback. */
export function useCopy() {
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    if (!copied) return;
    const timer = setTimeout(() => setCopied(false), 1600);
    return () => clearTimeout(timer);
  }, [copied]);

  const copy = async (text: string) => {
    try {
      await navigator.clipboard.writeText(text);
      setCopied(true);
    } catch {
      toast.error("Could not copy to the clipboard");
    }
  };

  return { copied, copy };
}
