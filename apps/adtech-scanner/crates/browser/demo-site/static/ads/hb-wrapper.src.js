/*! adstack 4.2.1 | Meridian Media Group | header bidding orchestration */
(function (win, doc) {
  'use strict';

  var CONFIG = {
    network: '/21804213/meridian-post',
    breakpoints: { mobile: 768 },
    gptUrl: '/tag/js/gpt.js?v=adstack-4.2.1',
    apstagUrl: '/static/ads/apstag.js',
    prebid: {
      auctionTimeout: 1200,
      priceGranularity: 'medium',
      currency: { adServerCurrency: 'USD' },
      userSync: { syncDelay: 3000, filterSettings: { iframe: { bidders: '*', filter: 'include' } } },
    },
    amazon: { pubID: '3391', timeout: 800 },
    failsafeTimeout: 2500,
    // PERF-812: on phones, bidding during first paint cost ~400ms of main-thread
    // time, so phones wait for the reader's first scroll before the auction.
    scrollGatedAuction: true,
    lazyMargin: 250,
    // PAGE-1744: a consent reload started the first auction twice. A start
    // that follows the previous one this closely is the same page again.
    doubleFireGuardMs: 120000,
    // GROWTH-377: readers who stay and are active see a fresh rail ad.
    engagementRefresh: { slot: 'ad-sidebar-1', viewableMs: 5000, activeWithinMs: 5000, checkEveryMs: 1000 },
    // GROWTH-402: a recirculation unit for readers on their second article.
    recirculation: {
      id: 'ad-recirc',
      unit: 'recirc',
      fromPageView: 2,
      sizes: { desktop: [[728, 90]], mobile: [[320, 50]] },
      bidders: ['appnexus', 'rubicon'],
    },
    slots: [
      {
        id: 'ad-leaderboard',
        unit: 'top',
        sizes: { desktop: [[970, 250], [728, 90]], mobile: [[320, 100], [320, 50]] },
        bidders: ['appnexus', 'rubicon', 'ix', 'pubmatic', 'criteo', 'openx'],
      },
      {
        id: 'ad-sidebar-1',
        unit: 'rail-1',
        sizes: { desktop: [[300, 600], [300, 250]], mobile: [[300, 250]] },
        bidders: ['appnexus', 'rubicon', 'pubmatic', 'criteo', 'openx', 'triplelift'],
      },
      {
        id: 'ad-sidebar-2',
        unit: 'rail-2',
        sizes: { desktop: [[300, 250]], mobile: [[300, 250]] },
        bidders: ['appnexus', 'rubicon', 'pubmatic', 'criteo'],
      },
      {
        id: 'ad-inarticle-1',
        unit: 'article-1',
        sizes: { desktop: [[336, 280], [300, 250]], mobile: [[300, 250]] },
        bidders: ['appnexus', 'rubicon', 'ix', 'pubmatic', 'criteo', 'openx', 'triplelift'],
      },
      {
        id: 'ad-inarticle-2',
        unit: 'article-2',
        lazy: true,
        sizes: { desktop: [[336, 280], [300, 250]], mobile: [[300, 250]] },
        inventoryCode: 'MeridianPost_article2_HDX',
        bidders: ['appnexus', 'rubicon', 'pubmatic', 'criteo', 'triplelift'],
      },
      {
        id: 'ad-sticky-footer',
        unit: 'anchor',
        anchor: true,
        sizes: { desktop: [[728, 90]], mobile: [[320, 50]] },
        bidders: ['appnexus', 'rubicon', 'ix'],
      },
    ],
  };

  var BIDDER_PARAMS = {
    appnexus: function (slot) { return { placementId: 28100400 + slot.id.length }; },
    rubicon: function (slot) { return { accountId: 26014, siteId: 518204, zoneId: 2950000 + slot.id.length }; },
    ix: function (slot) { return { siteId: '8842' + slot.id.length }; },
    pubmatic: function (slot) { return { publisherId: '164120', adSlot: slot.unit }; },
    criteo: function () { return { networkId: 7100 }; },
    triplelift: function (slot) { return { inventoryCode: slot.inventoryCode }; },
    openx: function (slot) { return { delDomain: 'meridianpost-d.openx.net', unit: '5412' + slot.id.length }; },
  };

  var googletag = (win.googletag = win.googletag || {});
  googletag.cmd = googletag.cmd || [];
  var pbjs = (win.pbjs = win.pbjs || {});
  pbjs.que = pbjs.que || [];

  var gptSlots = {};
  var session = sessionState();
  var sizeProfile = (win.innerWidth || doc.documentElement.clientWidth) < CONFIG.breakpoints.mobile ? 'mobile' : 'desktop';

  // What this reader's earlier pages left behind: how many there were, and
  // when the last auction started.
  function sessionState() {
    var state = { pageViews: 1, previousAuctionAt: 0 };
    try {
      state.pageViews = (Number(win.sessionStorage.getItem('adstack.pageViews')) || 0) + 1;
      state.previousAuctionAt = Number(win.sessionStorage.getItem('adstack.lastAuction')) || 0;
      win.sessionStorage.setItem('adstack.pageViews', String(state.pageViews));
    } catch (storageBlocked) {}
    return state;
  }

  function isDoubleFire() {
    return session.previousAuctionAt > 0 && Date.now() - session.previousAuctionAt < CONFIG.doubleFireGuardMs;
  }

  function rememberAuction() {
    try {
      win.sessionStorage.setItem('adstack.lastAuction', String(Date.now()));
    } catch (storageBlocked) {}
  }

  function log() {
    if (/[?&]adstack_debug=1/.test(win.location.search)) {
      console.log.apply(console, ['[adstack]'].concat([].slice.call(arguments)));
    }
  }

  function track(name, detail) {
    (win.dataLayer = win.dataLayer || []).push({ event: 'adstack.' + name, detail: detail || null });
  }

  function ensureLibrary(url) {
    if (doc.querySelector('script[src="' + url + '"]')) return;
    var script = doc.createElement('script');
    script.async = true;
    script.src = url;
    doc.head.appendChild(script);
  }

  function installAmazonStub() {
    if (win.apstag) return;
    function queue(command, args) {
      win.apstag._Q.push([command, args]);
    }
    win.apstag = {
      init: function () { queue('i', arguments); },
      fetchBids: function () { queue('f', arguments); },
      setDisplayBids: function () {},
      targetingKeys: function () { return []; },
      _Q: [],
    };
    ensureLibrary(CONFIG.apstagUrl);
    win.apstag.init({ pubID: CONFIG.amazon.pubID, adServer: 'googletag' });
  }

  function isMobileViewport() {
    // Phones are everything below the tablet breakpoint.
    return win.matchMedia('(min-width: ' + CONFIG.breakpoints.mobile + 'px)').matches;
  }

  function sizesFor(slot) {
    return slot.sizes[sizeProfile];
  }

  function toAdUnit(slot) {
    return {
      code: slot.id,
      mediaTypes: { banner: { sizes: sizesFor(slot) } },
      bids: slot.bidders.map(function (bidder) {
        return { bidder: bidder, params: BIDDER_PARAMS[bidder](slot) };
      }),
    };
  }

  function configurePrebid() {
    pbjs.que.push(function () {
      pbjs.setConfig({
        bidderTimeout: CONFIG.prebid.timeout || 250,
        priceGranularity: CONFIG.prebid.priceGranularity,
        currency: CONFIG.prebid.currency,
        userSync: CONFIG.prebid.userSync,
        enableSendAllBids: false,
      });
      pbjs.addAdUnits(CONFIG.slots.map(toAdUnit));
    });
  }

  function defineSlots() {
    googletag.cmd.push(function () {
      CONFIG.slots.forEach(function (slot) {
        if (!doc.getElementById(slot.id)) return;
        var defined = googletag.defineSlot(CONFIG.network + '/' + slot.unit, sizesFor(slot), slot.id);
        gptSlots[slot.id] = defined && defined.addService(googletag.pubads());
      });
      googletag.pubads().disableInitialLoad();
      googletag.pubads().enableSingleRequest();
      googletag.pubads().setTargeting('site', 'meridian-post').setTargeting('layout', sizeProfile);
      googletag.enableServices();
      Object.keys(gptSlots).forEach(function (id) {
        googletag.display(id);
      });
    });
  }

  // Prebid and Amazon bid in parallel; the ad server is called once both are
  // back, or when the failsafe fires.
  function runAuction(slots, label) {
    var codes = slots.map(function (slot) { return slot.id; });
    var waitingFor = { prebid: true, amazon: true };
    var sent = false;

    function sendAdServerRequest(reason) {
      if (sent) return;
      sent = true;
      clearTimeout(failsafe);
      googletag.cmd.push(function () {
        pbjs.que.push(function () {
          pbjs.setTargetingForGPTAsync(codes);
          win.apstag.setDisplayBids();
          var toRefresh = codes
            .map(function (code) { return gptSlots[code]; })
            .filter(Boolean);
          log(label, 'ad server request', reason, codes);
          track('adserver_request', { label: label, reason: reason, slots: codes.length });
          googletag.pubads().refresh(toRefresh);
        });
      });
    }

    function bidsBack(source) {
      return function () {
        waitingFor[source] = false;
        if (!waitingFor.prebid && !waitingFor.amazon) sendAdServerRequest('bids back');
      };
    }

    var failsafe = setTimeout(function () {
      sendAdServerRequest('failsafe');
    }, CONFIG.failsafeTimeout);

    track('auction_start', { label: label, slots: codes.length });
    if (isDoubleFire()) {
      log(label, 'auction skipped: double fire');
      waitingFor.prebid = false;
    } else {
      rememberAuction();
      pbjs.que.push(function () {
        pbjs.requestBids({ adUnitCodes: codes, bidsBackHandler: bidsBack('prebid') });
      });
    }
    win.apstag.fetchBids(
      {
        slots: slots.map(function (slot) {
          return { slotID: slot.id, slotName: CONFIG.network + '/' + slot.unit, sizes: sizesFor(slot) };
        }),
        timeout: CONFIG.amazon.timeout,
      },
      bidsBack('amazon')
    );
  }

  function startAnchor(slot) {
    googletag.cmd.push(function () {
      pbjs.que.push(function () {
        if (!isDoubleFire()) {
          rememberAuction();
          pbjs.requestBids({
            adUnitCodes: [slot.id],
            bidsBackHandler: function () {
              pbjs.setTargetingForGPTAsync([slot.id]);
            },
          });
        }
        googletag.pubads().refresh([gptSlots[slot.id]]);
        track('anchor_request', { slot: slot.id });
      });
    });
  }

  function watchLazySlots(slots) {
    if (!('IntersectionObserver' in win)) {
      runAuction(slots, 'lazy-fallback');
      return;
    }
    var observer = new IntersectionObserver(
      function (entries) {
        entries.forEach(function (entry) {
          if (!entry.isIntersecting) return;
          observer.unobserve(entry.target);
          var slot = slots.filter(function (candidate) { return candidate.id === entry.target.id; })[0];
          runAuction([slot], 'lazy:' + slot.id);
        });
      },
      { rootMargin: CONFIG.lazyMargin + 'px 0px' }
    );
    slots.forEach(function (slot) {
      var element = doc.getElementById(slot.id);
      if (element) observer.observe(element);
    });
  }

  function isHalfInView(element) {
    var box = element.getBoundingClientRect();
    var visible = Math.min(box.bottom, win.innerHeight) - Math.max(box.top, 0);
    return box.height > 0 && visible >= box.height / 2;
  }

  // Once the rail ad has counted as viewed, a reader who keeps it on screen
  // and keeps moving gets a new one.
  function watchEngagementRefresh() {
    var rule = CONFIG.engagementRefresh;
    var lastActiveAt = 0;
    var inViewSince = 0;
    function active() {
      lastActiveAt = Date.now();
    }
    win.addEventListener('pointermove', active, { passive: true });
    win.addEventListener('touchstart', active, { passive: true });
    win.addEventListener('touchmove', active, { passive: true });
    googletag.cmd.push(function () {
      googletag.pubads().addEventListener('impressionViewable', function (event) {
        if (inViewSince || event.slot.getSlotElementId() !== rule.slot) return;
        inViewSince = Date.now();
        var check = setInterval(function () {
          var slot = gptSlots[rule.slot];
          var element = doc.getElementById(rule.slot);
          if (!slot || !element) return clearInterval(check);
          var now = Date.now();
          if (!isHalfInView(element)) inViewSince = now;
          if (now - inViewSince < rule.viewableMs || now - lastActiveAt > rule.activeWithinMs) return;
          clearInterval(check);
          track('engagement_refresh', { slot: rule.slot });
          googletag.pubads().refresh([slot]);
        }, rule.checkEveryMs);
      });
    });
  }

  function startRecirculation() {
    var unit = CONFIG.recirculation;
    if (session.pageViews < unit.fromPageView || !doc.getElementById(unit.id)) return;
    googletag.cmd.push(function () {
      var slot = googletag
        .defineSlot(CONFIG.network + '/' + unit.unit, sizesFor(unit), unit.id)
        .addService(googletag.pubads());
      googletag.display(unit.id);
      pbjs.que.push(function () {
        pbjs.addAdUnits([toAdUnit(unit)]);
        pbjs.requestBids({
          adUnitCodes: [unit.id],
          bidsBackHandler: function () {
            pbjs.setTargetingForGPTAsync([unit.id]);
            googletag.pubads().refresh([slot]);
          },
        });
      });
    });
  }

  function initialSlots() {
    return CONFIG.slots.filter(function (slot) { return !slot.anchor && !slot.lazy; });
  }

  // PAGE-2291: story cards swap the article in place. Set the stack up for
  // the new article the way a page load does.
  function onRoute(event) {
    track('route', event.detail);
    configurePrebid();
    defineSlots();
    runAuction(initialSlots(), 'route');
  }

  function start() {
    var anchors = CONFIG.slots.filter(function (slot) { return slot.anchor; });
    var lazy = CONFIG.slots.filter(function (slot) { return slot.lazy; });
    var initial = initialSlots();

    defineSlots();
    anchors.forEach(startAnchor);
    watchLazySlots(lazy);
    watchEngagementRefresh();
    startRecirculation();
    win.addEventListener('meridian:route', onRoute);

    if (CONFIG.scrollGatedAuction && isMobileViewport()) {
      win.addEventListener(
        'scroll',
        function () { runAuction(initial, 'initial:scroll'); },
        { passive: true, once: true }
      );
      return;
    }
    runAuction(initial, 'initial');
  }

  ensureLibrary(CONFIG.gptUrl);
  installAmazonStub();
  configurePrebid();
  if (doc.readyState === 'loading') {
    doc.addEventListener('DOMContentLoaded', start);
  } else {
    start();
  }
})(window, document);
