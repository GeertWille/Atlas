// Runs before first paint so the stored theme never flashes. Kept as an external
// file (not inline) so it passes Tauri's default CSP.
(() => {
  let preference = "system";
  try {
    preference = localStorage.getItem("adscan.theme") ?? "system";
  } catch {}
  const dark =
    preference === "dark" ||
    (preference !== "light" && matchMedia("(prefers-color-scheme: dark)").matches);
  document.documentElement.classList.toggle("dark", dark);
})();
