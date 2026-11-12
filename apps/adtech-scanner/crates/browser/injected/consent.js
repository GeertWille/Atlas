// Finds a consent banner's "accept all" control and clicks it.
// Evaluates to the name of the banner that was accepted, or null.
(function () {
  'use strict';

  // Accept buttons of the consent platforms most publishers use.
  var KNOWN_BANNERS = [
    ['OneTrust', '#onetrust-accept-btn-handler'],
    ['Didomi', '#didomi-notice-agree-button'],
    ['Cookiebot', '#CybotCookiebotDialogBodyLevelButtonLevelOptinAllowAll, #CybotCookiebotDialogBodyButtonAccept'],
    ['Quantcast / InMobi Choice', '.qc-cmp2-summary-buttons button[mode="primary"], .qc-cmp2-footer button[mode="primary"]'],
    ['Usercentrics', '[data-testid="uc-accept-all-button"], button[data-action-type="accept"]'],
    ['TrustArc', '#truste-consent-button, .trustarc-agree-btn'],
    ['Sourcepoint', 'button.sp_choice_type_11, button.sp_choice_type_ACCEPT_ALL, .message-component[title="Accept All"]'],
    ['Google Funding Choices', '.fc-cta-consent'],
    ['Osano', '.osano-cm-accept-all'],
    ['Iubenda', '.iubenda-cs-accept-btn'],
    ['Complianz', '.cmplz-btn.cmplz-accept'],
    ['CookieYes', '.cky-btn-accept'],
  ];

  // Unambiguous "accept everything" labels: English, Dutch, German, French,
  // Spanish, Italian, Portuguese.
  var ACCEPT_ALL = [
    'accept all', 'accept all cookies', 'allow all', 'allow all cookies', 'agree to all', 'accept & continue',
    'accept and continue', 'agree and proceed', 'agree & proceed', 'yes, i agree', 'i accept all',
    'alles accepteren', 'accepteer alles', 'alle cookies accepteren', 'alles toestaan', 'alle cookies toestaan',
    'ik ga akkoord', 'akkoord en doorgaan',
    'alle akzeptieren', 'alles akzeptieren', 'alle cookies akzeptieren', 'alle zulassen', 'allen zustimmen',
    'akzeptieren und weiter', 'zustimmen und weiter',
    'tout accepter', 'accepter tout', 'accepter et continuer', 'accepter et fermer', "j'accepte tout",
    'tout autoriser',
    'aceptar todo', 'aceptar todas', 'aceptar todas las cookies', 'aceptar y continuar', 'permitir todas',
    'aceptar y cerrar',
    'accetta tutto', 'accetta tutti', 'accetta tutti i cookie', 'accetta e continua', 'accetta e chiudi',
    'aceitar tudo', 'aceitar todos', 'aceitar todos os cookies', 'aceitar e continuar', 'aceitar e fechar',
  ];
  // Short labels that only mean consent when they sit inside a consent dialog.
  var ACCEPT_IN_CONTEXT = [
    'accept', 'agree', 'i agree', 'i accept', 'allow', 'got it', 'ok',
    'accepteren', 'akkoord', 'toestaan',
    'akzeptieren', 'zustimmen', 'einverstanden', 'ich stimme zu',
    'accepter', "j'accepte", "d'accord", 'autoriser',
    'aceptar', 'acepto', 'de acuerdo', 'permitir',
  ];
  var CONSENT_CONTEXT = /cookie|consent|privacy|gdpr|rgpd|datenschutz|privacidad|confidentialit|cmp/i;
  var CLICKABLE = 'button, [role="button"], a, input[type="button"], input[type="submit"]';

  // The document plus everything reachable from it: open shadow roots and
  // same-origin iframes. Cross-origin iframes are evaluated separately.
  function searchRoots(root, roots) {
    roots.push(root);
    var elements = root.querySelectorAll('*');
    for (var i = 0; i < elements.length; i++) {
      var element = elements[i];
      if (element.shadowRoot) searchRoots(element.shadowRoot, roots);
      if (element.tagName === 'IFRAME') {
        try {
          if (element.contentDocument) searchRoots(element.contentDocument, roots);
        } catch (crossOrigin) {}
      }
    }
    return roots;
  }

  function isClickable(element) {
    if (element.disabled || !element.getClientRects().length) return false;
    var view = element.ownerDocument.defaultView || window;
    var style = view.getComputedStyle(element);
    return style.visibility !== 'hidden' && style.pointerEvents !== 'none' && Number(style.opacity) > 0.05;
  }

  function labelOf(element) {
    var text = element.tagName === 'INPUT' ? element.value : element.innerText || element.textContent;
    return String(text || '')
      .replace(/\s+/g, ' ')
      .replace(/[’`]/g, "'")
      .trim()
      .toLowerCase();
  }

  // `<html>` and `<body>` are not a dialog: feature-detection classes such as
  // Modernizr's "cookies" would put every button of the page in consent context.
  function isInsideConsentDialog(element) {
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

  function findKnownBanner(roots) {
    for (var b = 0; b < KNOWN_BANNERS.length; b++) {
      for (var r = 0; r < roots.length; r++) {
        var matches = roots[r].querySelectorAll(KNOWN_BANNERS[b][1]);
        for (var m = 0; m < matches.length; m++) {
          if (isClickable(matches[m])) return { name: KNOWN_BANNERS[b][0], element: matches[m] };
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
        var label = labelOf(element);
        if (!label || label.length > 40 || !isClickable(element)) continue;
        if (ACCEPT_ALL.indexOf(label) >= 0) return { name: 'button "' + label + '"', element: element };
        if (!inContext && ACCEPT_IN_CONTEXT.indexOf(label) >= 0 && isInsideConsentDialog(element)) {
          inContext = { name: 'button "' + label + '"', element: element };
        }
      }
    }
    return inContext;
  }

  try {
    var roots = searchRoots(document, []);
    var found = findKnownBanner(roots) || findByLabel(roots);
    if (!found) return null;
    found.element.click();
    return found.name;
  } catch (error) {
    return null;
  }
})()
