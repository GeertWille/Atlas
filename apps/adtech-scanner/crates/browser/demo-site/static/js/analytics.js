/* Meridian audience analytics: page views, scroll-depth milestones and engagement. First party, no ads. */
(function (win, doc) {
  'use strict';

  var ENDPOINT = '/collect';
  var MILESTONES = [25, 50, 75, 100];
  var reached = {};
  var site = win.__MERIDIAN__ || {};
  var dataLayer = (win.dataLayer = win.dataLayer || []);

  function send(name, properties) {
    var payload = JSON.stringify({
      event: name,
      article: site.articleId,
      section: site.section,
      path: win.location.pathname,
      properties: properties || {},
      sentAt: new Date().toISOString(),
    });
    if (navigator.sendBeacon) {
      navigator.sendBeacon(ENDPOINT, payload);
    } else {
      fetch(ENDPOINT, { method: 'POST', body: payload, keepalive: true }).catch(function () {});
    }
  }

  function onScroll() {
    var page = doc.scrollingElement || doc.documentElement;
    var scrollable = page.scrollHeight - win.innerHeight;
    if (scrollable <= 0) return;
    var percent = (win.scrollY / scrollable) * 100;
    MILESTONES.forEach(function (milestone) {
      if (percent >= milestone && !reached[milestone]) {
        reached[milestone] = true;
        dataLayer.push({ event: 'scroll_depth', depth: milestone });
        send('scroll_depth', { depth: milestone });
      }
    });
  }

  // Engagement: how much the reader moved, scrolled and touched, and whether
  // a person's devices caused it. Reported when the reader pauses and when
  // the page is left.
  var ENGAGEMENT_PAUSE_MS = 3000;
  var engagement = { pointerMoves: 0, wheels: 0, touches: 0, trusted: true };
  var engagementTimer = null;
  var engagementReported = '';

  function reportEngagement() {
    var summary = JSON.stringify(engagement);
    if (summary === engagementReported) return;
    engagementReported = summary;
    console.log('ENGAGEMENT ' + summary);
    send('engagement', engagement);
  }

  function onInput(counter) {
    return function (event) {
      engagement[counter] += 1;
      if (!event.isTrusted) engagement.trusted = false;
      clearTimeout(engagementTimer);
      engagementTimer = setTimeout(reportEngagement, ENGAGEMENT_PAUSE_MS);
    };
  }

  win.addEventListener('pointermove', onInput('pointerMoves'), { passive: true });
  win.addEventListener('wheel', onInput('wheels'), { passive: true });
  win.addEventListener('touchstart', onInput('touches'), { passive: true });
  win.addEventListener('pagehide', function () {
    if (engagement.pointerMoves + engagement.wheels + engagement.touches > 0) reportEngagement();
  });
  win.addEventListener('meridian:route', function (event) {
    send('page_view', { route: event.detail.path });
  });

  // Which events are sampled is decided off the main thread.
  if (win.Worker) {
    var sampler = new Worker('/static/js/analytics-worker.js');
    sampler.onmessage = function (message) {
      site.sampling = message.data;
    };
  }

  win.addEventListener('scroll', onScroll, { passive: true });
  send('page_view', { referrer: doc.referrer || null, viewport: win.innerWidth + 'x' + win.innerHeight });
})(window, document);
