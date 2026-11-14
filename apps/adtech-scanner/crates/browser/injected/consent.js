// Finds a consent banner's "accept all" control in one frame. Evaluated in the
// scanner's isolated world of every frame of the tab, because consent
// platforms put their dialog in an iframe of another origin as often as not.
//
// One function expression, called with `{ click, watchMs }`. It evaluates to
//   { name, label, x, y }  the control was found: `label` is what it says,
//                   `x`/`y` are a point on it in this frame's viewport, or
//                   null when the point is covered;
//   { dialog }      a consent dialog is here but nothing in it plainly means
//                   "accept all";
//   null            nothing of the kind.
// With `click` set, the control is clicked from script as well. Without it,
// nothing on the page is touched: the scanner clicks the point itself. With
// `watchMs`, it evaluates instead to a promise of whether the control was
// clicked within that time, so the scanner knows its click arrived.
(function (request) {
  'use strict';

  // The consent platforms most publishers use: the accept-all control, and
  // the dialog it belongs to. One row per platform; add rows, not code.
  var PLATFORMS = [
    {
      name: 'Sourcepoint',
      accept:
        'button.sp_choice_type_11, button.sp_choice_type_ACCEPT_ALL, .message-component[title="Accept All"], ' +
        '.message-component.message-button[title="Alle akzeptieren"], .message-component.message-button[title="Zustimmen"]',
      dialog: '[id^="sp_message_container"], [id^="sp_message_iframe"], .message-container .message-component',
    },
    { name: 'OneTrust', accept: '#onetrust-accept-btn-handler', dialog: '#onetrust-banner-sdk' },
    { name: 'Didomi', accept: '#didomi-notice-agree-button', dialog: '#didomi-notice, #didomi-popup' },
    {
      name: 'Cookiebot',
      accept: '#CybotCookiebotDialogBodyLevelButtonLevelOptinAllowAll, #CybotCookiebotDialogBodyButtonAccept',
      dialog: '#CybotCookiebotDialog',
    },
    {
      name: 'Quantcast / InMobi Choice',
      accept: '.qc-cmp2-summary-buttons button[mode="primary"], .qc-cmp2-footer button[mode="primary"]',
      dialog: '.qc-cmp2-container',
    },
    {
      name: 'Usercentrics',
      accept: '[data-testid="uc-accept-all-button"], button[data-action-type="accept"]',
      // Inside the shadow tree of `#usercentrics-root` (v2) or
      // `#usercentrics-cmp-ui` (v3). Those hosts are on the page whether or
      // not the dialog shows, and hold the privacy button after a choice.
      dialog: '#uc-center-container, [data-testid="uc-default-wall"], #uc-main-dialog',
    },
    {
      name: 'TrustArc',
      accept: '#truste-consent-button, .trustarc-agree-btn',
      dialog: '#truste-consent-track, #trustarc-banner-overlay',
    },
    { name: 'Google Funding Choices', accept: '.fc-cta-consent', dialog: '.fc-consent-root' },
    {
      name: 'consentmanager.net',
      accept: '#cmpwelcomebtnyes a, #cmpbntyestxt, .cmpboxbtnyes',
      dialog: '#cmpbox, .cmpbox',
    },
    { name: 'Iubenda', accept: '.iubenda-cs-accept-btn', dialog: '#iubenda-cs-banner' },
    {
      name: 'Borlabs Cookie',
      accept: '#BorlabsCookieBox [data-cookie-accept-all], .brlbs-btn-accept-all, [data-borlabs-cookie-accept-all]',
      dialog: '#BorlabsCookieBox, .brlbs-cmpnt-dialog',
    },
    { name: 'Osano', accept: '.osano-cm-accept-all', dialog: '.osano-cm-dialog' },
    { name: 'Complianz', accept: '.cmplz-btn.cmplz-accept', dialog: '.cmplz-cookiebanner' },
    { name: 'CookieYes', accept: '.cky-btn-accept', dialog: '.cky-consent-container' },
  ];

  // Unambiguous "accept everything" labels: English, Dutch, German, French,
  // Spanish, Italian, Portuguese, Danish.
  var ACCEPT_ALL = [
    'accept all', 'accept all cookies', 'allow all', 'allow all cookies', 'agree to all', 'accept & continue',
    'accept and continue', 'accept all and continue', 'accept all & continue', 'agree and proceed',
    'agree & proceed', 'yes, i agree', 'i accept all',
    'alles accepteren', 'accepteer alles', 'alle cookies accepteren', 'alles toestaan', 'alle cookies toestaan',
    'ik ga akkoord', 'akkoord en doorgaan', 'accepteren en doorgaan',
    'alle akzeptieren', 'alles akzeptieren', 'alle cookies akzeptieren', 'alle zulassen', 'alles zulassen',
    'alle cookies zulassen', 'allen zustimmen', 'akzeptieren und weiter', 'zustimmen und weiter',
    'alle akzeptieren und weiter', 'akzeptieren und schließen', 'akzeptieren & schließen',
    'tout accepter', 'accepter tout', 'accepter et continuer', 'accepter et fermer', 'accepter & fermer',
    "j'accepte tout", 'tout autoriser',
    'aceptar todo', 'aceptar todas', 'aceptar todas las cookies', 'aceptar y continuar', 'permitir todas',
    'aceptar y cerrar',
    'accetta tutto', 'accetta tutti', 'accetta tutti i cookie', 'accetta e continua', 'accetta e chiudi',
    'aceitar tudo', 'aceitar todos', 'aceitar todos os cookies', 'aceitar e continuar', 'aceitar e fechar',
    'tillad alle', 'tillad alle cookies', 'accepter alle', 'accepter alle cookies', 'godkend alle',
    'accepter og luk', 'accepter og fortsæt',
  ];
  // Short labels that only mean consent when they sit inside a consent dialog.
  var ACCEPT_IN_CONTEXT = [
    'accept', 'agree', 'i agree', 'i accept', 'allow', 'got it', 'ok',
    'accepteren', 'akkoord', 'toestaan',
    'akzeptieren', 'zustimmen', 'einverstanden', 'ich stimme zu',
    'accepter', "j'accepte", "d'accord", 'autoriser',
    'aceptar', 'acepto', 'de acuerdo', 'permitir',
    'accetta', 'accetto', 'acconsento',
    'tillad', 'godkend', 'jeg accepterer',
  ];
  // A consent wall may offer a paid way out next to the consent option
  // ("read with ads" or "subscribe"). Whatever a control is otherwise, one
  // that names a purchase is never clicked.
  var PURCHASE =
    /(^|[^a-z])(pur|abo|abos|abonn\w*|subscri\w*|suscri\w*|pay|paid|bezahl\w*|zahlen|kaufen|kopen|buy|acheter|payer|comprar|premium|plus|price|preis|prijs|prix)([^a-z]|$)|[€$£]/i;
  var CONSENT_CONTEXT = /cookie|consent|privacy|gdpr|rgpd|datenschutz|privacidad|confidentialit|cmp/i;
  var CLICKABLE = 'button, [role="button"], a, input[type="button"], input[type="submit"]';

  // The document plus everything reachable from it: open shadow roots and
  // same-origin iframes. Frames of other origins are evaluated on their own.
  function searchRoots(root, roots) {
    roots.push(root);
    var elements = root.querySelectorAll('*');
    for (var i = 0; i < elements.length; i++) {
      var element = elements[i];
      if (element.shadowRoot) searchRoots(element.shadowRoot, roots);
    }
    return roots;
  }

  function isClickable(element) {
    if (element.disabled || !element.getClientRects().length) return false;
    var style = getComputedStyle(element);
    return style.visibility !== 'hidden' && style.pointerEvents !== 'none' && Number(style.opacity) > 0.05;
  }

  function normalized(text) {
    return String(text || '')
      .replace(/\s+/g, ' ')
      .replace(/[’`]/g, "'")
      .trim()
      .toLowerCase();
  }

  // What the control is called: its text, and where that says nothing (an
  // icon, a styled span), its accessible name or tooltip.
  function labelsOf(element) {
    var text = element.tagName === 'INPUT' ? element.value : element.innerText || element.textContent;
    return [text, element.getAttribute('aria-label'), element.getAttribute('title')].map(normalized).filter(Boolean);
  }

  function namesAPurchase(element) {
    return labelsOf(element).some(function (label) {
      return PURCHASE.test(label);
    });
  }

  // A frame that holds nothing but a consent message says so in its address
  // or title; every control in it is in consent context.
  var isConsentFrame =
    window !== window.top && CONSENT_CONTEXT.test(location.hostname + location.pathname + ' ' + document.title);

  // `<html>` and `<body>` are not a dialog: feature-detection classes such as
  // Modernizr's "cookies" would put every button of the page in consent context.
  function isInsideConsentDialog(element) {
    if (isConsentFrame) return true;
    for (var node = element, depth = 0; node && depth < 8; depth++) {
      if (node.nodeType === 1) {
        if (node.tagName === 'BODY' || node.tagName === 'HTML') return false;
        var marker = (node.id || '') + ' ' + (typeof node.className === 'string' ? node.className : '');
        var label = node.getAttribute('aria-label') || '';
        if (CONSENT_CONTEXT.test(marker) || CONSENT_CONTEXT.test(label)) return true;
        if (node.getAttribute('role') === 'dialog' && CONSENT_CONTEXT.test(node.textContent.slice(0, 2000))) return true;
      }
      node = node.parentNode || node.host;
    }
    return false;
  }

  function findKnownPlatform(roots) {
    for (var p = 0; p < PLATFORMS.length; p++) {
      for (var r = 0; r < roots.length; r++) {
        var matches = roots[r].querySelectorAll(PLATFORMS[p].accept);
        for (var m = 0; m < matches.length; m++) {
          if (isClickable(matches[m]) && !namesAPurchase(matches[m])) {
            return { name: PLATFORMS[p].name, element: matches[m] };
          }
        }
      }
    }
    return null;
  }

  function findByLabel(roots) {
    var inContext = null;
    for (var r = 0; r < roots.length; r++) {
      var candidates = roots[r].querySelectorAll(CLICKABLE);
      for (var c = 0; c < candidates.length; c++) {
        var element = candidates[c];
        if (!isClickable(element) || namesAPurchase(element)) continue;
        var labels = labelsOf(element);
        for (var l = 0; l < labels.length; l++) {
          var label = labels[l];
          if (label.length > 40) continue;
          if (ACCEPT_ALL.indexOf(label) >= 0) return { name: 'button "' + label + '"', element: element };
          if (!inContext && ACCEPT_IN_CONTEXT.indexOf(label) >= 0 && isInsideConsentDialog(element)) {
            inContext = { name: 'button "' + label + '"', element: element };
          }
        }
      }
    }
    return inContext;
  }

  // A container that is laid out but empty, or hidden, is no dialog to
  // report: a consent platform keeps its container on pages where it shows
  // nothing.
  function isShowing(element) {
    var box = element.getBoundingClientRect();
    return box.width > 0 && box.height > 0 && getComputedStyle(element).visibility !== 'hidden';
  }

  function findDialog(roots) {
    for (var p = 0; p < PLATFORMS.length; p++) {
      for (var r = 0; r < roots.length; r++) {
        var matches = roots[r].querySelectorAll(PLATFORMS[p].dialog);
        for (var m = 0; m < matches.length; m++) {
          if (isShowing(matches[m])) return PLATFORMS[p].name;
        }
      }
    }
    return null;
  }

  // A point on the control that a click would land on, in this frame's
  // viewport; null when something else is in front of the control there.
  function pointOn(element) {
    var box = element.getBoundingClientRect();
    var x = box.left + box.width / 2;
    var y = box.top + box.height / 2;
    if (x < 0 || y < 0 || x > window.innerWidth || y > window.innerHeight) return null;
    var root = element.getRootNode();
    var hit = (root.elementFromPoint ? root : document).elementFromPoint(x, y);
    var onIt = hit && (hit === element || element.contains(hit));
    return onIt ? { x: x, y: y } : null;
  }

  try {
    var roots = searchRoots(document, []);
    var found = findKnownPlatform(roots) || findByLabel(roots);
    if (!found) {
      var dialog = findDialog(roots);
      return dialog ? { dialog: dialog } : null;
    }
    if (request.watchMs) {
      return new Promise(function (resolve) {
        var timer = setTimeout(function () {
          resolve(false);
        }, request.watchMs);
        found.element.addEventListener(
          'click',
          function () {
            clearTimeout(timer);
            resolve(true);
          },
          { capture: true, once: true }
        );
      });
    }
    var label = (labelsOf(found.element)[0] || '').slice(0, 60);
    if (request.click) {
      found.element.click();
      return { name: found.name, label: label, x: null, y: null };
    }
    var point = pointOn(found.element);
    return { name: found.name, label: label, x: point && point.x, y: point && point.y };
  } catch (error) {
    return null;
  }
})
