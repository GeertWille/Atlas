/*! tinykit-ui 2.3.0 | MIT | tabs, accordion and relative-time helpers */
(function (root) {
  'use strict';

  function each(selector, scope, fn) {
    Array.prototype.forEach.call((scope || document).querySelectorAll(selector), fn);
  }

  function tabs(container) {
    function select(name) {
      each('[role="tab"]', container, function (tab) {
        tab.setAttribute('aria-selected', String(tab.getAttribute('data-tab') === name));
      });
      each('[data-panel]', container, function (panel) {
        panel.hidden = panel.getAttribute('data-panel') !== name;
      });
    }
    each('[role="tab"]', container, function (tab) {
      tab.addEventListener('click', function () {
        select(tab.getAttribute('data-tab'));
      });
    });
  }

  function accordion(details) {
    details.addEventListener('toggle', function () {
      details.classList.toggle('is-open', details.open);
    });
  }

  var UNITS = [
    ['year', 31536000],
    ['month', 2592000],
    ['day', 86400],
    ['hour', 3600],
    ['minute', 60],
  ];

  function relativeTime(date, now) {
    var seconds = Math.round((date.getTime() - (now || Date.now())) / 1000);
    var formatter = typeof Intl !== 'undefined' && Intl.RelativeTimeFormat ? new Intl.RelativeTimeFormat('en', { numeric: 'auto' }) : null;
    for (var i = 0; i < UNITS.length; i++) {
      if (Math.abs(seconds) >= UNITS[i][1]) {
        var value = Math.round(seconds / UNITS[i][1]);
        return formatter ? formatter.format(value, UNITS[i][0]) : Math.abs(value) + ' ' + UNITS[i][0] + 's ago';
      }
    }
    return 'just now';
  }

  function throttle(fn, wait) {
    var last = 0;
    var timer = null;
    return function () {
      var remaining = wait - (Date.now() - last);
      var args = arguments;
      if (remaining <= 0) {
        last = Date.now();
        fn.apply(null, args);
      } else if (!timer) {
        timer = setTimeout(function () {
          timer = null;
          last = Date.now();
          fn.apply(null, args);
        }, remaining);
      }
    };
  }

  root.TinyKit = { each: each, tabs: tabs, accordion: accordion, relativeTime: relativeTime, throttle: throttle };
})(window);
