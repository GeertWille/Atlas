import type { AdSlot, PageView, ScriptRecord } from "@/bindings";

/** The HTML of a page view as the scanned site served it: its scripts in order, the ad containers, the article. */
export function documentHtml(page: PageView, scripts: ScriptRecord[]): string {
  const tags = scripts
    .filter((script) => script.pages.includes(page.index) || script.pages.length === 0)
    .map((script) =>
      script.inline
        ? "    <script>window.googletag = window.googletag || { cmd: [] }; window.pbjs = window.pbjs || { que: [] };</script>"
        : `    <script async src="${script.url}"></script>`,
    );
  return [
    "<!doctype html>",
    '<html lang="en">',
    "  <head>",
    '    <meta charset="utf-8" />',
    `    <title>${page.title ?? "News"}</title>`,
    `    <link rel="canonical" href="${page.url}" />`,
    ...tags,
    "  </head>",
    "  <body>",
    '    <div id="div-gpt-top-leaderboard" class="ad-slot"></div>',
    "    <main>",
    `      <h1>${page.title ?? "News"}</h1>`,
    "      <article>",
    "        <p>The window closes on Friday and three clubs are still talking.</p>",
    '        <div id="div-gpt-incontent-1" class="ad-slot"></div>',
    "        <p>None of the deals is done, people close to the talks said.</p>",
    "      </article>",
    '      <aside><div id="div-gpt-sidebar-1" class="ad-slot" data-lazy="true"></div></aside>',
    "    </main>",
    '    <footer><div id="div-gpt-footer" class="ad-slot" data-lazy="true"></div></footer>',
    "  </body>",
    "</html>",
  ].join("\n");
}

export function articleJson(page: PageView): string {
  return JSON.stringify({
    id: 481_220 + page.index,
    url: page.url,
    headline: page.title,
    section: "football",
    tags: ["transfers", "premier-league", "deadline-day"],
    paywall: { metered: true, remaining: 3 },
    body: [
      { type: "paragraph", text: "The window closes on Friday and three clubs are still talking." },
      { type: "ad", slot: "div-gpt-incontent-1" },
      { type: "paragraph", text: "None of the deals is done, people close to the talks said." },
    ],
    related: [481_102, 480_977, 480_951],
  });
}

/**
 * Reader comments are whatever readers typed. One tries to inject markup and one hides a
 * right-to-left override, so the body view can be seen to show both as plain text.
 */
export function commentsJson(): string {
  return JSON.stringify(
    {
      article: 481_220,
      total: 3,
      comments: [
        { id: 9001, author: "terrace_tom", text: "Finally a signing that makes sense." },
        { id: 9002, author: "x", text: "<img src=x onerror=\"document.title='owned'\">nice try" },
        { id: 9003, author: "fan‮noitcejni‬", text: "see https://evil.example/claim-prize ​now" },
      ],
    },
    null,
    2,
  );
}

export function creativeHtml(slot: AdSlot): string {
  return [
    "<!doctype html>",
    "<html>",
    "  <head><title>SafeFrame Container</title></head>",
    "  <body>",
    `    <div id="google_ads_iframe_${slot.adUnitPath ?? slot.elementId}_0">`,
    `      <a href="https://adclick.g.doubleclick.net/pcs/click?xai=AKAOjs" target="_blank">`,
    `        <img src="https://tpc.googlesyndication.com/simgad/1" width="${slot.renderedSize?.split("x")[0] ?? 300}" alt="" />`,
    "      </a>",
    "    </div>",
    "  </body>",
    "</html>",
  ].join("\n");
}

/**
 * A JSON document of about `bytes` characters: fixtures of a season, one per line. `cutAt` is where a
 * capture stopped storing it; what is left is then no longer valid JSON, as with a real truncated body.
 */
export function largeFeedJson(bytes: number, cutAt?: number): string {
  const rows: string[] = [];
  let length = 16;
  for (let index = 0; length < bytes; index += 1) {
    const row = `  {"id":${100_000 + index},"home":"Club ${index % 40}","away":"Club ${(index * 7 + 3) % 40}","kickoff":"2026-${String((index % 12) + 1).padStart(2, "0")}-${String((index % 28) + 1).padStart(2, "0")}T15:00:00Z","score":[${index % 5},${(index * 3) % 4}],"attendance":${21_000 + ((index * 977) % 40_000)}}`;
    rows.push(row);
    length += row.length + 2;
  }
  const text = `{"fixtures":[\n${rows.join(",\n")}\n]}`;
  return cutAt === undefined ? text : text.slice(0, cutAt);
}
