self.addEventListener('install', function () { self.skipWaiting(); });
self.addEventListener('activate', function (event) { event.waitUntil(self.clients.claim()); });
self.addEventListener('fetch', function (event) {
  if (event.request.url.indexOf('through-the-service-worker') >= 0) {
    event.respondWith(fetch('/status/200?asked-by-the-service-worker'));
  }
});
