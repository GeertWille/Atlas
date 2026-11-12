# Jev in the AdTech Scanner: strategy for best findings at least cost

Status: design only, no repo changes. Written 2026-11-12.

Every number is tagged: **[C]** read from the code, **[M]** measured here with throwaway scripts,
**[D]** from the TypeSafe docs, **[A]** assumption. Nothing was run against a real Jev or Anthropic
endpoint, so all model-quality statements and all Phase 2 trajectories are **[A]**.

Experiment files (same folder): `sim.py` (knowledge-base verdicts on the demo site), `chunk.py`
(chunker and lexical-anchor density), `cost.py` (Phase 2 cost model), `oxcfmt/` (the same oxc 0.158
parser and printer the app's beautifier uses), `realjs/` and `otherjs/` (public libraries fetched from
jsdelivr and vendor CDNs), `docs/` (TypeSafe pages).

---

## 0. The short version

1. **Swapping Haiku for Jev like-for-like saves about $0.004 per scan**, under 1% of the budget.
   Phase 1 costs about half a cent today and does not run at all on the demo site. The saving the
   user wants cannot come from the classifier's own price.
2. **Phase 2 is 97-99% of spend.** In the model below a heavy page costs $0.33-0.45 of Sonnet. About
   45% of that is cache writes of tool results, 25-45% is output tokens, and the rest is re-reading
   the growing conversation. Each exploratory turn costs about $0.017.
3. So Jev's value is in **steering Phase 2**: a ranked "start here" map of the publisher's ad logic,
   and keeping stock vendor libraries out of the way. Modelled effect on a heavy page: $0.34 -> about
   $0.17, or about $0.27 if the map is wrong half the time. Jev's own cost for this is about $0.012.
4. **Two deterministic changes need no Jev and should ship first**: scope `search_scripts` away from
   stock Prebid, and put a small wrapper straight into the opening message. They are the baseline
   that Jev has to beat. I cannot tell from here how much Jev adds on top; section 5 says how to find out.
5. **Safety rule**: Jev may rank and promote, but it may only drop scripts from the class Haiku can
   drop today (knowledge-base "ambiguous"). A script with a strong ad signature is never dropped by a
   probability.

---

## 1. Where the money goes today

### 1.1 Facts from the code

| Item | Value | Source |
|---|---|---|
| Phase 1 runs only for scripts the knowledge base leaves "ambiguous" (weak identifier, no strong signature, not a known host or file) | - | `domain/knowledge.rs`, `graph.rs` [C] |
| Phase 1 excerpt per script | head 1,500 + middle 700 + tail 700 = 2,900 chars | `filter.rs` [C] |
| Phase 1 batching | up to 25 scripts and 60,000 chars per call; `max_tokens` = 3,000 + 120 per script; effort low; no caching | `filter.rs` [C] |
| Haiku price | $0.10 in / $0.50 out per Mtok (5x above 100K prompt tokens) | `pricing.rs` [C] |
| Sonnet price | $2 in / $10 out / $0.10 cache read / $2.50 cache write per Mtok | `pricing.rs` [C] |
| Phase 2 set | categories Wrapper, **Prebid** and Unknown that are ad-related; max 12 scripts, 4 MB; ordered wrapper first, first-party first, then **capture order** | `scripts.rs`, `graph.rs` [C] |
| Phase 2 loop | 12 / 20 / 30 turns for low / medium / high effort (default medium); 8,000 output tokens per turn; whole conversation cached and re-read each turn | `analyze/mod.rs` [C] |
| `read_script` | at most 250 lines per call, lines cut at 300 chars, `"%5d| "` prefix | `toolbox.rs` [C] |
| `search_scripts` | first 40 matching lines, each with 2 lines of context, searched in script-index order across **all** Phase 2 scripts when `script_id` is null; output capped at 24,000 chars | `toolbox.rs` [C] |
| Token estimate used for budget checks | chars / 3 | `pricing.rs` [C] |
| Unknown model ids are priced at $15 / $75 per Mtok | - | `pricing.rs` [C] |

### 1.2 Measurements

Beautified with the app's own formatter library (oxc 0.158) [M]:

| Script | Bytes | Lines | Chars of one 250-line `read_script` | Tokens per read (chars / 3.5) |
|---|---|---|---|---|
| demo `hb-wrapper.min.js` (the 4 planted bugs) | 5,098 | 302 | 8,000 | 2,300 |
| demo `prebid.js` stand-in | 21,849 | 632 | 10,100 | 2,900 |
| real Prebid 11.49, core + 19 adapters + 9 modules (assembled from the published chunks) | 407,407 | 15,622 | 11,000 | 3,100 |
| real Prebid 11.49, all modules | 3,907,583 | 170,017 | 10,600 | 3,000 |
| real `gpt.js` loader | 138,572 | 5,082 | 8,400 | 2,400 |
| real `apstag.js` | 368,645 | 15,115 | 10,500 | 3,000 |

