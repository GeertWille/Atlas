# Design plans

Design documents written while building the scanner. They are records of the plan at the time, not
descriptions of the code: where they differ from the code, the code and `HANDOVER.md` are right.
Paths starting with `/tmp/` or `/home/user/` refer to the machine they were written on.

| File | Status |
|---|---|
| `scan-archive.md` | Built (pack file per scan, SQLite index). |
| `script-viewer.md` | Built (v1). Cross-script search and diff are open tickets. |
| `network-view.md` | Built (v1). HAR export, saved filters, body search and live tail are open tickets. |
| `visitor-and-multi-page.md` | Built. |
| `jev-strategy.md` | Built. Thresholds are provisional. |
| `ad-servers.md` | Not built. Planned in three phases; see the tickets. |
| `browserbase.md` | Built on 2026-11-13, with the owner's decisions of that day overriding the plan: no fail-closed probe, Browserbase's defaults (CAPTCHA solving, video, log) on, residential locations and manual visits included. `HANDOVER.md`, "Browserbase", is the description of what is built. |
| `browserbase-measurements.md` | The spike against the real service that the plan asked for: measured behaviour, not a plan. |
