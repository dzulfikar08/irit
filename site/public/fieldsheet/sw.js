// Caches the app's own files so it opens without a connection. Data lives in IndexedDB, not here.
// Bump VERSION when any file below changes.
const VERSION = 'fieldsheet-v1';
const FILES = ['./', 'index.html', 'style.css', 'app.js', 'manifest.webmanifest', 'icon.svg', 'icon-192.png', 'icon-512.png'];

self.addEventListener('install', (e) => {
	e.waitUntil(caches.open(VERSION).then((c) => c.addAll(FILES)).then(() => self.skipWaiting()));
});

self.addEventListener('activate', (e) => {
	e.waitUntil(
		caches.keys()
			.then((keys) => Promise.all(keys.filter((k) => k.startsWith('fieldsheet-') && k !== VERSION).map((k) => caches.delete(k))))
			.then(() => self.clients.claim())
	);
});

// Cache first, then network; a successful network answer refreshes the cache.
self.addEventListener('fetch', (e) => {
	const req = e.request;
	if (req.method !== 'GET' || new URL(req.url).origin !== location.origin) return;
	e.respondWith(
		caches.match(req, { ignoreSearch: true }).then((hit) => {
			const net = fetch(req)
				.then((res) => {
					if (res.ok) {
						const copy = res.clone();
						caches.open(VERSION).then((c) => c.put(req, copy));
					}
					return res;
				})
				.catch(() => hit);
			return hit || net;
		})
	);
});
