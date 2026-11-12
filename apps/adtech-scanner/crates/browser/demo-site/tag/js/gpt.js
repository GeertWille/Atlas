/* Google Publisher Tag stand-in for the offline demo site.
 *
 * Reproduces the parts of the googletag API that header-bidding wrappers use,
 * and the slot event sequence with realistic timing. The "ad server" has demand
 * only for header-bidding line items: a slot whose request carries Prebid
 * targeting (hb_adid) is filled with that bid's creative, any other request
 * comes back empty.
 */
(function (root) {
  'use strict';

  var googletag = (root.googletag = root.googletag || {});
  if (googletag.apiReady) {
    console.warn('[GPT] gpt.js was loaded more than once on this page; the second copy was ignored.');
    return;
  }
  var queued = Array.isArray(googletag.cmd) ? googletag.cmd : [];

  var SERVICE_NAME = 'publisher_ads';
  var VIEWABLE_AFTER_MS = 1000;
  var slots = [];
  var listeners = {};
  var pageTargeting = {};
  var initialLoadDisabled = false;
  var servicesEnabled = false;
  var collapseEmpty = false;
  var requestCount = 0;

  function hash(text) {
    var h = 2166136261;
    for (var i = 0; i < text.length; i++) {
      h ^= text.charCodeAt(i);
      h = Math.imul(h, 16777619);
    }
    return h >>> 0;
  }

  function fire(type, detail) {
    (listeners[type] || []).slice().forEach(function (listener) {
      try {
        listener(detail);
      } catch (error) {
        console.error('[GPT] Exception in ' + type + ' listener', error);
      }
    });
  }

  function Size(width, height) {
    this.width = width;
    this.height = height;
  }
  Size.prototype.getWidth = function () {
    return this.width;
  };
  Size.prototype.getHeight = function () {
    return this.height;
  };

  function toSizes(input) {
    if (!Array.isArray(input)) return [];
    if (input.length === 2 && typeof input[0] === 'number') return [new Size(input[0], input[1])];
    return input
      .filter(function (size) {
        return Array.isArray(size) && size.length === 2;
      })
      .map(function (size) {
        return new Size(size[0], size[1]);
      });
  }

  function targetingApi(owner, store) {
    owner.setTargeting = function (key, value) {
      store[key] = [].concat(value).map(String);
      return owner;
    };
    owner.getTargeting = function (key) {
      return (store[key] || []).slice();
    };
    owner.getTargetingKeys = function () {
      return Object.keys(store);
    };
    owner.clearTargeting = function (key) {
      if (key === undefined) {
        Object.keys(store).forEach(function (existing) {
          delete store[existing];
        });
      } else {
        delete store[key];
      }
      return owner;
    };
  }

  function Slot(adUnitPath, sizes, elementId) {
    this.adUnitPath = adUnitPath;
    this.sizes = toSizes(sizes);
    this.elementId = elementId;
    this.targeting = {};
    this.responseInformation = null;
    this.renderCount = 0;
    this.viewability = null;
    targetingApi(this, this.targeting);
  }
  Slot.prototype.getSlotElementId = function () {
    return this.elementId;
  };
  Slot.prototype.getAdUnitPath = function () {
    return this.adUnitPath;
  };
  Slot.prototype.getSizes = function () {
    return this.sizes.slice();
  };
  Slot.prototype.getTargetingMap = function () {
    return JSON.parse(JSON.stringify(this.targeting));
  };
  Slot.prototype.getConfig = function (key) {
    return key === 'targeting' ? { targeting: this.getTargetingMap() } : {};
  };
  Slot.prototype.getResponseInformation = function () {
    return this.responseInformation;
  };
  Slot.prototype.addService = function () {
    if (slots.indexOf(this) < 0) slots.push(this);
    return this;
  };
  Slot.prototype.defineSizeMapping = function () {
    return this;
  };
  Slot.prototype.setCollapseEmptyDiv = function () {
    return this;
  };

  var definedIds = {};
  var generatedIds = 0;

  googletag.defineSlot = function (adUnitPath, sizes, elementId) {
    var id = elementId || 'gpt_unit_' + adUnitPath + '_' + generatedIds++;
    if (definedIds[id]) {
      console.error(
        '[GPT] Error in googletag.defineSlot: Cannot create slot ' + adUnitPath + '. Div element "' + id +
          '" is already associated with another slot.'
      );
      return null;
    }
    definedIds[id] = true;
    return new Slot(adUnitPath, sizes, id);
  };

  googletag.defineOutOfPageSlot = function (adUnitPath, elementId) {
    return googletag.defineSlot(adUnitPath, [], elementId);
  };

  googletag.destroySlots = function (toDestroy) {
    var doomed = toDestroy || slots.slice();
    doomed.forEach(function (slot) {
      delete definedIds[slot.elementId];
      if (slot.viewability) slot.viewability.disconnect();
    });
    slots = slots.filter(function (slot) {
      return doomed.indexOf(slot) < 0;
    });
    return true;
  };

  // ----------------------------------------------------------- ad requests

  function watchViewability(slot, container) {
    if (slot.viewability) slot.viewability.disconnect();
    if (typeof IntersectionObserver !== 'function') return;
    var timer = null;
    slot.viewability = new IntersectionObserver(
      function (entries) {
        var inView = entries[entries.length - 1].intersectionRatio >= 0.5;
        if (!inView) {
          clearTimeout(timer);
          timer = null;
        } else if (!timer) {
          timer = setTimeout(function () {
            slot.viewability.disconnect();
            fire('impressionViewable', { slot: slot, serviceName: SERVICE_NAME });
          }, VIEWABLE_AFTER_MS);
        }
      },
      { threshold: [0, 0.5, 1] }
    );
    slot.viewability.observe(container);
  }

  function renderResponse(slot, headerBid) {
    var container = document.getElementById(slot.elementId);
    fire('slotResponseReceived', { slot: slot, serviceName: SERVICE_NAME });
    if (!container) return;
    container.textContent = '';
    if (!headerBid.adId) {
      slot.responseInformation = null;
      container.setAttribute('data-google-query-id', 'unfilled');
      if (collapseEmpty) container.style.display = 'none';
      fire('slotRenderEnded', {
        slot: slot,
        serviceName: SERVICE_NAME,
        isEmpty: true,
        size: null,
        advertiserId: null,
        campaignId: null,
        creativeId: null,
        lineItemId: null,
        sourceAgnosticCreativeId: null,
        sourceAgnosticLineItemId: null,
        isBackfill: false,
        slotContentChanged: true,
      });
      return;
    }
    var dimensions = (headerBid.size || '').split('x').map(Number);
    var width = dimensions[0] || slot.sizes[0].width;
    var height = dimensions[1] || slot.sizes[0].height;
    var lineItemId = 6120045000 + Math.round(parseFloat(headerBid.priceBucket || '0') * 100);
    var creativeId = 138400000000 + (hash(slot.adUnitPath) % 1000000);
    slot.responseInformation = {
      advertiserId: 4728193,
      campaignId: 2914400817,
      creativeId: creativeId,
      lineItemId: lineItemId,
      sourceAgnosticCreativeId: creativeId,
      sourceAgnosticLineItemId: lineItemId,
      isBackfill: false,
    };
    slot.renderCount += 1;
    var frame = document.createElement('iframe');
    frame.id = 'google_ads_iframe_' + slot.adUnitPath + '_' + (slot.renderCount - 1);
    frame.title = '3rd party ad content';
    frame.width = width;
    frame.height = height;
    frame.scrolling = 'no';
    frame.style.cssText = 'border:0;vertical-align:bottom;max-width:100%';
    container.setAttribute('data-google-query-id', 'CL' + hash(slot.elementId + requestCount).toString(36));
    container.appendChild(frame);
    // Registered after insertion: the empty frame's own load event is not the creative's.
    frame.addEventListener('load', function onCreativeLoad() {
      frame.removeEventListener('load', onCreativeLoad);
      fire('slotOnload', { slot: slot, serviceName: SERVICE_NAME });
    });
    fire(
      'slotRenderEnded',
      Object.assign({ slot: slot, serviceName: SERVICE_NAME, isEmpty: false, size: [width, height], slotContentChanged: true }, slot.responseInformation)
    );
    // The header-bidding line item's creative asks Prebid to draw the winning ad.
    setTimeout(function () {
      if (root.pbjs && typeof root.pbjs.renderAd === 'function') {
        root.pbjs.renderAd(frame.contentWindow.document, headerBid.adId);
      }
    }, 0);
    watchViewability(slot, container);
  }

  function request(slot) {
    requestCount += 1;
    // Targeting is read when the request leaves; later changes do not reach it.
    var headerBid = {
      adId: slot.getTargeting('hb_adid')[0],
      size: slot.getTargeting('hb_size')[0],
      priceBucket: slot.getTargeting('hb_pb')[0],
    };
    fire('slotRequested', { slot: slot, serviceName: SERVICE_NAME });
    var latency = 85 + (hash(slot.elementId + ':' + requestCount) % 70);
    setTimeout(function () {
      renderResponse(slot, headerBid);
    }, latency);
  }

  var pubads = {
    addEventListener: function (type, listener) {
      (listeners[type] = listeners[type] || []).push(listener);
      return pubads;
    },
    removeEventListener: function (type, listener) {
      listeners[type] = (listeners[type] || []).filter(function (existing) {
        return existing !== listener;
      });
    },
    getSlots: function () {
      return slots.slice();
    },
    refresh: function (toRefresh) {
      if (!servicesEnabled) return;
      (toRefresh || slots).forEach(request);
    },
    disableInitialLoad: function () {
      initialLoadDisabled = true;
    },
    isInitialLoadDisabled: function () {
      return initialLoadDisabled;
    },
    enableSingleRequest: function () {
      return true;
    },
    collapseEmptyDivs: function () {
      collapseEmpty = true;
      return true;
    },
    setCentering: function () {},
    enableLazyLoad: function () {},
    setPrivacySettings: function () {
      return pubads;
    },
    updateCorrelator: function () {
      return pubads;
    },
    getName: function () {
      return SERVICE_NAME;
    },
  };
  targetingApi(pubads, pageTargeting);

  googletag.pubads = function () {
    return pubads;
  };

  googletag.enableServices = function () {
    servicesEnabled = true;
    googletag.pubadsReady = true;
  };

  googletag.display = function (slotOrElementId) {
    var slot = slots.filter(function (candidate) {
      return candidate === slotOrElementId || candidate.elementId === slotOrElementId;
    })[0];
    if (slot && servicesEnabled && !initialLoadDisabled) request(slot);
  };

  googletag.getVersion = function () {
    return '202611050101';
  };

  googletag.apiReady = true;

  function run(command) {
    try {
      command.call(root);
    } catch (error) {
      console.error('[GPT] Exception in queued GPT command', error);
    }
    return 1;
  }
  googletag.cmd = { push: run };
  queued.forEach(run);
})(window);
