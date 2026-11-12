// What a scripted visitor needs to know about the page: how far it is
// scrolled, what there is to look at, and where its links are. Evaluated in
// the scanner's isolated world, so the page can neither see nor replace it.
//
// Facts only. Which link is followed, and whether any is, is decided in Rust
// (src/capture/visitor/links.rs). Only the top document is looked at: what is
// inside an iframe is never a candidate.
//
// One function expression, called with `{ slotIds, links }`: the ids of the ad
// containers the tracker reported, and whether links are wanted at all.
(function (request) {
  'use strict';

  var MAX_ANCHORS = 1500;
  var MAX_LINKS = 120;
  var MAX_TARGETS = 40;
  var MAX_TEXT = 80;
  var MIN_LINK_PX = 8;
  // An anchor this much covered by an ad frame is taken for part of the ad.
  var AD_OVERLAP = 0.3;
  var AD_SELECTOR =
    '[id^="google_ads_iframe"], [id^="div-gpt-ad"], [data-google-query-id], ins.adsbygoogle, ' +
    '[data-ad-slot], [data-ad-unit], [id^="taboola-"], [class*="trc_"], .OUTBRAIN, [data-widget-id][data-ob-template]';
  // Whole words of an id or class name, after splitting on "-", "_" and
  // spaces: "ad-slot" and "native_ad" name an ad, "header" and "shadow" do not.
  var AD_WORDS = ['ad', 'ads', 'advert', 'advertisement', 'adslot', 'sponsored', 'promoted'];
  // Attributes that make a click do more than open the link's address.
  var ACTS_ON_CLICK =
    '[onclick], [data-method], [data-turbo-method], [data-remote], [data-confirm], [data-turbo-confirm], ' +
    '[hx-get], [hx-post], [hx-put], [hx-patch], [hx-delete], [ping]';
  var REGIONS = [
    ['footer', 'footer, [role="contentinfo"]'],
    ['nav', 'nav, header, [role="navigation"], [role="banner"]'],
    ['aside', 'aside, [role="complementary"]'],
    ['content', 'main, article, [role="main"]'],
  ];

  var doc = document;
  var viewport = { width: window.innerWidth, height: window.innerHeight };
  var slotIds = {};
  (request.slotIds || []).forEach(function (id) {
    slotIds[id] = true;
  });

  function rectOf(element) {
    var box = element.getBoundingClientRect();
    return { x: box.left, y: box.top, width: box.width, height: box.height };
  }

  function namesAnAd(element) {
    var names = (element.id + ' ' + (typeof element.className === 'string' ? element.className : '')).toLowerCase();
    return names.split(/[-_\s]+/).some(function (word) {
      return AD_WORDS.indexOf(word) >= 0;
    });
  }

  // The page's own frame (html, body) is left out: a class such as
  // "ad-supported" on it says nothing about one link.
  function isInsideAd(element) {
    for (var node = element; node && node !== doc.body && node !== doc.documentElement; node = node.parentElement) {
      if (slotIds[node.id] || namesAnAd(node) || node.matches(AD_SELECTOR)) return true;
    }
    return false;
  }

  var adFrames = Array.prototype.slice
    .call(doc.querySelectorAll('iframe'))
    .concat(
      Object.keys(slotIds)
        .map(function (id) {
          return doc.getElementById(id);
        })
        .filter(Boolean)
    )
    .map(rectOf)
    .filter(function (box) {
      return box.width > 0 && box.height > 0;
    });

  function overlapsAdFrame(box) {
    var area = box.width * box.height;
    return adFrames.some(function (frame) {
      var width = Math.min(box.x + box.width, frame.x + frame.width) - Math.max(box.x, frame.x);
      var height = Math.min(box.y + box.height, frame.y + frame.height) - Math.max(box.y, frame.y);
      return width > 0 && height > 0 && width * height >= AD_OVERLAP * area;
    });
  }

  function isVisible(element, box) {
    if (box.width < MIN_LINK_PX || box.height < MIN_LINK_PX) return false;
    if (typeof element.checkVisibility === 'function' && !element.checkVisibility({ checkVisibilityCSS: true })) {
      return false;
    }
    var style = getComputedStyle(element);
    return Number(style.opacity) > 0.1 && style.pointerEvents !== 'none';
  }

  function regionOf(element) {
    for (var node = element.parentElement; node; node = node.parentElement) {
      for (var i = 0; i < REGIONS.length; i++) {
        if (node.matches(REGIONS[i][1])) return REGIONS[i][0];
      }
    }
    return 'other';
  }

  function textOf(anchor) {
    var image = anchor.querySelector('img[alt]');
    // `innerText` keeps the breaks between a kicker and a headline that are
    // blocks of their own; `textContent` would run them together.
    var text = anchor.innerText || anchor.textContent || anchor.getAttribute('aria-label') || (image && image.alt) || '';
    return text.replace(/\s+/g, ' ').trim().slice(0, MAX_TEXT);
  }

  function labelOf(anchor) {
    var label = (anchor.getAttribute('aria-label') || '') + ' ' + (anchor.getAttribute('title') || '');
    return label.replace(/\s+/g, ' ').trim().slice(0, MAX_TEXT);
  }

  function distanceFromView(box) {
    var middle = box.y + box.height / 2;
    return middle < 0 ? -middle : Math.max(0, middle - viewport.height);
  }

  function links() {
    var anchors = doc.querySelectorAll('a[href]');
    var found = [];
    for (var i = 0; i < anchors.length && i < MAX_ANCHORS; i++) {
      var anchor = anchors[i];
      var box = rectOf(anchor);
      found.push({
        href: anchor.href,
        text: textOf(anchor),
        label: labelOf(anchor),
        rel: anchor.rel || '',
        target: anchor.target || '',
        download: anchor.hasAttribute('download'),
        role: anchor.getAttribute('role') || '',
        rect: box,
        visible: isVisible(anchor, box),
        region: regionOf(anchor),
        inAd: isInsideAd(anchor) || !!anchor.querySelector('iframe') || overlapsAdFrame(box),
        inForm: !!anchor.closest('form'),
        actsOnClick: anchor.matches(ACTS_ON_CLICK),
        hasImage: !!anchor.querySelector('img, picture, svg, video'),
      });
    }
    // The links a reader would come across first: those in view, then the
    // nearest. Equally near ones stay in document order.
    return found
      .map(function (link, index) {
        return { link: link, index: index, distance: distanceFromView(link.rect) };
      })
      .sort(function (a, b) {
        return a.distance - b.distance || a.index - b.index;
      })
      .slice(0, MAX_LINKS)
      .map(function (entry) {
        return entry.link;
      });
  }

  // Things a reader looks at that are fully in view and not part of an ad.
  function readingTargets() {
    var kinds = { P: 'paragraph', H1: 'heading', H2: 'heading', H3: 'heading', IMG: 'image', FIGURE: 'image' };
    var elements = doc.querySelectorAll('p, h1, h2, h3, img, figure');
    var targets = [];
    for (var i = 0; i < elements.length && targets.length < MAX_TARGETS; i++) {
      var box = rectOf(elements[i]);
      var inView = box.y >= 0 && box.x >= 0 && box.y + box.height <= viewport.height && box.x + box.width <= viewport.width;
      if (!inView || box.width < 40 || box.height < 12) continue;
      if (isInsideAd(elements[i]) || overlapsAdFrame(box)) continue;
      targets.push({ kind: kinds[elements[i].tagName], rect: box });
    }
    return targets;
  }

  var page = doc.scrollingElement || doc.documentElement;
  return {
    viewport: viewport,
    scrollTop: window.scrollY,
    pageHeight: page ? page.scrollHeight : viewport.height,
    targets: readingTargets(),
    links: request.links ? links() : [],
  };
})
