// Self-unregistering service worker — replaces old cached worker that caused ERR_CACHE_MISS
self.addEventListener('install', function() {
  self.skipWaiting();
});
self.addEventListener('activate', function() {
  self.registration.unregister().then(function() {
    self.clients.matchAll().then(function(clients) {
      clients.forEach(function(c) { c.navigate(c.url); });
    });
  });
});
