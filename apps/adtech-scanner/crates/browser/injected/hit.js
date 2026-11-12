// What a click at a point of the viewport would land on: the link there, or
// nothing when the point is covered by something else (an overlay, a sticky
// bar, an iframe) or by no link at all. Evaluated in the scanner's isolated
// world right before the visitor clicks, because pages move.
//
// Nothing as well when the point is on something inside the link that takes
// the click for itself: a frame (an ad, a player), a form control, or a
// custom element, whose shadow tree cannot be looked into from here.
//
// One function expression, called with the point.
(function (point) {
  'use strict';
  var OWN_CLICK_TARGETS = 'iframe, frame, object, embed, button, input, select, textarea, label, summary, video, audio';
  var element = document.elementFromPoint(point.x, point.y);
  if (!element || typeof element.closest !== 'function') return null;
  var anchor = element.closest('a[href]');
  if (!anchor) return null;
  for (var node = element; node && node !== anchor; node = node.parentElement) {
    if (node.matches(OWN_CLICK_TARGETS) || node.shadowRoot || node.localName.indexOf('-') >= 0) return null;
  }
  return { href: anchor.href };
})