Other measurements [M]:

- **Demo site: Phase 1 is skipped.** All 8 scripts are decided by the knowledge base (wrapper by
  strong signatures, `prebid.js` and `gpt.js` and `apstag.js` by name, the rest have no identifiers).
  `e2e-tauri/ai-scan.spec.ts` asserts the same. Phase 2 gets two scripts: the wrapper (302 lines) and
  the Prebid stand-in (632 lines).
- **Vendor-library noise in search.** Lines matching `timeout` (case-insensitive): 85 in the realistic
  Prebid build, 8 in the demo wrapper. `refresh`: 31 vs 2. `adUnits`: 35 vs 1. `requestBids`: 17 vs 2.
  With the 40-match cap and index-order search, a Prebid file captured before the wrapper can fill
  the whole result for `timeout` and push the wrapper's lines out. This costs money (a 2.6K-token
  result of library internals) and quality (the model never sees the publisher's line).
- **Phase 1 coverage.** For a 665 KB script (video.js, which the knowledge base would mark ambiguous
  because 4 of its 131 chunks contain a weak identifier), Haiku sees 2,900 chars: 0.4% of the body,
  and none of it chosen for relevance.
- **Lexical anchors are sparse outside ad code.** Share of ~160-line chunks containing any knowledge
  base identifier (ignoring the 3-letter `cpm`): react-dom 0/37, lodash 0/19, hls.js 0/92, video.js
  4/131, a GTM container 9/73, real `gpt.js` 8/28, real `apstag.js` 44/88, realistic Prebid 84/93.
- Beautifying is cheap: 3.9 MB parsed and printed in 0.18 s (release build). Beautified text is about
  1.45x the minified size.

### 1.3 Token flow of one scan

**Phase 1 today, heavy page [A]**: 100 scripts, of which about 45 known hosts, 15 strong-signature
wrappers (including inline `googletag.cmd.push` snippets), 20 with no identifiers, 20 ambiguous.
20 x about 3,300 chars = 66K chars -> 2 calls -> about 23K input tokens ($0.0023) plus 3-5K output
tokens including thinking ($0.0015-0.0025). **About $0.005.** The budget check reserves about $0.005
per call (worst case, [C]).

**Phase 2 today**: the prefix (system prompt 3,421 chars, six tool definitions about 6,200 chars
[M]) is about 2.9K tokens [A]. The opening message is about 1.8K tokens on the demo and about 6K on a
heavy page [A]. Then per turn, as the code bills it [C]:

- everything already in the conversation is a cache read ($0.10/Mtok),
- the last tool results are a cache write ($2.50/Mtok),
- thinking plus the tool call is output ($10/Mtok), and comes back as cache read on later turns.

Modelled trajectories (`cost.py`; the tool mix and 450-850 output tokens per turn are [A]):

| Scenario | Turns | Cache write | Cache read | Output | Phase 2 total |
|---|---|---|---|---|---|
| Demo today | 12 | $0.038 | $0.016 | $0.072 | **$0.13** |
| Heavy today (hits the 20-turn cap) | 20 | $0.150 | $0.074 | $0.106 | **$0.33** |
| Heavy today, if thinking doubles output per turn | 20 | $0.150 | $0.084 | $0.212 | **$0.45** |

Unit costs that drive everything below:

- one more exploratory turn at 40K context: **$0.017** (read $0.004 + write of a 3K result $0.0075 + output $0.0055)
- one 250-line read, written once and re-read for 10 turns: **$0.011**
- one finding (about 850 output tokens): $0.0085

The largest unknown is output tokens per turn at medium effort with adaptive thinking. The first
real run settles it; the per-call usage is already logged.

### 1.4 Jev price facts [D]

$0.042 per Mtok input, output free. 64K tokens per request, 32K per question; the state is read and
billed once per request, each question adds its own tokens. Any number of questions. Long-context
header raises limits to 262K but is "slower and less accurate": do not use it. Models `jev-1.14.0`,
`jev-1.13.0`; aliases move, so pin the versioned id. 429 and 529 are retryable; gzip accepted. Jev
is literal, weak at indirection and counting, loses accuracy with irrelevant state, leans to the
first Choice option, and can be moved by text that argues for its own classification. Not trained on
customer data; zero retention is an enterprise option.

Jev's tokenizer is unknown. I use chars / 3 (the app's own conservative estimate). The response's
`usage.input_tokens` gives the real ratio on the first call.

