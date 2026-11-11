/* Amazon TAM (apstag) stand-in for the offline demo site.
 * Takes over the queueing stub installed by the page and answers fetchBids
 * with simulated bids after a fixed delay.
 */
(function (root) {
  'use strict';

  var apstag = root.apstag;
  if (!apstag || !Array.isArray(apstag._Q)) return;

  var RESPONSE_MS = 190;
  var lastBids = [];

  function fetchBids(config, callback) {
    var slots = (config && config.slots) || [];
    setTimeout(function () {
      lastBids = slots.map(function (slot, index) {
        var size = (slot.sizes && slot.sizes[0]) || [300, 250];
        return {
          slotID: slot.slotID,
          amzniid: 'Jx' + (index + 1) + 'k7dQ0a9',
          amznbid: 'v' + (index + 3) + 'q1s',
          amznp: '1',
          size: size[0] + 'x' + size[1],
        };
      });
      if (typeof callback === 'function') callback(lastBids);
    }, RESPONSE_MS);
  }

  function setDisplayBids() {
    if (!root.googletag || !root.googletag.pubads) return;
    root.googletag.pubads().getSlots().forEach(function (slot) {
      var bid = lastBids.filter(function (candidate) {
        return candidate.slotID === slot.getSlotElementId();
      })[0];
      if (!bid) return;
      slot.setTargeting('amzniid', bid.amzniid);
      slot.setTargeting('amznbid', bid.amznbid);
    });
  }

  var queued = apstag._Q;
  apstag.init = function () {};
  apstag.fetchBids = fetchBids;
  apstag.setDisplayBids = setDisplayBids;
  apstag.targetingKeys = function () {
    return ['amzniid', 'amznbid'];
  };
  apstag._Q = [];
  queued.forEach(function (call) {
    if (call[0] === 'f') fetchBids.apply(null, call[1]);
  });
})(window);
