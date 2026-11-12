import { execFileSync, spawnSync } from "node:child_process";
import type { Browser } from "./app";

/** Native file dialogs are outside the webview; on X11 they can be driven with xdotool, and xclip for the path. */
export const canDriveNativeDialogs = ["xdotool", "xclip"].every((tool) => spawnSync("which", [tool]).status === 0);

const xdotool = (...args: string[]) => execFileSync("xdotool", args, { encoding: "utf8" });
const appWindows = () => xdotool("search", "--onlyvisible", "--class", "adtech-scanner").trim().split("\n");

const SETTLE_MS = 300;

export type NativeDialog = {
  cancel(): Promise<void>;
  /** Enters an absolute path and confirms. */
  submitPath(path: string): Promise<void>;
};

/** Runs `open`, which must make the app show a native file dialog, and returns that dialog once it is up. */
export async function nativeDialog(browser: Browser, open: () => Promise<void>): Promise<NativeDialog> {
  const before = appWindows();
  await open();
  let dialog: string | undefined;
  await browser.waitUntil(
    async () => {
      dialog = appWindows().find((window) => !before.includes(window));
      return dialog !== undefined;
    },
    { timeout: 15_000, timeoutMsg: "no native dialog appeared" },
  );
  // Without a window manager the keyboard goes to the window under the pointer.
  xdotool("mousemove", "--window", dialog ?? "", "300", "300");
  type FocusWindow = Window & { __refocused?: boolean };
  await browser.execute(() => {
    const page = window as FocusWindow;
    page.__refocused = false;
    window.addEventListener("focus", () => (page.__refocused = true), { once: true });
  });
  // The window gets its focus back a moment after the dialog is gone, and that
  // moves focus in the page, which would close a menu a test opened before it.
  const closed = () =>
    browser.waitUntil(async () => browser.execute(() => (window as FocusWindow).__refocused === true), {
      timeoutMsg: "the dialog stayed open",
    });
  return {
    cancel: async () => {
      xdotool("key", "Escape");
      await closed();
    },
    submitPath: async (path) => {
      // Pasted, not typed: the dialog's path completion garbles fast typing.
      execFileSync("xclip", ["-selection", "clipboard"], { input: path, stdio: ["pipe", "ignore", "ignore"] });
      // The dialog builds its path field after it is shown, and again when asked for it.
      await browser.pause(SETTLE_MS);
      xdotool("key", "ctrl+l");
      await browser.pause(SETTLE_MS);
      xdotool("key", "ctrl+a", "ctrl+v");
      await browser.pause(SETTLE_MS);
      xdotool("key", "Return");
      await closed();
    },
  };
}