---

## 2. Options: cost table and evaluation

### 2.1 Cost per scan

| Option | Demo P1 | Demo P2 | Demo total | Heavy P1 | Heavy P2 | Heavy total |
|---|---|---|---|---|---|---|
| Today | $0 | $0.13 | **$0.13** | $0.005 | $0.33 (to $0.45) | **$0.34 (to $0.45)** |
| A0. Like-for-like swap (same excerpts to Jev) | $0 | $0.13 | $0.13 | $0.001 | $0.33 | $0.33 |
| A. Jev sees whole bodies of ambiguous scripts, naive | $0 | $0.13 | $0.13 | about $0.05 | $0.33 | $0.38 (worse) |
| A+B. Jev on anchor chunks + probability routing | $0 | $0.13 | $0.13 | $0.006 | $0.33 | $0.34 (recall up) |
| D0. Deterministic vendor demotion, no Jev | $0 | about $0.12 | $0.12 | $0.005 | $0.25 | **$0.25** |
| C+D. Jev map + start-here pack + vendor demotion | $0.0003 | $0.07 | **$0.07** | $0.012 | $0.16 | **$0.17** |
| C+D, map wrong half the time | - | - | - | $0.012 | $0.26 | $0.27 |

P1 figures for Jev: A naive = 20 ambiguous scripts x 120 KB [A] x 1.45 / 3 = 1.16M tokens. Anchor
chunks = about 10% of that [M for density, A for the page]. C+D = ambiguous anchors 130K + wrapper
chunks 80K + Prebid mixed-bundle check 21K + question overhead 53K = about 285K tokens = $0.012.

Row D0 and row C+D both rest on assumed trajectories. Treat the ordering as reliable and the
magnitudes as a hypothesis for section 4.

### 2.2 (a) Script triage on far more of each script

- **What**: beautify the candidates, cut into chunks, send only chunks that contain a lexical anchor,
  aggregate chunk answers per script.
- **Why not "whole script when it fits"**: a script fits the 32K-token question limit up to about 65 KB minified [A], but the docs say accuracy falls
  as unrelated state grows, and the naive version costs 10x what Haiku costs today. Small chunks cost
  the same per token, are the accuracy-friendly shape, and their answers double as the map for (c).
- **Chunk size**: target 160 beautified lines, cut at the shallowest-indented line between line 120
  and 200, hard cap 8,000 chars (about 2K-2.7K tokens). One chunk always fits one `read_script` call.
  [M]: the demo wrapper becomes 2 chunks, realistic Prebid 93.
- **Aggregation**: script score = **maximum** chunk probability, plus the count of chunks at or above
  0.5 for the log. Not noisy-OR: with 100 chunks at 0.05 each it would report near certainty. Not the
  mean: one wrapper function inside a 1 MB bundle would vanish.
- **Cost**: neutral (about $0.006 vs $0.005). **Quality**: every region with an ad identifier is
  judged instead of a 2.9K-char sample; this is where recall on wrappers buried in bundles should
  improve. **Risk**: Jev on mangled minified code is untested; beautified code helps but names stay
  `e, t, i`. **Complexity**: medium (chunker, port, adapter, routing).
- Scripts over the formatter's 3 MB limit, or that time out, come back unformatted (often one
  enormous line) [C]. Chunk those by characters, and note that Phase 2 cannot address them by line
  today either: `read_script` shows 300 chars of such a line. That is an existing blind spot.

### 2.3 (b) Probability routing

Rules, in order. `P_ad` = max chunk probability of the `ad_orchestration` question.

| Rule | Condition | Action |
|---|---|---|
| R0 `kb-vendor` | known host or file (ad server, bidder, CMP, analytics, CDN) | unchanged, never sent to Jev |
| R1 `kb-strong` | knowledge base says Wrapper | always analysed; Jev only ranks and maps it |
| R2 `kb-prebid` | knowledge base says Prebid library | see (d) |
| R3 `jev-yes` | ambiguous, `P_ad` >= 0.50 | analyse |
| R4 `jev-no` | ambiguous, `P_ad` < 0.15 on every screened chunk | drop |
| R5 `jev-unsure` | ambiguous, 0.15 <= `P_ad` < 0.50 | include, ranked last; if the script is over 100 KB, ask Haiku first, with Jev's top 3 chunks as the excerpt instead of head/middle/tail |
| R6 `fallback` | Jev unavailable, failed, or its sub-budget is spent | today's Haiku path, unchanged |

