/* Meridian audience analytics: page views and scroll-depth milestones. First party, no ads. */
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

  win.addEventListener('scroll', onScroll, { passive: true });
  send('page_view', { referrer: doc.referrer || null, viewport: win.innerWidth + 'x' + win.innerHeight });
})(window, document);
