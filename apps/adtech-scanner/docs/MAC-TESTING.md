# Mac test list

Everything here was built and tested on Linux and has never been run on macOS, or needs real
accounts, real sites or your network. Work through it top to bottom; sections are ordered so that
a failure early on does not waste time later. Tick items in this file as you go.

For anything that fails, the most useful things to send back are:

- the audit trail of the scan (Report → Audit trail, or Export → JSON)
- a screenshot
- for a crash or a hang: the terminal output of `pnpm tauri dev`

## 0. Setup

- [ ] `cd apps/adtech-scanner && pnpm install`
- [ ] `pnpm tauri dev` opens the app
- [ ] `pnpm tauri build` produces a `.app` and a `.dmg` under `target/release/bundle`
- [ ] Note the commit you are testing: `git rev-parse --short HEAD`

## 1. App shell

- [ ] Traffic lights do not overlap the sidebar or the title
- [ ] Dragging the top strip moves the window; double-click zooms it
- [ ] App icon looks right in the Dock and Finder (built app)
- [ ] Light and dark theme, and no flash of the wrong theme on launch
- [ ] Window at its minimum size (1024×680): nothing is cut off on Home, a report, the network view
- [ ] External links ("Get a key", "Open URL in browser") open your browser
- [ ] Quit the app during a scan: Chrome closes with it
- [ ] Force-quit the app during a scan (Activity Monitor): no Chrome process is left after a few seconds

## 2. Keyboard shortcuts

- [ ] Cmd+N new scan, Cmd+K search, Cmd+, settings
- [ ] Home: Cmd+Enter starts the scan, Cmd+S saves or updates the configuration
- [ ] Script viewer: Cmd+F find (type a word key by key), Cmd+G next match, Ctrl+G go to line, Cmd+P quick-open, Cmd+B file list, Cmd+Alt+B side panel
- [ ] Script viewer: Option+Down / Option+Up go to the next and previous finding (F8 / Shift+F8 do the same, but F8 is a media key on a Mac laptop and needs Fn). Check that Option+Down does not also move the caret or scroll, and that in the find field (Cmd+F) it is left to the field. The "Next (F8)" button of a finding shows both shortcuts when you hover it.
- [ ] Network view: arrow keys move the row, Enter opens details, Esc closes, `[` and `]` change page view, Cmd+F or `/` focuses the filter, Cmd+C copies the URL
- [ ] No shortcut fires while typing in a text field unless it should
- [ ] Focus rings: with the mouse, pick an option in a select (Settings → Visitor), open and close a menu (History → the "…" of a scan), open a dialog (Settings → "Delete all scans…") and click a report tab. None of those controls should show a focus ring afterwards. Then press Tab: the ring must appear. (On Linux only the select showed the ring before the fix; macOS may differ.)

## 3. Keychain

- [ ] First launch after updating: old keychain items are moved into one item (a last round of prompts is expected, one per old item)
- [ ] After that: opening the app, Settings and reports shows no keychain prompt
- [ ] First scan that needs a key: at most one prompt
- [ ] Later scans in the same launch: no prompt
- [ ] Click "Deny": the app says "Keychain access was denied", not "no API key configured"
- [ ] Save, replace and remove the Anthropic key, the TypeSafe key and a proxy password
- [ ] In Keychain Access there is one item for `com.webclew.adtech-scanner`, account `vault`
- [ ] With a signed build (see section 13): click "Always Allow" once; no prompt after the next rebuild

## 4. Finding and driving Chrome

- [ ] Chrome in `/Applications` is detected automatically
- [ ] Settings → Browser → choose `Google Chrome.app` in the file picker works
- [ ] Scan with "Show the browser" on: a Chrome window opens and closes by itself
- [ ] Scan with it off (headless): no window appears
- [ ] Does Chrome show an "unsupported command-line flag" bar in any mode? Note which mode.
- [ ] No "Chrome wants to use your confidential information" prompt (the app passes `--use-mock-keychain`)

## 5. The built-in demo site