Why the thresholds sit below the docs' 0.3 / 0.7 starting points: the two errors cost very different
amounts. A wrongly included script costs one index line and perhaps one wasted search or read, about
$0.01-0.02, and less once the map exists. A wrongly dropped wrapper loses the findings the scan
exists for. Include when `p x (value of a missed wrapper) > (1 - p) x $0.02`, which puts the break-even
far below 0.15; 0.15 is a practical floor that keeps the 12-script cap from filling with junk. Both
numbers are starting points. Tune `jev-no` on the labelled corpus as the largest value with zero
dropped wrappers, minus a margin, against a pinned model id.

Also change the Phase 2 selection order from capture order to score order. Today 12 small inline
`googletag.cmd.push` snippets that appear before the real wrapper would fill the cap (inferred from
`select_for_analysis`, not observed).

Risk: a page can lower its own scores with text aimed at the model [D]. R1 limits the damage: Jev has
no power over strong-signature scripts.

### 2.4 (c) Chunk-level map for Phase 2: the main lever

**What**: for every chunk of the Phase 2 scripts that has an anchor, ask Jev eight Nouls in one
request (speculative fan-out: the chunk is billed once, each question about 80 tokens). Combine in
code into a rank, and give Phase 2:

1. a **map** in the opening message: the top 20 ranges, one line each, e.g.
   `s4 L118-262  auction gating 0.94, device detection 0.88, lazy load 0.71`
   (about 600 tokens);
2. a **start-here pack**: the top-ranked ranges inlined as numbered source, up to 5,000 tokens. A
   wrapper under about 400 lines goes in whole;
3. `search_scripts` results **ordered by chunk rank** instead of capture order, so the 40-match cap
   keeps the best matches.

Rank = `(0.3 + 0.7 x P(publisher_specific)) x (0.35 gating + 0.20 device + 0.15 timing + 0.15 lazy +
0.10 slots + 0.05 consent)`. Weights live in code and are a first guess.

Three sources feed the map, cheapest first:

- **Telemetry literals (free, deterministic)**: chunks containing an ad unit code, slot element id or
  GAM path that the capture recorded. Stock library code never contains `div-gpt-ad-leaderboard`.
- **Lexical anchors (free)**: the knowledge base's identifiers plus `IntersectionObserver`,
  `matchMedia`, `innerWidth`, `userAgent`, `scroll`.
- **Jev (about $0.01)**: decides which anchored chunks are real orchestration, tags the aspect, and
  ranks. This is the only layer that can tell `timeout` in a bidder adapter from `timeout` in the
  publisher's config.

**Cost effect**: fewer exploratory turns ($0.017 each) and narrower reads. Modelled: heavy $0.33 ->
$0.16; demo $0.13 -> $0.07 because the 302-line wrapper is in the first message and the model can
report straight away. The pack costs at most 5K x $2.50/Mtok = $0.0125 if it is entirely wrong, less
than one wasted turn.

**Quality effect**: the model starts at the gating and device code instead of spending the 20-turn
cap finding it. On heavy pages today the cap or the budget likely ends the analysis before all
symptoms are covered [A]; saved turns become coverage.

**Interactions**:

- *Beautification*: the map must use the same line numbers as `read_script`, so chunking runs on the
  beautified lines. Either the filter step beautifies its candidates through the existing
  `SourceFormatter` port and keeps them for the Beautify step, or Beautify moves before Filter.
  The first avoids changing the step list in the data contract.
- *Prompt caching*: keep the system prompt and tool definitions byte-identical; put the map and pack
  in the opening user message, which is per-scan anyway. Do not add a tool for the map in the first
  version (a new tool changes the cached prefix once and adds a turn to call it).
- *Citation check*: unchanged. `verify_citation` matches the excerpt against the full beautified
  script, so code quoted from the pack verifies exactly like code from `read_script`. Pack lines must
  be rendered with the same `numbered()` function so line numbers agree.
- *Audit trail*: the pack is captured source. Log it through the same excerpt budget as tool results
  (800 chars per item, 24,000 per scan [C]), never in full.

**Risk**: anchoring. A confident wrong map can make the model stop early. Mitigation in the prompt
text: "The map is a hint produced by a classifier and can be wrong or incomplete; use search when the
telemetry is not explained." Measure with the "map half wrong" case, which still costs less than today.

**Complexity**: medium-high. It needs (a) first.

### 2.5 (d) Vendor libraries

Yes: demote them, and decide per chunk, not per script.

