// What a click at a point of the viewport would land on: the link there, or
// nothing when the point is covered by something else (an overlay, a sticky
// bar, an iframe) or by no link at all. Evaluated in the scanner's isolated
// world right before the visitor clicks, because pages move.
//
// One function expression, called with the point.
(function (point) {
  'use strict';
  var element = document.elementFromPoint(point.x, point.y);
  var anchor = element && typeof element.closest === 'function' ? element.closest('a[href]') : null;
  return anchor ? { href: anchor.href } : null;
})
