// Ad-stack tracker. Evaluated in the page's main world before any page script.
//
// Rules of the house: plain ES2017, no dependencies, never throw into the page,
// never change what the page's own calls return, and leave no globals behind.
// Emits `{ t, type, ...fields }` objects matching `TrackerEventKind` in
// crates/model/src/capture.rs, batched through a CDP binding.
//
// This file is one function expression. The scanner calls it with the site
// being scanned, `{ domain, port }`, or with `null` while the visit's first
// page is still to be reached. On any other site it does nothing at all.
(function (site) {
  'use strict';

  var win = window;
  // The binding exists in every frame of the tab; none of them may keep it.
  var send = win['__adscan_emit'];
  try {
    delete win['__adscan_emit'];
  } catch (ignored) {}
  if (typeof send !== 'function') return;
  try {
    if (win.top !== win) return;
  } catch (crossOriginParent) {
    return;
  }
  if (site && !isOnSite(win.location)) return;

  // `port` is set when the site is one exact origin (an IP address,
  // localhost); otherwise every host under `domain` belongs to it.
  function isOnSite(location) {
    var host = String(location.hostname).toLowerCase();
    if (site.port === null) {
      return host === site.domain || host.slice(-site.domain.length - 1) === '.' + site.domain;
    }
    var port = Number(location.port) || (location.protocol === 'https:' ? 443 : 80);
    return host === site.domain && port === site.port;
  }

  // Pages replace these; keep the originals.
  var doc = win.document;
  var perf = win.performance;
  var setTimer = win.setTimeout.bind(win);
  var stringify = JSON.stringify;
  var isArray = Array.isArray;
  var nativeToString = Function.prototype.toString;

  var FLUSH_DELAY_MS = 100;
  var MAX_BATCH = 200;
  var MAX_EVENTS = 40000;
  var MAX_TEXT = 1000;
  var MAX_LIST = 200;
  var MAX_OBJECT_KEYS = 60;
  var FAST_POLL_MS = 50;
  var SLOW_POLL_MS = 1000;
  var FAST_POLL_WINDOW_MS = 30000;

  function now() {
    return perf.now();
  }

  function guard(fn) {
    return function () {
      try {
        return fn.apply(this, arguments);
      } catch (ignored) {}
    };
  }

  // ---------------------------------------------------------------- transport

  var queue = [];
  var emitted = 0;
  var flushScheduled = false;

  function flush() {
    flushScheduled = false;
    if (!queue.length) return;
    var batch = queue;
    queue = [];
    try {
      send(stringify({ o: perf.timeOrigin, e: batch }));
    } catch (ignored) {}
  }

  function emit(type, fields, at) {
    if (emitted >= MAX_EVENTS) return;
    emitted += 1;
    var event = { t: Math.round((num(at) === null ? now() : at) * 10) / 10, type: type };
    for (var key in fields) event[key] = fields[key];
    queue.push(event);
    if (queue.length >= MAX_BATCH) {
      flush();
    } else if (!flushScheduled) {
      flushScheduled = true;
      setTimer(flush, FLUSH_DELAY_MS);
    }
  }

  win.addEventListener('__adscan_flush', flush);
  win.addEventListener('pagehide', flush);

  // ------------------------------------------------------------------ values

  function str(value) {
    if (typeof value === 'string') return value ? clip(value) : null;
    if (typeof value === 'number' && isFinite(value)) return String(value);
    return null;
  }

  function num(value) {
    return typeof value === 'number' && isFinite(value) ? value : null;
  }

  function clip(text) {
    return text.length > MAX_TEXT ? text.slice(0, MAX_TEXT) + '…' : text;
  }

  function pushUnique(list, value) {
    if (value !== null && list.length < MAX_LIST && list.indexOf(value) < 0) list.push(value);
  }

  function sizeText(width, height) {
    var w = Number(width);
    var h = Number(height);
    return isFinite(w) && isFinite(h) && w > 0 && h > 0 ? Math.round(w) + 'x' + Math.round(h) : null;
  }

  // One size in any of the shapes Prebid and GPT use: [w, h], "WxH", "fluid",
  // { width, height }, { w, h } or a GPT Size object.
  function sizeOf(value) {
    if (typeof value === 'string') return value || null;
    if (isArray(value)) return value.length >= 2 ? sizeText(value[0], value[1]) : null;
    if (!value || typeof value !== 'object') return null;
    if (typeof value.getWidth === 'function') return sizeText(value.getWidth(), value.getHeight());
    return sizeText(value.width, value.height) || sizeText(value.w, value.h);
  }

  function sizesOf(value) {
    var sizes = [];
    if (!isArray(value)) {
      pushUnique(sizes, sizeOf(value));
    } else if (value.length === 2 && typeof value[0] === 'number' && typeof value[1] === 'number') {
      pushUnique(sizes, sizeOf(value));
    } else {
      for (var i = 0; i < value.length; i++) pushUnique(sizes, sizeOf(value[i]));
    }
    return sizes;
  }

  function describe(value) {
    if (typeof value === 'string') return value;
    if (value === null || value === undefined) return String(value);
    if (typeof value === 'number' || typeof value === 'boolean') return String(value);
    if (value instanceof Error) return value.message || String(value);
    if (typeof value === 'function') return '[function]';
    try {
      var json = stringify(plain(value, 2));
      return json.length > 300 ? json.slice(0, 300) + '…' : json;
    } catch (unserializable) {
      return '[object]';
    }
  }

  function joinArgs(args, from) {
    var parts = [];
    for (var i = from; i < args.length && parts.length < 8; i++) parts.push(describe(args[i]));
    return clip(parts.join(' '));
  }

  // A JSON-safe copy: functions and DOM nodes dropped, depth and width bounded.
  function plain(value, depth) {
    if (value === null || typeof value === 'string' || typeof value === 'boolean') return value;
    if (typeof value === 'number') return isFinite(value) ? value : null;
    if (typeof value !== 'object') return undefined;
    if (depth <= 0 || (typeof Node === 'function' && value instanceof Node) || value === win) return '[…]';
    var copy;
    if (isArray(value)) {
      copy = [];
      for (var i = 0; i < value.length && i < 40; i++) {
        var item = plain(value[i], depth - 1);
        copy.push(item === undefined ? null : item);
      }
      return copy;
    }
    copy = {};
    var keys = Object.keys(value);
    for (var k = 0; k < keys.length && k < MAX_OBJECT_KEYS; k++) {
      var entry = plain(value[keys[k]], depth - 1);
      if (entry !== undefined) copy[keys[k]] = entry;
    }
    return copy;
  }

  // A wrapper that looks like the function it replaces to anything inspecting it.
  function disguise(wrapper, original) {
    try {
      Object.defineProperty(wrapper, 'name', { value: original.name, configurable: true });
      Object.defineProperty(wrapper, 'length', { value: original.length, configurable: true });
      Object.defineProperty(wrapper, 'toString', {
        value: function () {
          return nativeToString.call(original);
        },
        configurable: true,
        writable: true,
      });
    } catch (ignored) {}
    return wrapper;
  }

  // Calls `onValue` the moment the page first assigns `window[name]`, without
  // the global existing before that. Libraries that keep no event history (GPT,
  // apstag) must be hooked before the assigning script continues.
  function watchGlobal(name, onValue) {
    try {
      var existing = Object.getOwnPropertyDescriptor(win, name);
      if (existing) {
        if (existing.value) onValue(existing.value);
        return;
      }
      Object.defineProperty(win, name, {
        configurable: true,
        enumerable: true,
        get: function () {
          return undefined;
        },
        set: function (value) {
          try {
            Object.defineProperty(win, name, {
              configurable: true,
              enumerable: true,
              writable: true,
              value: value,
            });
            onValue(value);
          } catch (ignored) {}
        },
      });
    } catch (ignored) {}
  }

  // ------------------------------------------------------------------ Prebid

  var PREBID_EVENTS = [
    'requestBids',
    'addAdUnits',
    'auctionInit',
    'bidRequested',
    'bidResponse',
    'noBid',
    'bidTimeout',
    'bidderError',
    'auctionEnd',
    'setTargeting',
    'bidWon',
    'adRenderFailed',
    'adRenderSucceeded',
    'auctionDebug',
  ];
  var CONFIG_KEYS = [
    'bidderTimeout',
    'timeoutBuffer',
    'priceGranularity',
    'customPriceBucket',
    'mediaTypePriceGranularity',
    'auctionOptions',
    'currency',
    'floors',
    'userSync',
    'consentManagement',
    'enableSendAllBids',
    'bidderSequence',
    'useBidCache',
    'targetingControls',
    'sendBidsControl',
    's2sConfig',
    'realTimeData',
    'deviceAccess',
    'maxRequestsPerOrigin',
    'enableTIDs',
    'exchangeId',
  ];
  var MAX_CONFIG_CHARS = 20000;
  var MAX_PREBID_LOGS = 4000;
  var MAX_PARAMS_CHARS = 2000;
  var MAX_SCHAIN_NODES = 10;

  // Instances whose `requestBids` calls are reported with their arguments.
  var watchedRequests = new WeakSet();
  var prebidInstances = new WeakSet();
  var configReported = new WeakSet();
  var hookedQueues = new WeakSet();
  var prebidLogs = 0;

  function prebidGlobalNames() {
    var names = ['pbjs'];
    var registered = win._pbjsGlobals;
    if (isArray(registered)) {
      for (var i = 0; i < registered.length; i++) {
        if (typeof registered[i] === 'string') pushUnique(names, registered[i]);
      }
    }
    return names;
  }

  function inspectPrebid() {
    var names = prebidGlobalNames();
    for (var i = 0; i < names.length; i++) {
      var instance = win[names[i]];
      if (!instance || typeof instance !== 'object') continue;
      if (typeof instance.onEvent === 'function') {
        attachPrebid(names[i], instance);
      } else {
        hookCommandQueue(instance.que, names[i]);
        hookCommandQueue(instance.cmd, names[i]);
      }
    }
  }

  // Getting to the front of the command queue means subscribing before the
  // publisher's first command runs.
  function hookCommandQueue(commands, name) {
    if (!isArray(commands) || hookedQueues.has(commands)) return;
    hookedQueues.add(commands);
    commands.unshift(
      guard(function () {
        var instance = win[name];
        if (instance && typeof instance.onEvent === 'function') attachPrebid(name, instance);
      })
    );
  }

  function attachPrebid(name, instance) {
    if (prebidInstances.has(instance)) return;
    prebidInstances.add(instance);
    emit('prebidDetected', {
      globalName: name,
      version: str(instance.version),
      modules: moduleNames(instance.installedModules),
    });
    // Debug mode only adds log lines; auctions run the same.
    guard(function () {
      if (typeof instance.setConfig === 'function') instance.setConfig({ debug: true });
    })();
    // Whatever fired before this point is replayed once from Prebid's own
    // history; subscribing afterwards cannot count anything twice.
    guard(function () {
      var history = typeof instance.getEvents === 'function' ? instance.getEvents() : [];
      for (var i = 0; isArray(history) && i < history.length; i++) {
        var past = history[i] || {};
        if (PREBID_EVENTS.indexOf(past.eventType) >= 0) {
          guard(onPrebidEvent)(instance, past.eventType, past.args, num(past.elapsedTime));
        }
      }
    })();
    PREBID_EVENTS.forEach(function (eventType) {
      guard(function () {
        instance.onEvent(eventType, function (payload) {
          guard(onPrebidEvent)(instance, eventType, payload, null);
        });
      })();
    });
    guard(watchBidRequests)(instance);
  }

  // What the page asks Prebid for is only in the arguments of `requestBids`;
  // the event Prebid emits for the call carries none. Prebid's own hook on
  // the function (the one its modules use) shows them. The call goes on
  // exactly as made, whatever happens here.
  function watchBidRequests(instance) {
    var request = instance.requestBids;
    if (typeof request !== 'function' || typeof request.before !== 'function') return;
    request.before(function (next, options) {
      guard(reportBidsRequested)(instance, options);
      return next.apply(this, Array.prototype.slice.call(arguments, 1));
    });
    watchedRequests.add(instance);
  }

  function registeredAdUnits(instance) {
    return isArray(instance.adUnits) ? instance.adUnits.length : null;
  }

  function reportBidsRequested(instance, options) {
    var asked = options || {};
    var codes = [];
    for (var i = 0; isArray(asked.adUnitCodes) && i < asked.adUnitCodes.length && i < MAX_LIST; i++) {
      pushUnique(codes, str(asked.adUnitCodes[i]));
    }
    emit('bidsRequested', {
      adUnitCodes: codes,
      // Ad units passed with the call replace the registered ones.
      adUnitCount: isArray(asked.adUnits) ? asked.adUnits.length : registeredAdUnits(instance),
      timeout: num(asked.timeout),
    });
  }

  function moduleNames(modules) {
    var names = [];
    for (var i = 0; isArray(modules) && i < modules.length && names.length < 300; i++) {
      if (typeof modules[i] === 'string') names.push(modules[i]);
    }
    return names;
  }

  function bidderOf(bid) {
    return str(bid && (bid.bidderCode || bid.bidder)) || 'unknown';
  }

  function onPrebidEvent(instance, eventType, payload, at) {
    var data = payload || {};
    // `at` is set for an event replayed from Prebid's history: what Prebid
    // holds now says nothing about then.
    var live = num(at) === null;
    switch (eventType) {
      case 'requestBids':
        if (live && watchedRequests.has(instance)) break;
        emit(
          'bidsRequested',
          { adUnitCodes: [], adUnitCount: live ? registeredAdUnits(instance) : null, timeout: null },
          at
        );
        break;
      case 'addAdUnits':
        emit('adUnitsAdded', { registered: live ? registeredAdUnits(instance) : null }, at);
        break;
      case 'auctionInit':
        guard(reportPrebidConfig)(instance, at);
        emit(
          'auctionInit',
          { auctionId: str(data.auctionId) || 'unknown', timeout: num(data.timeout), adUnits: adUnitsOf(data.adUnits) },
          at
        );
        break;
      case 'bidRequested':
        emit(
          'bidRequested',
          {
            auctionId: str(data.auctionId) || 'unknown',
            bidder: bidderOf(data),
            adUnitCodes: adUnitCodesOf(data.bids),
            gdprApplies: data.gdprConsent ? flag(data.gdprConsent.gdprApplies) : null,
            hasConsentString: data.gdprConsent ? !!str(data.gdprConsent.consentString) : null,
            schain: schainOf(data),
          },
          at
        );
        break;
      case 'bidResponse':
        emit(
          'bidResponse',
          {
            auctionId: str(data.auctionId) || 'unknown',
            bidder: bidderOf(data),
            adUnitCode: str(data.adUnitCode) || 'unknown',
            cpm: num(Number(data.cpm)) || 0,
            currency: str(data.currency),
            size: sizeOf(data.size) || sizeText(data.width, data.height),
            timeToRespond: num(data.timeToRespond),
            dealId: str(data.dealId),
          },
          at
        );
        break;
      case 'noBid':
        emit('noBid', bidKey(data), at);
        break;
      case 'bidTimeout':
        (isArray(payload) ? payload : [payload]).forEach(function (bid) {
          emit('bidTimeout', bidKey(bid || {}), at);
        });
        break;
      case 'bidderError':
        var request = data.bidderRequest || data;
        emit(
          'bidderError',
          { auctionId: str(request.auctionId), bidder: bidderOf(request), message: bidderErrorMessage(data.error) },
          at
        );
        break;
      case 'auctionEnd':
        emit('auctionEnd', { auctionId: str(data.auctionId) || 'unknown' }, at);
        break;
      case 'setTargeting':
        // Prebid lists every ad unit it looked at, with `{}` for those that
        // got no keys.
        var targeted = [];
        Object.keys(data).forEach(function (code) {
          if (hasKeys(data[code])) pushUnique(targeted, str(code));
        });
        emit('setTargeting', { adUnitCodes: targeted }, at);
        break;
      case 'bidWon':
        emit(
          'bidWon',
          {
            auctionId: str(data.auctionId),
            bidder: bidderOf(data),
            adUnitCode: str(data.adUnitCode) || 'unknown',
            cpm: num(Number(data.cpm)) || 0,
          },
          at
        );
        break;
      case 'adRenderFailed':
        emit(
          'adRenderFailed',
          {
            adUnitCode: str(data.bid && data.bid.adUnitCode),
            bidder: data.bid ? bidderOf(data.bid) : null,
            reason: str(data.reason) || str(data.message),
          },
          at
        );
        break;
      case 'adRenderSucceeded':
        emit(
          'adRenderSucceeded',
          { adUnitCode: str(data.bid && data.bid.adUnitCode), bidder: data.bid ? bidderOf(data.bid) : null },
          at
        );
        break;
      case 'auctionDebug':
        var kind = String(data.type || '').toUpperCase();
        var args = data.arguments && typeof data.arguments.length === 'number' ? data.arguments : [];
        emitPrebidLog(kind === 'ERROR' ? 'error' : kind === 'WARNING' ? 'warning' : 'info', joinArgs(args, 0), at);
        break;
    }
  }

  // `error` is the failed request as Prebid's ajax layer reports it (an XHR
  // or an XHR-shaped object), an Error, or a string.
  function bidderErrorMessage(error) {
    if (typeof error === 'string') return str(error);
    if (!error || typeof error !== 'object') return null;
    if (error.timedOut === true) return 'request timed out';
    return (
      str(error.message) ||
      str(error.statusText) ||
      (num(error.status) ? 'HTTP ' + error.status : null) ||
      str(error.reason) ||
      str(error.responseText)
    );
  }

  function bidKey(bid) {
    return {
      auctionId: str(bid.auctionId) || 'unknown',
      bidder: bidderOf(bid),
      adUnitCode: str(bid.adUnitCode) || 'unknown',
    };
  }

  function adUnitsOf(adUnits) {
    var units = [];
    for (var i = 0; isArray(adUnits) && i < adUnits.length && i < MAX_LIST; i++) {
      var unit = adUnits[i] || {};
      var mediaTypes = unit.mediaTypes || {};
      var sizes = sizesOf(mediaTypes.banner && mediaTypes.banner.sizes);
      sizesOf(mediaTypes.video && mediaTypes.video.playerSize).forEach(function (size) {
        pushUnique(sizes, size);
      });
      if (!sizes.length) sizes = sizesOf(unit.sizes);
      var bidders = [];
      var bids = [];
      for (var b = 0; isArray(unit.bids) && b < unit.bids.length; b++) {
        var bidder = str(unit.bids[b] && unit.bids[b].bidder);
        pushUnique(bidders, bidder);
        var params = bidder && bids.length < MAX_LIST ? paramsOf(unit.bids[b].params) : null;
        if (params) bids.push({ bidder: bidder, params: params });
      }
      units.push({ code: str(unit.code) || 'unknown', sizes: sizes, bidders: bidders, bids: bids });
    }
    return units;
  }

  // The adapter params as configured. Params too large to copy whole are
  // left out rather than cut: half an object would read as missing keys.
  function paramsOf(params) {
    if (params === undefined) return {};
    if (!params || typeof params !== 'object' || isArray(params)) return null;
    try {
      if (Object.keys(params).length > MAX_OBJECT_KEYS) return null;
      var copy = plain(params, 4);
      return stringify(copy).length <= MAX_PARAMS_CHARS ? copy : null;
    } catch (unreadable) {
      // A getter or proxy that throws costs this bid's params, not the event.
      return null;
    }
  }

  function flag(value) {
    return typeof value === 'boolean' ? value : null;
  }

  // Prebid 9 and earlier put the supply chain on each bid, later versions
  // in the first-party data of the request.
  function schainOf(bidderRequest) {
    var firstBid = (isArray(bidderRequest.bids) && bidderRequest.bids[0]) || {};
    var ortb2 = bidderRequest.ortb2 || firstBid.ortb2 || {};
    var schain = (ortb2.source && ortb2.source.ext && ortb2.source.ext.schain) || firstBid.schain || {};
    var nodes = [];
    for (var i = 0; isArray(schain.nodes) && i < schain.nodes.length && nodes.length < MAX_SCHAIN_NODES; i++) {
      var node = schain.nodes[i] || {};
      var asi = str(node.asi);
      var sid = str(node.sid);
      if (asi && sid) nodes.push({ asi: asi, sid: sid, hp: node.hp === 1 ? true : node.hp === 0 ? false : null });
    }
    return nodes;
  }

  function adUnitCodesOf(bids) {
    var codes = [];
    for (var i = 0; isArray(bids) && i < bids.length; i++) pushUnique(codes, str(bids[i] && bids[i].adUnitCode));
    return codes;
  }

  function reportPrebidConfig(instance, at) {
    if (configReported.has(instance) || typeof instance.getConfig !== 'function') return;
    configReported.add(instance);
    var config = instance.getConfig() || {};
    var granularity = config.priceGranularity;
    var userSync = config.userSync;
    emit(
      'prebidConfig',
      {
        bidderTimeout: num(config.bidderTimeout),
        priceGranularity: typeof granularity === 'string' ? granularity : granularity ? 'custom' : null,
        hasFloors: hasKeys(config.floors),
        hasUserSync: !!userSync && userSync.syncEnabled !== false,
        hasConsentManagement: hasKeys(config.consentManagement),
        currency: str(config.currency && config.currency.adServerCurrency),
        raw: boundedConfig(config),
      },
      at
    );
  }

  function hasKeys(value) {
    return !!value && typeof value === 'object' && Object.keys(value).length > 0;
  }

  function boundedConfig(config) {
    var selected = {};
    CONFIG_KEYS.forEach(function (key) {
      var value = plain(config[key], 6);
      if (value !== undefined) selected[key] = value;
    });
    var keys = Object.keys(selected);
    while (stringify(selected).length > MAX_CONFIG_CHARS && keys.length) {
      var largest = keys.reduce(function (a, b) {
        return stringify(selected[a]).length >= stringify(selected[b]).length ? a : b;
      });
      selected[largest] = '[omitted: too large]';
      keys.splice(keys.indexOf(largest), 1);
    }
    return selected;
  }

  function emitPrebidLog(level, message, at) {
    if (!message || prebidLogs >= MAX_PREBID_LOGS) return;
    prebidLogs += 1;
    emit('prebidLog', { level: level, message: message }, at);
  }

  // Prebid prints `%cPrebid <style> LABEL: ...`. Warnings and errors also come
  // through `auctionDebug` (history included), so only the rest is taken here.
  var CONSOLE_LABELS = { 'MESSAGE:': 'log', 'INFO:': 'info', 'DEBUG:': 'debug' };

  function onConsoleCall(args) {
    var first = args[0];
    if (typeof first !== 'string' || first.indexOf('%cPrebid') !== 0) return;
    var styleCount = first.split('%c').length - 1;
    var level = CONSOLE_LABELS[args[1 + styleCount]];
    if (level) emitPrebidLog(level, joinArgs(args, 2 + styleCount), null);
  }

  function wrapConsole() {
    var target = win.console;
    if (!target) return;
    ['log', 'info', 'debug', 'warn', 'error'].forEach(function (method) {
      var original = target[method];
      if (typeof original !== 'function') return;
      target[method] = disguise(function () {
        guard(onConsoleCall)(arguments);
        return original.apply(this, arguments);
      }, original);
    });
  }

  // --------------------------------------------------------------------- GPT

  var GPT_SLOT_EVENTS = ['slotRequested', 'slotResponseReceived', 'slotRenderEnded', 'slotOnload', 'impressionViewable'];

  var gptReported = false;
  var gptWrappers = new WeakSet();
  var listenedLibraries = new WeakSet();
  var knownSlots = new WeakSet();
  // Slots defined before their container exists, waiting to be measured.
  var unmeasuredSlots = [];

  function inspectGpt() {
    var googletag = win.googletag;
    if (!googletag || typeof googletag !== 'object') return;
    if (googletag.apiReady && typeof googletag.pubads === 'function') {
      attachGpt(googletag);
      return;
    }
    if (googletag.cmd === undefined) {
      googletag.cmd = [];
    }
    if (isArray(googletag.cmd) && !hookedQueues.has(googletag.cmd)) {
      hookedQueues.add(googletag.cmd);
      googletag.cmd.unshift(
        guard(function () {
          attachGpt(win.googletag);
        })
      );
    }
  }

  function attachGpt(googletag) {
    if (!gptReported) {
      gptReported = true;
      emit('gptDetected', {
        version: str(typeof googletag.getVersion === 'function' ? googletag.getVersion() : null),
      });
    }
    wrapSlotFactory(googletag, 'defineSlot', 2);
    wrapSlotFactory(googletag, 'defineOutOfPageSlot', 1);
    wrapDestroySlots(googletag);
    if (listenedLibraries.has(googletag)) return;
    var pubads = googletag.pubads();
    if (!pubads || typeof pubads.addEventListener !== 'function') return;
    listenedLibraries.add(googletag);
    var existing = typeof pubads.getSlots === 'function' ? pubads.getSlots() : [];
    for (var i = 0; isArray(existing) && i < existing.length; i++) noteSlot(existing[i]);
    GPT_SLOT_EVENTS.forEach(function (eventType) {
      pubads.addEventListener(
        eventType,
        guard(function (event) {
          onSlotEvent(eventType, event || {});
        })
      );
    });
  }

  // `elementIdAt` is the position of the container id among the arguments.
  function wrapSlotFactory(googletag, method, elementIdAt) {
    var original = googletag[method];
    if (typeof original !== 'function' || gptWrappers.has(original)) return;
    var wrapper = disguise(function () {
      var slot = original.apply(this, arguments);
      var elementId = arguments[elementIdAt];
      guard(function () {
        // GPT refuses a second slot for a container and returns null.
        if (slot === null && str(elementId)) emit('slotDefinitionRejected', { elementId: str(elementId) });
        else noteSlot(slot);
      })();
      return slot;
    }, original);
    gptWrappers.add(wrapper);
    googletag[method] = wrapper;
  }

  function wrapDestroySlots(googletag) {
    var original = googletag.destroySlots;
    if (typeof original !== 'function' || gptWrappers.has(original)) return;
    var wrapper = disguise(function (slots) {
      var doomed = [];
      guard(function () {
        var listed = isArray(slots) ? slots : googletag.pubads().getSlots();
        for (var i = 0; isArray(listed) && i < listed.length; i++) pushUnique(doomed, slotElementId(listed[i]));
      })();
      var destroyed = original.apply(this, arguments);
      if (destroyed && doomed.length) emit('slotsDestroyed', { elementIds: doomed });
      return destroyed;
    }, original);
    gptWrappers.add(wrapper);
    googletag.destroySlots = wrapper;
  }

  function slotElementId(slot) {
    return str(slot && typeof slot.getSlotElementId === 'function' ? slot.getSlotElementId() : null);
  }

  function noteSlot(slot) {
    if (!slot || typeof slot !== 'object' || knownSlots.has(slot)) return;
    var elementId = slotElementId(slot);
    if (!elementId) return;
    knownSlots.add(slot);
    var definition = {
      slot: slot,
      at: now(),
      fields: {
        elementId: elementId,
        adUnitPath: str(typeof slot.getAdUnitPath === 'function' ? slot.getAdUnitPath() : null),
        sizes: sizesOf(typeof slot.getSizes === 'function' ? slot.getSizes() : []),
        inViewport: null,
      },
    };
    unmeasuredSlots.push(definition);
    reportMeasurableSlots(null);
  }

  // `slotDefined` waits (keeping its original timestamp) until the container is
  // in the DOM and its position is known. `force` gives up waiting: for one
  // slot that is about to report other events, or for all (`true`).
  function reportMeasurableSlots(force) {
    if (!unmeasuredSlots.length) return;
    var waiting = [];
    unmeasuredSlots.forEach(function (definition) {
      var element = doc.getElementById(definition.fields.elementId);
      if (!element && force !== true && force !== definition.slot) {
        waiting.push(definition);
        return;
      }
      definition.fields.inViewport = element ? isInViewport(element) : null;
      emit('slotDefined', definition.fields, definition.at);
    });
    unmeasuredSlots = waiting;
  }

  function isInViewport(element) {
    if (!element.getClientRects().length) return false;
    var rect = element.getBoundingClientRect();
    return rect.bottom >= 0 && rect.right >= 0 && rect.top < win.innerHeight && rect.left < win.innerWidth;
  }

  // GPT is retiring the slot's targeting getters in favour of
  // `getConfig('targeting')`; the getters are used for as long as they exist.
  function slotTargeting(slot) {
    var targeting = {};
    if (typeof slot.getTargetingKeys === 'function') {
      var keys = slot.getTargetingKeys();
      for (var i = 0; isArray(keys) && i < keys.length; i++) {
        targeting[keys[i]] = typeof slot.getTargeting === 'function' ? slot.getTargeting(keys[i]) : [];
      }
      return targeting;
    }
    var config = typeof slot.getConfig === 'function' ? slot.getConfig('targeting') : null;
    var configured = config && config.targeting;
    if (!configured || typeof configured !== 'object') return targeting;
    Object.keys(configured).forEach(function (key) {
      var value = configured[key];
      if (isArray(value) ? value.length > 0 : value !== null && value !== undefined) targeting[key] = value;
    });
    return targeting;
  }

  function onSlotEvent(eventType, event) {
    var slot = event.slot;
    var elementId = slotElementId(slot);
    if (!elementId) return;
    noteSlot(slot);
    reportMeasurableSlots(slot);
    var fields = { elementId: elementId };
    if (eventType === 'slotRequested') {
      var targeting = slotTargeting(slot);
      fields.prebidTargetingKeys = [];
      Object.keys(targeting).forEach(function (key) {
        if (key.indexOf('hb_') === 0) pushUnique(fields.prebidTargetingKeys, str(key));
      });
      // Which bid the ad server is asked to serve: the same id on two
      // requests means a bid is being offered twice.
      var adId = targeting.hb_adid;
      fields.prebidAdId = str(isArray(adId) ? adId[0] : adId);
    } else if (eventType === 'slotRenderEnded') {
      fields.isEmpty = event.isEmpty !== false;
      fields.size = sizeOf(event.size);
      fields.advertiserId = str(event.advertiserId);
      fields.lineItemId = str(event.lineItemId) || str(event.sourceAgnosticLineItemId);
    }
    emit(eventType, fields, null);
  }

  // ------------------------------------------------------------- Amazon TAM

  var apsReported = false;
  var apsWrappers = new WeakSet();
  var apsCallbacks = new WeakSet();

  function inspectAps() {
    var apstag = win.apstag;
    if (!apstag || typeof apstag !== 'object') return;
    var original = apstag.fetchBids;
    if (typeof original !== 'function' || apsWrappers.has(original)) return;
    if (!apsReported) {
      apsReported = true;
      emit('amazonDetected', {});
    }
    var wrapper = disguise(function (config, callback) {
      var args = arguments;
      try {
        // The stub queues calls for the real library, which must not report
        // the same fetch a second time.
        if (typeof callback !== 'function') {
          observeApsFetch(config, callback);
        } else if (!apsCallbacks.has(callback)) {
          args = Array.prototype.slice.call(arguments);
          args[1] = observeApsFetch(config, callback);
        }
      } catch (ignored) {}
      return original.apply(this, args);
    }, original);
    apsWrappers.add(wrapper);
    apstag.fetchBids = wrapper;
  }

  function observeApsFetch(config, callback) {
    var startedAt = now();
    emit('apsFetchBids', { slotIds: apsSlotIds(config && config.slots), timeout: num(config && config.timeout) });
    if (typeof callback !== 'function') return callback;
    var observed = disguise(function (bids) {
      guard(function () {
        emit('apsBidsReturned', { slotIds: apsSlotIds(bids), duration: Math.round(now() - startedAt) });
      })();
      return callback.apply(this, arguments);
    }, callback);
    apsCallbacks.add(observed);
    return observed;
  }

  function apsSlotIds(slots) {
    var ids = [];
    for (var i = 0; isArray(slots) && i < slots.length; i++) {
      pushUnique(ids, str(slots[i] && (slots[i].slotID || slots[i].slotId)));
    }
    return ids;
  }

  // ----------------------------------------------------------------- consent

  var MAX_VENDOR_ID = 5000;
  var MAX_PURPOSE_ID = 24;

  var cmpReported = false;
  var tcfListened = new WeakSet();
  var lastTcfState = null;

  // Listens to the CMP like any vendor on the page does. The TC string itself
  // is never read into an event: only whether there is one, and what it allows.
  function inspectConsent() {
    var tcfapi = win.__tcfapi;
    if (typeof tcfapi !== 'function' || tcfListened.has(tcfapi)) return;
    tcfListened.add(tcfapi);
    if (!cmpReported) {
      cmpReported = true;
      emit('cmpDetected', { api: 'tcf' });
    }
    tcfapi('addEventListener', 2, guard(onTcData));
  }

  function onTcData(tcData, success) {
    if (!success || !tcData || typeof tcData !== 'object') return;
    var state = {
      eventStatus: str(tcData.eventStatus),
      gdprApplies: flag(tcData.gdprApplies),
      hasTcString: !!str(tcData.tcString),
      purposeConsents: grantedIds(tcData.purpose && tcData.purpose.consents, MAX_PURPOSE_ID),
      vendorConsents: grantedIds(tcData.vendor && tcData.vendor.consents, MAX_VENDOR_ID),
    };
    // The stub and the CMP that replaces it both answer the same listener.
    var fingerprint = stringify(state);
    if (fingerprint === lastTcfState) return;
    lastTcfState = fingerprint;
    emit('tcfState', state);
  }

  function grantedIds(consents, maxId) {
    var ids = [];
    if (!consents || typeof consents !== 'object') return ids;
    var keys = Object.keys(consents);
    for (var i = 0; i < keys.length && ids.length < maxId; i++) {
      var id = Number(keys[i]);
      if (consents[keys[i]] === true && id >= 1 && id <= maxId && Math.floor(id) === id) ids.push(id);
    }
    return ids;
  }

  // --------------------------------------------------------- page lifecycle

  var SCROLL_THROTTLE_MS = 200;
  var seenPhases = {};
  var scrollMeasureScheduled = false;
  var lastDepth = -1;

  function phase(name) {
    if (seenPhases[name]) return;
    seenPhases[name] = true;
    emit('pageLifecycle', { phase: name });
  }

  function measureScroll() {
    scrollMeasureScheduled = false;
    var page = doc.scrollingElement || doc.documentElement;
    if (!page) return;
    var scrollable = page.scrollHeight - win.innerHeight;
    var depth = scrollable > 0 ? Math.min(1, Math.max(0, win.scrollY / scrollable)) : 0;
    depth = Math.round(depth * 1000) / 1000;
    if (depth === lastDepth) return;
    lastDepth = depth;
    emit('scroll', { depth: depth });
  }

  function watchLifecycle() {
    var passive = { capture: true, passive: true };
    doc.addEventListener(
      'DOMContentLoaded',
      guard(function () {
        phase('domContentLoaded');
        inspectAll();
        reportMeasurableSlots(true);
      })
    );
    win.addEventListener(
      'load',
      guard(function () {
        phase('load');
        inspectAll();
      })
    );
    win.addEventListener(
      'scroll',
      guard(function () {
        phase('firstScroll');
        if (scrollMeasureScheduled) return;
        scrollMeasureScheduled = true;
        setTimer(guard(measureScroll), SCROLL_THROTTLE_MS);
      }),
      passive
    );
    ['pointerdown', 'touchstart', 'mousedown', 'keydown', 'wheel'].forEach(function (type) {
      win.addEventListener(
        type,
        guard(function () {
          phase('firstInteraction');
        }),
        passive
      );
    });
    // Moving the pointer is not an interaction, but ad stacks wait for it too.
    win.addEventListener(
      'pointermove',
      guard(function () {
        phase('firstPointerMove');
      }),
      { capture: true, passive: true, once: true }
    );
    // Script `load` events do not bubble but can be captured: a library that
    // just finished loading is hooked without waiting for the next poll.
    doc.addEventListener('load', inspectAll, true);
  }

  // ----------------------------------------------------------- route changes

  var ROUTE_POLL_MS = 100;

  // A single-page app changes the address without loading a document. The
  // Navigation API reports that as it happens; `history` is never patched.
  // Browsers without the API are asked for the address ten times a second.
  function watchRoutes() {
    var navigation = win.navigation;
    if (navigation && typeof navigation.addEventListener === 'function') {
      navigation.addEventListener(
        'currententrychange',
        guard(function (event) {
          reportRoute(str(event && event.navigationType) || 'push');
        })
      );
      return;
    }
    var lastSeen = win.location.href;
    (function pollRoute() {
      guard(function () {
        if (win.location.href === lastSeen) return;
        lastSeen = win.location.href;
        reportRoute('push');
      })();
      setTimer(pollRoute, ROUTE_POLL_MS);
    })();
  }

  function reportRoute(navigationType) {
    emit('routeChange', { url: clip(String(win.location.href)), navigationType: navigationType });
  }

  // ------------------------------------------------------------------ wiring

  var inspectAll = guard(function () {
    guard(inspectPrebid)();
    guard(inspectGpt)();
    guard(inspectAps)();
    guard(inspectConsent)();
    reportMeasurableSlots(null);
  });

  function poll() {
    inspectAll();
    setTimer(poll, now() < FAST_POLL_WINDOW_MS ? FAST_POLL_MS : SLOW_POLL_MS);
  }

  guard(wrapConsole)();
  guard(watchLifecycle)();
  guard(watchRoutes)();
  watchGlobal('googletag', inspectGpt);
  watchGlobal('apstag', inspectAps);
  poll();
})