- **Deterministic part (no Jev)**: scripts the knowledge base classifies as Prebid library stay
  readable by explicit `script_id` but are left out of `search_scripts` when `script_id` is null, out
  of the pack, and last in the index. The search result says `17 more matches in vendor library s7
  hidden; pass script_id to see them`. The system prompt already tells the model that vendor code is
  rarely where the bug is; this makes the tools agree with it.
- **Why per chunk**: publishers often ship Prebid and their config in one file. The knowledge base
  marks the whole file Prebid (two library markers suffice [C]). Demoting the whole file would hide
  the publisher's ad units and `setConfig`. So for Prebid-category scripts, screen the first two and
  last two chunks, plus any chunk with a telemetry literal or a call-site anchor (`pbjs.que`,
  `.addAdUnits(`, `.requestBids(`, `googletag.cmd`). A chunk with `P(ad_orchestration) >= 0.5` and
  `P(publisher_specific) >= 0.5`, or with a telemetry literal, is promoted into the map and the
  default search scope; the rest of the file stays demoted. Cost: about 10 chunks, $0.001. Screening
  the whole 400 KB library would cost $0.0085 and is not needed.
- Telling them apart: knowledge base first (host, file name, library markers), telemetry literals
  second, the `publisher_specific` Noul third. A hash list of official Prebid releases is not
  practical because every publisher build is a different module set.
- **Cost**: D0 alone is modelled at -25% on a heavy page. **Risk**: a real bug inside a vendor library
  (a patched Prebid fork) is harder to find; it stays reachable by id. **Complexity**: low.

Do not summarise vendor libraries with a model. The telemetry already carries Prebid version, module
list and the `getConfig` snapshot, which is the summary Phase 2 needs.

### 2.6 (e) Jev answers as heuristic or ranking signals

Out of scope for now. The 18 rules are deterministic and work with no key; mixing in a probabilistic
signal from an optional provider would make findings differ by which keys are set. Two cheap
neighbours are worth noting for later:

- Use the heuristic findings to **weight the map's aspects** in code (a "bids after scroll" finding
  raises gating, device and lazy-load weights). No extra model call.
