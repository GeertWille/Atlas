import { createServer } from "node:http";

const PAGE = `<!doctype html>
<html lang="en">
  <head><meta charset="utf-8" /><title>Fixture publisher</title></head>
  <body>
    <h1>Fixture publisher</h1>
    <div id="ad-top"></div>
    <script src="/lazy-ads.js"></script>
  </body>
</html>`;

/**
 * Names ad concepts without calling any ad library, which is exactly what the
 * built-in knowledge base cannot decide and hands to Phase 1.
 */
const LAZY_ADS_SCRIPT = `function isMobileViewport() {
  return window.matchMedia("(min-width: 768px)").matches;
}
function loadAds(units) {
  return units;
}
if (isMobileViewport()) {
  loadAds(window.adUnits);
}
var cpm = 0;
`;

export const UNDECIDED_SCRIPT = "lazy-ads.js";

export type PublisherPage = { url: string; close(): Promise<void> };

/** A loopback page whose only script has to be classified by the model. */
export async function startPublisherPage(): Promise<PublisherPage> {
  const server = createServer((request, response) => {
    if (request.url === `/${UNDECIDED_SCRIPT}`) {
      response.writeHead(200, { "content-type": "text/javascript" }).end(LAZY_ADS_SCRIPT);
    } else if (request.url === "/") {
      response.writeHead(200, { "content-type": "text/html" }).end(PAGE);
    } else {
      response.writeHead(404).end();
    }
  });
  await new Promise<void>((done) => server.listen(0, "127.0.0.1", done));
  const address = server.address();
  if (!address || typeof address !== "object") throw new Error("the fixture page has no port");
  return {
    url: `http://127.0.0.1:${address.port}/`,
    close: () =>
      new Promise((done) => {
        server.closeAllConnections();
        server.close(() => done());
      }),
  };
}