- [ ] "Try the demo" scan completes without an API key
- [ ] Findings include: scroll gate on desktop, anchor slot requested before the auction ended, 250 ms timeout, ad unit without a slot, GPT loaded twice
- [ ] Same scan as Mobile: the scroll-gate finding is gone or different
- [ ] Browse like a visitor, 3 pages: three page views appear, and none of the trap links was followed (check the audit trail's Visitor entries)

## 6. Real model APIs

- [ ] Anthropic key saved; an AI scan of the demo site finishes and shows AI findings with code evidence
- [ ] TypeSafe key saved and "Classify scripts with Jev" on; the audit trail shows Jev classification entries with probabilities
- [ ] Turn the TypeSafe key off mid-way (or use a wrong key): the scan falls back to Haiku and says so
- [ ] Cost per scan as shown in the report. Note it for each real site you scan (ticket #4).
- [ ] Budget set to $0.05: the scan stops analysis early and says the budget was reached

## 7. Consent click-through (ticket #3)

For each site: did the scan accept the dialog by itself, and how long after load?

- [ ] bild.de (Sourcepoint, with the PUR subscription option): "Alle akzeptieren" is clicked, the subscription option is not
- [ ] A OneTrust site
- [ ] A Didomi site
- [ ] A Cookiebot site
- [ ] A Usercentrics site
- [ ] When it fails, the report says "consent banner detected, automatic accept failed"

## 8. bild.de specifically

This is the site that exposed most of the real-world problems.

- [ ] Scripted scan (browse, 3 pages): does page 1 now run an auction after consent?
- [ ] If Prebid loads but no auction runs: is there a finding "bids requested without ad units"?
- [ ] Does the AI analysis read the wrapper inside the Prebid bundle (it should no longer say it "didn't inspect" it)?
- [ ] "I'll browse myself" on the same page: compare bidders and auctions with the scripted scan
- [ ] Note: ad slots will be empty in the report, because bild.de uses Xandr and Smart AdServer, which are not read yet (ticket #14)

## 9. Visitor behaviour and manual sessions

- [ ] Observe only, Scroll, Browse like a visitor each behave as described in the composer
- [ ] "I'll browse myself": Chrome opens, you browse, "Finish capture" ends it and produces a report
- [ ] Closing Chrome's last window also ends a manual session with a report (on macOS Chrome stays running after its last window closes; check the app still notices)
- [ ] Cancel during a manual session discards it
- [ ] Cancel a scripted scan while it runs: in the audit trail the step that was running has a grey "cancelled" icon and the summary "Cancelled", not the red "failed" icon; the steps after it read as skipped
- [ ] The scripted visitor never clicks an ad, a sign-in link, a download, or leaves the site (watch one visible scan on a real news site)
- [ ] "Repeat this visit" repeats the same scripted path

## 10. Browser profile, proxies, configurations

- [ ] Settings → keep a browser profile between scans: second scan of a site shows no consent dialog; "Clear" works and is disabled during a scan
- [ ] Two scans at once with a kept profile: the second waits
- [ ] Add a real HTTP proxy with user name and password; a scan through it works and the report shows the proxy name (ticket #6)
- [ ] HTTPS proxy
- [ ] SOCKS5 proxy without credentials (with credentials it is refused on purpose)
- [ ] Same site from two countries: do the bidders differ?
- [ ] Save a configuration, run it from the list and from Cmd+K, edit it, Revert, Update, Save as new, delete
- [ ] Delete a proxy that a configuration uses: refused, with the configuration named

## 11. Stored scan data, script viewer, network view

- [ ] After a real scan, Settings → Stored scan data shows usage; "Clear stored scan data" empties it
- [ ] `~/Library/Application Support/com.webclew.adtech-scanner/archive` is readable only by you
- [ ] Script viewer on a real site's wrapper: scroll a large file, find, jump to a finding, Formatted/Original, quick-open; scrolling is smooth
- [ ] Network view on a real scan (often 1,000–4,000 requests): scrolling and typing a filter feel instant; opening a request's details is quick
- [ ] Bid requests chip: each bid shows a price, a no-bid reason, "late" or "request unknown"; open one and check the decoded request and response make sense
- [ ] Bid exchanges view: compare "late +N ms" cells with what Prebid itself reported as timeouts
- [ ] Bid exchanges view on knack.be (3-page browse), every auction: each ad unit code is one column with all its sizes, every bidder row has its cells under the right ad unit and its request summary under "Request", also after switching between auctions in the menu; "N ad units" matches the number of columns
- [ ] Same view: where an auction has bids in EUR and USD, every price shows its currency code and the line above the table says the prices are not comparable; the "Bidder" heading starts where the bidder names start
- [ ] Headers tab: cookie and authorization values are blanked; with "Store full headers" on and a new scan they are shown
- [ ] Sign in to a site during a manual session, then look at the stored login request: the password field must be blanked
- [ ] Copy as cURL pastes a working command (with a placeholder where something was blanked)
- [ ] Storage switched off: a new scan still shows findings and bid verdicts, and the network view says recording was off
- [ ] Dragging the split between list and details works

## 12. Bidder checks

- [ ] A real site's "never returned a bid" finding names a cause per bidder where one was found (ads.txt, parameters, endpoint reason, consent)
- [ ] ads.txt: pick one bidder the app says is missing and check the site's real ads.txt by hand
- [ ] "Check sellers.json at ad exchanges" is now on by default (also after updating an existing install). First scan of a real site: note the scan time, and how long the "Bidder checks" step took (audit trail); check one reported mismatch by hand
- [ ] Scan the same site again, or another site with the same exchanges: the Bidder checks step should be clearly faster, and its audit trail should say "kept copy, fetched … ago" per exchange instead of "downloaded"
- [ ] Settings → Bidder checks shows the size of the kept sellers.json files and the number of exchanges; "Clear kept files" empties it, and the next scan downloads again
- [ ] A scan through a proxy profile: the sellers.json requests must not show up at the proxy (they go directly, or through the system's proxy); the site's ads.txt must
- [ ] Switch the check off: the next report says "Not looked up for this scan" under Bidder checks
- [ ] `~/Library/Application Support/com.webclew.adtech-scanner/cache/sellers-json` is readable only by you

## 13. Signing and release (ticket #2)

- [ ] Decide: Apple Developer ID or a self-signed certificate (steps in `HANDOVER.md`, "Keychain prompts")
- [ ] `codesign -dv --verbose=4 "AdTech Scanner.app"` shows the identity
- [ ] The signed `.dmg` opens on another Mac without a Gatekeeper block (needs notarization)

## 14. Numbers worth writing down

These calibrate estimates that are currently guesses.

| What | Where to read it | Your value |
|---|---|---|
| Model cost per scan, 5–10 real sites | Report header | |
| Jev: any publisher wrapper dropped, any analytics bundle kept | Audit trail, Classification entries | |
| Requests per scan and archive size per scan | Network view footer; Settings → Stored scan data | |
| Scan time with the sellers.json check on: first scan of a site, and second scan | Report header; audit trail, Bidder checks step | |
| Size of the kept sellers.json files after 5–10 sites | Settings → Bidder checks | |
| Time from pressing Scan to the report | Report header | |
| Network view: time to first rows on a 3,000+ request scan | By eye | |