- A Jev **check of AI findings** after Phase 2 ("does the cited code support this claim", as in the
  docs' citation-check cookbook) at about $0.00005 per finding. Logic bugs need multi-step reasoning,
  which the docs list as a weakness, so it may only catch gross mismatches. Experiment, not plan.

### 2.7 Other ideas

- **Verdict cache by content hash.** Store `hash + question-set version + model id -> probabilities`
  in SQLite. Numbers only, no source. Repeat scans of a site, and stock library chunks, cost nothing
  in Phase 1. Low complexity.
- **Dedupe identical bodies** within a scan before any model call (the demo loads `gpt.js` twice).
- **Effort by symptoms.** When no deterministic rule fired and the telemetry is healthy, run Phase 2
  at low effort (12 turns). Not a Jev matter; mentioned because output tokens are 25-45% of spend.

---

## 3. Audit trail without model reasons

Jev returns numbers, so the explanation has to be built from what the code did. Log four things in
the FilterScripts step, using the existing `LogEntry` (message is full-text indexed, and so is
`data`):

1. **Question set, once per step** (`LlmRequest`): model id, a version tag (`jev-triage-v1`), and the
   full text of every question's instructions and criteria. This is our own text, safe to store whole.
2. **One line per Jev request** (`LlmRequest` / `LlmResponse`): script id, URL, line range, chunk
   chars, why the chunk qualified (`anchors: requestBids, innerWidth` or `telemetry literal:
   ad-leaderboard`), then the answers. Keep at most 300 chars of the chunk, under a per-scan excerpt
   budget like the existing 24,000-char one. Probabilities are stored in full in `data`:
   `{"scriptId":"s4","lines":[118,262],"answers":{"ad_orchestration":0.97,"publisher_specific":0.91,...},"inputTokens":2310,"modelAnswered":"jev-1.14.0"}`.
3. **One decision line per script** (`Message`, level info):
   `s14 app.3f9c.js -> DROP by jev-no: max P(ad_orchestration) 0.04 < 0.15 over 6 screened chunks of 131 (top: L4410-4571 0.04)`.
   `data` carries rule id, thresholds, every input to the rule, the knowledge-base verdict, and the
   outcome, so a search for `jev-unsure`, a script id or a URL finds it.
4. **Step summary**: counts per rule, tokens, cost, fallbacks taken.

`ScriptRecord.classification_reason` (persisted, shown in the UI) is generated from the rule, for
example: `Jev: ad orchestration 0.93 in lines 118-262 (auction gating 0.94, device detection 0.88);
rule jev-yes (>= 0.50)`. `classification_confidence` is the probability.

The map and pack given to Phase 2 are logged with the Phase 2 request: the map in full (line ranges
and numbers), the pack as excerpts only.

A developer can then answer "why was this script dropped / why did the model start here" from the
log alone: which chunks were asked about, which question, what number, which threshold, which rule.

---

## 4. Evaluation plan

### 4.1 Here, without a key: mock TypeSafe server

Add `ADSCAN_TYPESAFE_BASE_URL` and `ADSCAN_TYPESAFE_API_KEY`, mirroring the existing Anthropic
overrides, with the same https-unless-loopback rule. The mock (wiremock in Rust tests, and a
`mock-typesafe.ts` next to `mock-anthropic.ts` for the e2e suite) answers `/v1/systemone` and
`/v1/models` in these modes:

| Mode | Answers | Proves |
|---|---|---|
| `lexical` | probability from anchor counts in the state | end-to-end plumbing, map rendering, line numbers |
| `oracle` | from a label file keyed by chunk hash | routing and thresholds give the intended result when the model is right |
| `all-zero`, `all-one`, `all-half`, seeded random | constant or noise | Jev cannot drop a strong-signature wrapper; unsure band behaves; no crash |
| `errors` | 401, 429 with `retry-after`, 529, 400 `max_tokens_exceeded`, timeout | Haiku fallback, the scan never fails because of Jev, budget is not charged for failed calls |

Also testable now with no model at all: chunker properties (every line in exactly one chunk, each
chunk fits one `read_script`), aggregation, budget accounting for a zero-output-price model, log
contents (no chunk longer than the excerpt limit is ever stored), and the D0 search scoping.

### 4.2 Labelled corpus available offline

| Sample | Label | Count |
|---|---|---|
| Demo site scripts, including `hb-wrapper.src.js` as a readable variant | wrapper 2, stock 4, app 4 | 10 |
| Real Prebid 11.49 chunks (core, adapters, modules), `gpt.js`, `apstag.js` | stock ad library | about 30 files, about 220 chunks |
| react-dom, lodash, hls.js, video.js, a GTM container | not ad (video.js and GTM are the hard negatives: they contain weak identifiers) | 5 files, about 350 chunks |
| Synthetic: demo wrapper appended to realistic Prebid; demo wrapper inserted into the middle of react-dom; wrapper with aliased globals (`window["pb" + "js"]`) | wrapper inside a bundle | 3 or more |

Chunk-level ground truth for the map comes from the demo README: the four planted bugs sit at known
lines of the wrapper (`isMobileViewport`, `startAnchor`, the `timeout || 250` read, `ensureLibrary`
and the `ad-sidebar-2` unit).

This corpus is small and has one real wrapper. It tests the machinery and gives a first read on stock
versus publisher code. It cannot establish recall on real publisher wrappers.

### 4.3 Metrics

1. **Wrapper recall** at script level: share of labelled publisher wrapper scripts routed to
   analysis. Target 100% on the corpus; this is the gate.
2. **Bug-line coverage at k**: share of ground-truth bug lines inside the top-k map ranges (k = 5, 10)
   and inside the pack.
3. False-include rate and bytes wrongly sent to Phase 2 (cost side).
4. Stock-versus-publisher accuracy per chunk.
5. Phase 2 outcome per scan, from the existing usage logs: turns, tokens by type, cost, findings
   that pass the citation check, planted bugs found (demo: out of 4).

### 4.4 First real runs on the Mac

1. Demo site, three times each: today's pipeline, D0 only, C+D. Compare metric 5. The demo has a
   small wrapper, so it mostly tests the pack.
2. Three to five real publisher pages the user knows well, same three configurations. Pages change
   between loads, so compare cost and turn counts across repeated runs, and judge findings by hand.
3. Record measured chars per token for Jev and output tokens per Sonnet turn; replace the [A] values
   in section 1.3.
4. Sweep on the corpus: chunk target 80 / 160 / 320 lines; beautified versus minified chunks; one
   chunk per request versus many chunks in one state. The whole sweep costs well under $0.10.
5. Tune `jev-no` and `jev-yes`, then pin the model id that was tuned against.

**Side-by-side mode (cheap to build)**: a developer setting `compare classifiers`. When both keys
exist, both classifiers run on the ambiguous set through the same port; the primary one decides, and
the log gets one line per script:
`compare s14: jev=DROP (0.04, jev-no) | haiku=adRelated:false (0.80, "UI bundle, cpm appears in a string") | agree`
plus a step summary (`agreement 18/20; Jev-only includes: s3; Haiku-only includes: s9`). Extra cost
about $0.005 per scan. It also gives Haiku's sentence as a human-readable second opinion next to
Jev's numbers.

**Labelling real pages needs a decision from the user.** Script bodies are never persisted. Options:
label by content hash and URL only (bodies are re-captured on the next scan), or an explicit
developer-only switch that writes captured bodies to a folder the user names. The second is a
deliberate exception to the no-raw-payload rule and should not exist in release builds.

---

## 5. Recommendation

### 5.1 Build order

**Phase A, now, no Jev involved (largest sure win, lowest risk)**

1. `search_scripts` with null `script_id` skips Prebid-library scripts and says how many matches it
   hid. Order the Phase 2 index and search by relevance (wrapper, first-party, size) instead of
   capture order.
2. When the publisher scripts total under about 400 beautified lines, inline them in the opening
   message.
3. Dedupe identical bodies.

Expected: heavy $0.34 -> about $0.25; demo $0.13 -> about $0.07 (the demo wrapper qualifies for 2).

**Phase B, now, the requested feature: Jev as optional Phase 1**

4. TypeSafe port and adapter, key in the keychain, Jev prices in the price table, sub-budget.
5. Anchor-chunk triage with rules R0-R6, score-ordered selection, Haiku as fallback and as the
   escalation for large unsure scripts.
6. Audit logging as in section 3; compare mode; mock server.

Expected cost effect: none to speak of. Expected quality effect: better recall on wrappers inside
bundles, to be confirmed with metric 1.

**Phase C, next, once B's answers exist for every chunk anyway**

7. Map and start-here pack in the Phase 2 opening message; chunk-ranked search results.
8. Mixed-bundle check for Prebid-category files.
9. Verdict cache by content hash.

Expected: heavy about $0.17 (about $0.27 if the map is poor). Ship behind a setting until metric 2
and metric 5 show it beats Phase A alone.

**Later / experiments**: aspect weights from heuristic findings; Jev check of AI findings; effort by
symptoms.

### 5.2 Defaults

| Setting | Default | Basis |
|---|---|---|
| Model | `jev-1.14.0` (pinned, not `jev-latest`) | [D] aliases move |
| Chunk | 160 lines target, cut window 120-200, 8,000 char cap, no overlap | [M] fits one read; [A] accuracy |
| Requests | one chunk per request, all questions in it, 8 concurrent, retry 429/529 with backoff, 4 attempts | [D]; rate limits unknown |
| `jev-yes` | `P_ad` >= 0.50 | [A], recall-biased |
| `jev-no` | `P_ad` < 0.15 on all screened chunks | [A], recall-biased |
| Haiku escalation | unsure and script over 100 KB | [A] |
| Jev sub-budget | $0.03 per scan (about 700K tokens), charged to the one scan budget; when spent, remaining scripts take the fallback rule | [A] |
| Map | top 20 ranges with rank >= 0.30 | [A] |
| Pack | 5,000 tokens; whole script if under 400 lines | [A] |
| Mixed-bundle promotion | `ad_orchestration` >= 0.50 and `publisher_specific` >= 0.50, or a telemetry literal | [A] |

### 5.3 Question drafts

State for every request (small on purpose):

```json
{ "script": { "url": "...", "firstParty": true, "lines": "118-262 of 302" }, "code": "<chunk, beautified>" }
```

All are Nouls. Written literally, one judgment each, yes-meaning stated first, as the docs advise.

| Id | Instructions | `true` | `false` |
|---|---|---|---|
| `ad_orchestration` | Does the JavaScript in `code` call or configure an advertising stack for a web page? | The code itself calls functions such as requestBids, addAdUnits, setConfig, defineSlot, display, refresh, setTargetingForGPTAsync or fetchBids, or defines the ad units, bidders or slot sizes of a site. | The code is unrelated to advertising, or only mentions an advertising word inside a string, comment, URL or analytics event without calling or configuring an ad stack. |
| `publisher_specific` | Is the JavaScript in `code` written or configured for one specific website, rather than being a generic library that is identical on every site that uses it? | It contains site-specific values: ad unit codes or paths, element ids of ad containers, bidder account or placement ids, page-specific conditions, or glue code that wires libraries together for one site. | Generic reusable library internals: adapter implementations, utilities, polyfills or framework code with no site-specific identifiers. |
| `auction_gating` | Does the code in `code` decide whether or when an ad auction or ad request starts? | A condition, event listener, timer, promise or callback controls when requestBids, fetchBids, refresh or display is called. | Ad functions are called unconditionally, or no ad auction or ad request is started in this code. |
| `device_detection` | Does the code in `code` determine the visitor's device type, screen size or viewport? | It reads userAgent, innerWidth, screen size, matchMedia, touch support or a mobile/desktop flag and branches on the result. | It does not inspect the device, screen or viewport. |
| `timing_config` | Does the code in `code` set a timeout or delay for ad auctions or ad loading? | It sets bidderTimeout, an auction timeout or a failsafe timer, or passes a number of milliseconds to requestBids, setTimeout or a similar call in ad-loading code. | No timeout or delay for ads is set here. |
| `lazy_load` | Does the code in `code` wait until an ad slot is near the visible area, or until the user scrolls, before requesting ads? | It uses IntersectionObserver, a scroll listener, getBoundingClientRect or enableLazyLoad to trigger ad requests. | Ads are not tied to visibility or scrolling in this code. |
| `slot_definition` | Does the code in `code` define ad slots or ad units? | It lists ad unit codes, slot element ids, ad unit paths, sizes or the bidders for each unit, or calls defineSlot or addAdUnits. | No ad slot or ad unit is defined here. |
| `consent_gating` | Does the code in `code` wait for or check user consent before running ad code? | It calls __tcfapi or a consent platform callback, or reads a consent cookie or flag, and ad code depends on the result. | No consent check controls ad code here. |

Triage uses only `ad_orchestration`. The other seven are speculative: asked in the same request, used
for the map. If Phase C is not enabled, send only the first two.

### 5.4 Layering

- **Domain (pure)**: chunker over beautified lines; anchor detection (extends `knowledge.rs`
  identifiers); routing (`signals + thresholds -> decision + rule id`); map ranking; Jev rates in
  `pricing.rs` for `jev-1.14.0`, `jev-1.13.0` and both aliases. Without the price entries a Jev call
  is billed at the $15/Mtok fallback [C] and the $0.50 budget is gone after about 33K tokens.
- **Application**: a `DecisionModel` port (`state`, typed questions -> answers, answering model id,
  usage), separate from `LlmClient` because the shapes share nothing. A `ScriptClassifier`
  abstraction with the current Haiku code as one implementation and Jev as the other, which is also
  what makes compare mode nearly free. `SecretStore` today holds exactly one key [C]; it needs a
  second named secret. `ScanJob` carries the optional TypeSafe key in memory only.
- **Infrastructure**: reqwest adapter for `/v1/systemone` and `/v1/models` (key validation on save,
  like Anthropic), bearer auth, gzip, retry policy, the same base-URL safety check.
- **Budget**: every Jev call goes through `record_llm_call`, pre-checked with `can_afford` on
  `chars / 3 x $0.042`. Output is free, so the pre-check is exact up to the token estimate.
- A Jev failure never sets `ai_failure`; it routes to R6.

### 5.5 Expected per-scan cost

| | Before | After Phase A | After Phase A+B+C |
|---|---|---|---|
| Demo site | about $0.13 | about $0.07 | about $0.07 |
| Heavy publisher page | $0.34-0.45 | about $0.25 | about $0.17 (up to $0.27) |

All Phase 2 figures are modelled, not measured. The measured parts are script sizes, lines per read,
match counts, anchor density and the prompt sizes; the prices are from the code and the docs.

### 5.6 Open points and things I am unsure of

- **Whether Jev adds much over the free layers** (telemetry literals + lexical anchors + size-based
  pack). Property names such as `requestBids` survive minification, so a regex map is strong. Jev's
  distinct contribution is ranking, stock-versus-publisher, and aspects with no fixed token. Phase A
  first, then measure the increment, is the way to avoid building C on a hope.
- **Jev on minified JavaScript** is not covered by any doc example. The docs say it does better on
  high-level code than low-level representations.
- **Data leaving the machine.** With Jev on, far more of each candidate script goes to a second
  vendor than the 2.9K-char excerpts Haiku gets today (Phase 2 already sends source to Anthropic on
  demand). Inline scripts can hold user data. Anchored-chunks-only limits this; the settings screen
  should say it plainly. Zero retention at TypeSafe is enterprise-only [D].
- **Rate limits and latency** of TypeSafe for about 100 small requests are unknown; the concurrency
  of 8 is a guess.
- **`cpm` as a weak identifier** is three letters and will match by accident in large minified
  bundles, which makes scripts ambiguous today. I excluded it from anchors in the measurements. Worth
  a separate look.
- The heavy-page script mix (how many ambiguous, how large) is an assumption; the first real scans
  will show it in the existing script table.
