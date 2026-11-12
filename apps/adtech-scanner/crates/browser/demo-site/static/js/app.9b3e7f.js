/* The Meridian Post - article page behaviour (navigation, theme, reading progress, in-page routing). */
(function (win, doc) {
  'use strict';

  var kit = win.TinyKit;
  var site = win.__MERIDIAN__ || {};

  function initNavigation() {
    var toggle = doc.querySelector('[data-nav-toggle]');
    var nav = doc.querySelector('[data-nav]');
    if (!toggle || !nav) return;
    toggle.addEventListener('click', function () {
      var open = nav.classList.toggle('is-open');
      toggle.setAttribute('aria-expanded', String(open));
    });
  }

  function initTheme() {
    var button = doc.querySelector('[data-theme-toggle]');
    if (!button) return;
    button.addEventListener('click', function () {
      var root = doc.documentElement;
      var next = root.getAttribute('data-theme') === 'dark' ? 'light' : 'dark';
      root.setAttribute('data-theme', next);
      try {
        win.localStorage.setItem('meridian-theme', next);
      } catch (storageBlocked) {}
    });
  }

  function initReadingProgress() {
    var bar = doc.querySelector('.reading-progress span');
    var article = doc.querySelector('.article__body');
    if (!bar || !article) return;
    var update = kit.throttle(function () {
      var rect = article.getBoundingClientRect();
      var total = rect.height - win.innerHeight;
      var read = Math.min(1, Math.max(0, -rect.top / Math.max(1, total)));
      bar.style.width = (read * 100).toFixed(1) + '%';
    }, 80);
    win.addEventListener('scroll', update, { passive: true });
    update();
  }

  function initReadingTime() {
    var target = doc.querySelector('[data-reading-time]');
    var body = doc.querySelector('.article__body');
    if (!target || !body) return;
    var words = body.textContent.trim().split(/\s+/).length;
    target.textContent = Math.max(1, Math.round(words / 230)) + ' min read';
  }

  function initRelativeTimes() {
    kit.each('[data-relative-time]', doc, function (element) {
      var date = new Date(element.getAttribute('datetime'));
      if (!isNaN(date)) element.title = kit.relativeTime(date);
    });
  }

  function initNewsletter() {
    var form = doc.querySelector('[data-newsletter]');
    if (!form) return;
    form.addEventListener('submit', function (event) {
      event.preventDefault();
      form.innerHTML = '<p>Thanks. Check your inbox to confirm.</p>';
      (win.dataLayer = win.dataLayer || []).push({ event: 'newsletter_signup', section: site.section });
    });
  }

  function initStickyAd() {
    var close = doc.querySelector('[data-sticky-close]');
    var container = doc.querySelector('[data-sticky-ad]');
    if (!close || !container) return;
    close.addEventListener('click', function () {
      container.hidden = true;
    });
  }

  // Story cards load their article into the page instead of loading a page.
  var ENTRY_ARTICLE = 'tide-powered-town';

  function fragmentUrl(path) {
    var slug = path.replace(/^\/climate\//, '').replace(/^\/$/, ENTRY_ARTICLE);
    return '/fragments/' + slug + '.html';
  }

  function showArticle(path, pushed) {
    return fetch(fragmentUrl(path))
      .then(function (response) {
        if (!response.ok) throw new Error('HTTP ' + response.status);
        return response.text();
      })
      .then(function (html) {
        var holder = doc.createElement('div');
        holder.innerHTML = html;
        var next = holder.querySelector('.article');
        var current = doc.querySelector('.article');
        if (!next || !current) throw new Error('no article');
        current.replaceWith(next);
        doc.title = next.getAttribute('data-title') + ' | The Meridian Post';
        site.articleId = next.getAttribute('data-article-id');
        if (pushed) win.history.pushState({ article: site.articleId }, '', path);
        win.scrollTo(0, 0);
        initArticle();
        win.dispatchEvent(new CustomEvent('meridian:route', { detail: { path: path, article: site.articleId } }));
      })
      .catch(function () {
        // The article could not be loaded in place: load its page.
        win.location.assign(path);
      });
  }

  function initRouter() {
    doc.addEventListener('click', function (event) {
      var link = event.target.closest ? event.target.closest('a[data-route]') : null;
      var plainClick = event.button === 0 && !event.metaKey && !event.ctrlKey && !event.shiftKey && !event.altKey;
      if (!link || !plainClick || event.defaultPrevented) return;
      event.preventDefault();
      showArticle(link.getAttribute('href'), true);
    });
    win.addEventListener('popstate', function () {
      showArticle(win.location.pathname, false);
    });
  }

  function initArticle() {
    kit.each('[data-tabs]', doc, kit.tabs);
    kit.each('[data-accordion]', doc, kit.accordion);
    initReadingProgress();
    initReadingTime();
    initRelativeTimes();
  }

  initArticle();
  initNavigation();
  initTheme();
  initNewsletter();
  initStickyAd();
  initRouter();
})(window, document);
