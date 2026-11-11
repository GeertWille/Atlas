/* Prebid.js stand-in for the offline demo site.
 *
 * Reproduces the public API and event payload shapes of Prebid.js closely enough
 * for wrappers and monitoring tools to behave as they do against the real
 * library. Bidders are simulated in the page: each has a latency and CPM range,
 * and every outcome is derived from a seeded hash, so runs are repeatable.
 */
(function (root) {
  'use strict';

  var GLOBAL_NAME = 'pbjs';
  root._pbjsGlobals = root._pbjsGlobals || [];
  if (root._pbjsGlobals.indexOf(GLOBAL_NAME) < 0) root._pbjsGlobals.push(GLOBAL_NAME);
  var pbjs = (root[GLOBAL_NAME] = root[GLOBAL_NAME] || {});
  if (pbjs.libLoaded) return;

  pbjs.que = pbjs.que || [];
  pbjs.cmd = pbjs.cmd || [];
  pbjs.version = 'v10.14.0';
  pbjs.libLoaded = true;
  pbjs.installedModules = [
    'appnexusBidAdapter',
    'rubiconBidAdapter',
    'ixBidAdapter',
    'pubmaticBidAdapter',
    'criteoBidAdapter',
    'consentManagementTcf',
    'currency',
    'gptPreAuction',
    'priceFloors',
    'userId',
  ];
  pbjs.adUnits = pbjs.adUnits || [];
  pbjs.bidderSettings = pbjs.bidderSettings || {};

  // ------------------------------------------------------ simulated bidders

  // latency in ms, cpm in USD, bidRate = share of requests answered with a bid.
  var BIDDERS = {
    appnexus: { latency: [70, 150], cpm: [0.35, 1.9], bidRate: 0.95 },
    rubicon: { latency: [105, 195], cpm: [0.45, 2.4], bidRate: 0.9 },
    ix: { latency: [130, 215], cpm: [0.3, 1.6], bidRate: 0.45 },
    pubmatic: { latency: [330, 620], cpm: [1.8, 4.6], bidRate: 1 },
    criteo: { latency: [1500, 2100], cpm: [0.6, 2.2], bidRate: 1 },
  };
  var UNKNOWN_BIDDER = { latency: [100, 300], cpm: [0.1, 1], bidRate: 0.5 };

  function hash(text) {
    var h = 2166136261;
    for (var i = 0; i < text.length; i++) {
      h ^= text.charCodeAt(i);
      h = Math.imul(h, 16777619);
    }
    return h >>> 0;
  }

  // mulberry32
  function generator(seed) {
    var state = seed >>> 0;
    return function () {
      state = (state + 0x6d2b79f5) >>> 0;
      var t = state;
      t = Math.imul(t ^ (t >>> 15), t | 1);
      t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
      return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
    };
  }

  function between(range, draw) {
    return range[0] + (range[1] - range[0]) * draw;
  }

  var idCounter = 0;
  function uniqueId(prefix) {
    idCounter += 1;
    var random = generator(hash(prefix + ':' + idCounter));
    var hex = '';
    for (var i = 0; i < 4; i++) hex += ('0000000' + Math.floor(random() * 4294967296).toString(16)).slice(-8);
    return hex.slice(0, 8) + '-' + hex.slice(8, 12) + '-4' + hex.slice(13, 16) + '-a' + hex.slice(17, 20) + '-' + hex.slice(20, 32);
  }

  function shortId(prefix) {
    idCounter += 1;
    return hash(prefix + ':' + idCounter).toString(16) + idCounter.toString(16);
  }

  // --------------------------------------------------------------- logging

  var LOG_STYLE = 'display: inline-block; color: #fff; background: #3b88c3; padding: 1px 4px; border-radius: 3px;';

  function decorate(args, label) {
    return ['%cPrebid', LOG_STYLE, label].concat(Array.prototype.slice.call(args));
  }

  function logMessage() {
    if (config.debug) console.log.apply(console, decorate(arguments, 'MESSAGE:'));
  }

  function logInfo() {
    if (config.debug) console.info.apply(console, decorate(arguments, 'INFO:'));
  }

  function logWarn() {
    if (config.debug) console.warn.apply(console, decorate(arguments, 'WARNING:'));
    emit('auctionDebug', { type: 'WARNING', arguments: arguments });
  }

  function logError() {
    if (config.debug) console.error.apply(console, decorate(arguments, 'ERROR:'));
    emit('auctionDebug', { type: 'ERROR', arguments: arguments });
  }

  // ---------------------------------------------------------------- events

  var handlers = {};
  var eventsFired = [];

  function emit(eventType, args) {
    eventsFired.push({ eventType: eventType, args: args, id: args && args.adUnitCode, elapsedTime: performance.now() });
    (handlers[eventType] || []).slice().forEach(function (entry) {
      if (entry.id && (!args || entry.id !== args.adUnitCode)) return;
      try {
        entry.handler(args);
      } catch (error) {
        if (eventType !== 'auctionDebug') logError('Error executing handler:', 'events.js', error, eventType);
      }
    });
  }

  pbjs.onEvent = function (eventType, handler, id) {
    if (typeof handler !== 'function') {
      logError('The event handler provided is not a function and was not set on event "' + eventType + '".');
      return;
    }
    (handlers[eventType] = handlers[eventType] || []).push({ handler: handler, id: id });
  };

  pbjs.offEvent = function (eventType, handler, id) {
    handlers[eventType] = (handlers[eventType] || []).filter(function (entry) {
      return entry.handler !== handler || entry.id !== id;
    });
  };

  pbjs.getEvents = function () {
    return eventsFired.slice();
  };

  // ---------------------------------------------------------------- config

  var config = {
    debug: false,
    bidderTimeout: 3000,
    timeoutBuffer: 400,
    enableSendAllBids: true,
    useBidCache: false,
    deviceAccess: true,
    bidderSequence: 'random',
    maxRequestsPerOrigin: 4,
    priceGranularity: 'medium',
    mediaTypePriceGranularity: {},
    userSync: {
      syncEnabled: true,
      filterSettings: { image: { bidders: '*', filter: 'include' } },
      syncsPerBidder: 5,
      syncDelay: 3000,
      auctionDelay: 500,
    },
  };
  var configListeners = [];

  pbjs.setConfig = function (options) {
    if (!options || typeof options !== 'object') {
      logError('setConfig options must be an object');
      return;
    }
    Object.keys(options).forEach(function (key) {
      var value = options[key];
      var isMergeable = value && typeof value === 'object' && !Array.isArray(value) && config[key] && typeof config[key] === 'object';
      config[key] = isMergeable ? Object.assign({}, config[key], value) : value;
    });
    configListeners.forEach(function (listener) {
      if (!listener.topic) listener.callback(options);
      else if (listener.topic in options) {
        var update = {};
        update[listener.topic] = options[listener.topic];
        listener.callback(update);
      }
    });
  };

  pbjs.getConfig = function (topic, callback) {
    if (typeof topic === 'function') {
      configListeners.push({ topic: null, callback: topic });
      return;
    }
    if (typeof callback === 'function') {
      configListeners.push({ topic: topic, callback: callback });
      return;
    }
    return typeof topic === 'string' ? config[topic] : Object.assign({}, config);
  };

  pbjs.readConfig = pbjs.getConfig;

  // -------------------------------------------------------------- ad units

  pbjs.addAdUnits = function (adUnits) {
    (Array.isArray(adUnits) ? adUnits : [adUnits]).forEach(function (unit) {
      pbjs.adUnits.push(unit);
    });
  };

  pbjs.removeAdUnit = function (code) {
    var codes = code === undefined ? null : [].concat(code);
    pbjs.adUnits = pbjs.adUnits.filter(function (unit) {
      return codes && codes.indexOf(unit.code) < 0;
    });
  };

  function bannerSizes(unit) {
    var sizes = (unit.mediaTypes && unit.mediaTypes.banner && unit.mediaTypes.banner.sizes) || unit.sizes || [];
    return sizes.length === 2 && typeof sizes[0] === 'number' ? [sizes] : sizes;
  }

  // -------------------------------------------------------------- auctions

  var auctions = [];
  var bidsById = {};
  var requestCounts = {};

  function priceBucket(cpm, increment, cap) {
    return (Math.floor(Math.min(cpm, cap) / increment) * increment).toFixed(2);
  }

  function auctionSnapshot(auction) {
    return {
      auctionId: auction.auctionId,
      timestamp: auction.timestamp,
      auctionEnd: auction.auctionEnd,
      auctionStatus: auction.auctionStatus,
      adUnits: auction.adUnits,
      adUnitCodes: auction.adUnitCodes,
      labels: undefined,
      bidderRequests: auction.bidderRequests,
      noBids: auction.noBids.slice(),
      bidsReceived: auction.bidsReceived.slice(),
      bidsRejected: [],
      winningBids: auction.winningBids.slice(),
      timeout: auction.timeout,
    };
  }

  function buildBidderRequests(auction) {
    var byBidder = {};
    auction.adUnits.forEach(function (unit) {
      (unit.bids || []).forEach(function (bid) {
        var request = (byBidder[bid.bidder] = byBidder[bid.bidder] || {
          bidderCode: bid.bidder,
          auctionId: auction.auctionId,
          bidderRequestId: shortId('bidderRequest'),
          bids: [],
          auctionStart: auction.timestamp,
          timeout: auction.timeout,
          refererInfo: { page: root.location.href, domain: root.location.hostname, reachedTop: true },
        });
        request.bids.push({
          bidder: bid.bidder,
          params: bid.params || {},
          mediaTypes: unit.mediaTypes || { banner: { sizes: bannerSizes(unit) } },
          adUnitCode: unit.code,
          transactionId: uniqueId('transaction'),
          adUnitId: uniqueId('adUnit'),
          sizes: bannerSizes(unit),
          bidId: shortId('bid'),
          bidderRequestId: request.bidderRequestId,
          auctionId: auction.auctionId,
          src: 'client',
          bidRequestsCount: 1,
        });
      });
    });
    return Object.keys(byBidder).map(function (bidder) {
      return byBidder[bidder];
    });
  }

  // What one simulated bidder answers for one bid request. Seeded by bidder, ad
  // unit and how often that pair was requested, so it does not depend on timing.
  function simulate(bidRequest) {
    var pair = bidRequest.bidder + '|' + bidRequest.adUnitCode;
    requestCounts[pair] = (requestCounts[pair] || 0) + 1;
    var random = generator(hash(pair + '|' + requestCounts[pair]));
    var profile = BIDDERS[bidRequest.bidder] || UNKNOWN_BIDDER;
    var latencyDraw = random();
    var bids = random() < profile.bidRate;
    var cpm = Math.round(between(profile.cpm, random()) * 100) / 100;
    var size = bidRequest.sizes[Math.floor(random() * bidRequest.sizes.length)] || [300, 250];
    return { latencyDraw: latencyDraw, bids: bids, cpm: cpm, size: size };
  }

  function creativeMarkup(bid) {
    return (
      '<!doctype html><html><head><style>html,body{margin:0;height:100%}' +
      '.ad{box-sizing:border-box;height:100%;display:flex;flex-direction:column;align-items:center;justify-content:center;' +
      'font:600 14px/1.3 system-ui,sans-serif;color:#fff;background:linear-gradient(135deg,#0f766e,#1d4ed8);text-align:center}' +
      '.ad small{font-weight:400;opacity:.85;margin-top:4px}</style></head><body>' +
      '<div class="ad"><span>Sponsored · ' + bid.bidderCode + '</span>' +
      '<small>' + bid.size + ' · $' + bid.cpm.toFixed(2) + ' CPM</small></div></body></html>'
    );
  }

  function buildBid(auction, bidRequest, outcome, requestStartedAt) {
    var respondedAt = Date.now();
    var adId = shortId('ad');
    var size = outcome.size[0] + 'x' + outcome.size[1];
    var bid = {
      bidderCode: bidRequest.bidder,
      bidder: bidRequest.bidder,
      width: outcome.size[0],
      height: outcome.size[1],
      statusMessage: 'Bid available',
      adId: adId,
      requestId: bidRequest.bidId,
      transactionId: bidRequest.transactionId,
      adUnitId: bidRequest.adUnitId,
      auctionId: auction.auctionId,
      mediaType: 'banner',
      source: 'client',
      cpm: outcome.cpm,
      originalCpm: outcome.cpm,
      currency: 'USD',
      originalCurrency: 'USD',
      netRevenue: true,
      ttl: 300,
      creativeId: bidRequest.bidder + '-cr-' + hash(bidRequest.adUnitCode + size) % 100000,
      meta: { advertiserDomains: ['example-advertiser.test'] },
      responseTimestamp: respondedAt,
      requestTimestamp: requestStartedAt,
      timeToRespond: respondedAt - requestStartedAt,
      pbLg: priceBucket(outcome.cpm, 0.5, 5),
      pbMg: priceBucket(outcome.cpm, 0.1, 20),
      pbHg: priceBucket(outcome.cpm, 0.01, 20),
      pbAg: priceBucket(outcome.cpm, 0.05, 20),
      size: size,
      adUnitCode: bidRequest.adUnitCode,
      status: 'targetingSet',
    };
    bid.ad = creativeMarkup(bid);
    bid.adserverTargeting = {
      hb_bidder: bid.bidderCode,
      hb_adid: adId,
      hb_pb: bid.pbMg,
      hb_size: size,
      hb_format: 'banner',
      hb_source: 'client',
    };
    return bid;
  }

  function callBidder(auction, bidderRequest) {
    var startedAt = Date.now();
    bidderRequest.start = startedAt;
    auction.bidderRequests.push(bidderRequest);
    logMessage('CALLING BIDDER', bidderRequest);
    emit('bidRequested', bidderRequest);
    var outcomes = bidderRequest.bids.map(simulate);
    var profile = BIDDERS[bidderRequest.bidderCode] || UNKNOWN_BIDDER;
    // One HTTP round trip per bidder: every bid of the request arrives together.
    var latency = Math.round(between(profile.latency, outcomes[0].latencyDraw));
    setTimeout(function () {
      bidderRequest.done = true;
      if (auction.auctionStatus === 'completed') {
        logWarn(
          'Late response from ' + bidderRequest.bidderCode + ' ignored: it arrived ' + (Date.now() - startedAt) +
            'ms after the request, auction ' + auction.auctionId + ' timed out at ' + auction.timeout + 'ms'
        );
        return;
      }
      bidderRequest.bids.forEach(function (bidRequest, index) {
        var outcome = outcomes[index];
        if (!outcome.bids) {
          auction.noBids.push(bidRequest);
          emit('noBid', bidRequest);
          return;
        }
        var bid = buildBid(auction, bidRequest, outcome, startedAt);
        auction.bidsReceived.push(bid);
        bidsById[bid.adId] = bid;
        emit('bidResponse', bid);
      });
      var everyBidderDone = auction.bidderRequests.every(function (request) {
        return request.done;
      });
      if (everyBidderDone) completeAuction(auction, false);
    }, latency);
  }

  function completeAuction(auction, timedOut) {
    if (auction.auctionStatus === 'completed') return;
    clearTimeout(auction.timer);
    if (timedOut) {
      logMessage('Auction ' + auction.auctionId + ' timedOut');
      var timedOutBids = [];
      auction.bidderRequests.forEach(function (request) {
        if (request.done) return;
        request.bids.forEach(function (bid) {
          timedOutBids.push(Object.assign({}, bid, { timeout: auction.timeout }));
        });
      });
      if (timedOutBids.length) emit('bidTimeout', timedOutBids);
    }
    auction.auctionStatus = 'completed';
    auction.auctionEnd = Date.now();
    logInfo('Bids Received for Auction with id: ' + auction.auctionId, auction.bidsReceived);
    emit('auctionEnd', auctionSnapshot(auction));
    try {
      if (auction.callback) auction.callback(bidsByAdUnit(auction), timedOut, auction.auctionId);
    } catch (error) {
      logError('Error executing bidsBackHandler', null, error);
    }
    auction.resolve({ bids: bidsByAdUnit(auction), timedOut: timedOut, auctionId: auction.auctionId });
  }

  function bidsByAdUnit(auction) {
    var grouped = {};
    auction.bidsReceived.forEach(function (bid) {
      (grouped[bid.adUnitCode] = grouped[bid.adUnitCode] || { bids: [] }).bids.push(bid);
    });
    return grouped;
  }

  pbjs.requestBids = function (options) {
    var request = options || {};
    logInfo('Invoking pbjs.requestBids', request);
    var codes = request.adUnitCodes ? [].concat(request.adUnitCodes) : null;
    var adUnits = (request.adUnits || pbjs.adUnits).filter(function (unit) {
      return !codes || codes.indexOf(unit.code) >= 0;
    });
    var auction = {
      auctionId: uniqueId('auction'),
      timestamp: Date.now(),
      auctionStatus: 'inProgress',
      adUnits: adUnits,
      adUnitCodes: adUnits.map(function (unit) {
        return unit.code;
      }),
      bidderRequests: [],
      noBids: [],
      bidsReceived: [],
      winningBids: [],
      timeout: request.timeout || config.bidderTimeout,
      callback: request.bidsBackHandler,
    };
    var finished = new Promise(function (resolve) {
      auction.resolve = resolve;
    });
    if (!adUnits.length) {
      logMessage('No adUnits configured. No bids requested.');
      auction.auctionStatus = 'completed';
      if (auction.callback) auction.callback({}, false, auction.auctionId);
      auction.resolve({ bids: {}, timedOut: false, auctionId: auction.auctionId });
      return finished;
    }
    auctions.push(auction);
    var bidderRequests = buildBidderRequests(auction);
    logInfo('Auction ' + auction.auctionId + ' started: ' + auction.adUnitCodes.length + ' ad units, timeout ' + auction.timeout + 'ms');
    emit('auctionInit', Object.assign(auctionSnapshot(auction), { bidderRequests: bidderRequests }));
    auction.timer = setTimeout(function () {
      completeAuction(auction, true);
    }, auction.timeout);
    bidderRequests.forEach(function (bidderRequest) {
      callBidder(auction, bidderRequest);
    });
    return finished;
  };

  // ------------------------------------------------------------- targeting

  function unusedBidsFor(code) {
    var bids = [];
    auctions.forEach(function (auction) {
      auction.bidsReceived.forEach(function (bid) {
        if (bid.adUnitCode === code && bid.status !== 'rendered') bids.push(bid);
      });
    });
    return bids;
  }

  pbjs.getBidResponsesForAdUnitCode = function (code) {
    return { bids: unusedBidsFor(code) };
  };

  pbjs.getBidResponses = function () {
    var responses = {};
    pbjs.adUnits.forEach(function (unit) {
      var bids = unusedBidsFor(unit.code);
      if (bids.length) responses[unit.code] = { bids: bids };
    });
    return responses;
  };

  pbjs.getHighestCpmBids = function (code) {
    var codes = code ? [].concat(code) : pbjs.adUnits.map(function (unit) {
      return unit.code;
    });
    return codes
      .map(function (adUnitCode) {
        return unusedBidsFor(adUnitCode).reduce(function (best, bid) {
          return !best || bid.cpm > best.cpm ? bid : best;
        }, null);
      })
      .filter(Boolean);
  };

  pbjs.getAdserverTargetingForAdUnitCode = function (code) {
    var winner = pbjs.getHighestCpmBids(code)[0];
    return winner ? Object.assign({}, winner.adserverTargeting) : {};
  };

  pbjs.getAdserverTargeting = function (code) {
    var targeting = {};
    pbjs.getHighestCpmBids(code).forEach(function (bid) {
      targeting[bid.adUnitCode] = Object.assign({}, bid.adserverTargeting);
    });
    return targeting;
  };

  pbjs.setTargetingForGPTAsync = function (code) {
    if (!root.googletag || !root.googletag.pubads || !root.googletag.pubads().getSlots) {
      logError('window.googletag is not defined on the page');
      return;
    }
    var targeting = pbjs.getAdserverTargeting(code);
    var codes = code ? [].concat(code) : null;
    root.googletag.pubads().getSlots().forEach(function (slot) {
      var matches = [slot.getSlotElementId(), slot.getAdUnitPath()];
      var unitCode = matches.filter(function (candidate) {
        return codes ? codes.indexOf(candidate) >= 0 : pbjs.adUnits.some(function (unit) {
          return unit.code === candidate;
        });
      })[0];
      if (!unitCode) return;
      slot.getTargetingKeys().forEach(function (key) {
        if (key.indexOf('hb_') === 0) slot.clearTargeting(key);
      });
      var values = targeting[unitCode];
      if (!values) return;
      Object.keys(values).forEach(function (key) {
        slot.setTargeting(key, values[key]);
      });
    });
    Object.keys(targeting).forEach(function (unitCode) {
      var winner = pbjs.getHighestCpmBids(unitCode)[0];
      var auction = winner && auctions.filter(function (candidate) {
        return candidate.auctionId === winner.auctionId;
      })[0];
      if (auction && auction.winningBids.indexOf(winner) < 0) auction.winningBids.push(winner);
    });
    emit('setTargeting', targeting);
  };

  // ------------------------------------------------------------- rendering

  pbjs.renderAd = function (doc, adId) {
    logInfo('Calling renderAd with adId :' + adId);
    var bid = bidsById[adId];
    if (!doc || !bid) {
      var message = 'Error rendering ad (id: ' + adId + '): Cannot find ad \'' + adId + '\'';
      logError(message);
      emit('adRenderFailed', { reason: 'cannotFindAd', message: message, adId: adId });
      return;
    }
    bid.status = 'rendered';
    emit('bidWon', bid);
    try {
      doc.open();
      doc.write(bid.ad);
      doc.close();
      emit('adRenderSucceeded', { doc: doc, bid: bid, adId: adId });
    } catch (error) {
      emit('adRenderFailed', { reason: 'exception', message: error.message, bid: bid, adId: adId });
    }
  };

  // ---------------------------------------------------------- command queue

  function run(command) {
    if (typeof command !== 'function') {
      logError('Commands written into ' + GLOBAL_NAME + '.cmd.push must be wrapped in a function');
      return;
    }
    try {
      command.call();
    } catch (error) {
      logError('Error processing command :', error.message, error.stack);
    }
  }

  pbjs.processQueue = function () {
    [pbjs.que, pbjs.cmd].forEach(function (queue) {
      var pending = queue.slice();
      queue.push = run;
      pending.forEach(function (command) {
        if (command && command.called === undefined) {
          run(command);
          command.called = true;
        }
      });
    });
  };

  pbjs.processQueue();
})(window);
