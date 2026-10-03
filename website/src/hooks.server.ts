import type { Handle } from '@sveltejs/kit';

// Edge cache for registry-backed responses (Cloudflare Cache API —
// built-in, no binding, no dashboard rule). Turso is single-region, so
// every origin read is a transatlantic round-trip; this keeps repeat
// views global and instant. TTLs mirror the Cache-Control the routes
// already emit. POST/PUT/DELETE and preview tokens bypass.
const TTL: Array<[RegExp, number]> = [
	[/^\/api\/packages(\?|$)/, 300],
	[/^\/packages\//, 300]
];

function ttlFor(path: string): number | null {
	for (const [re, ttl] of TTL) {
		if (re.test(path)) return ttl;
	}
	return null;
}

export const handle: Handle = async ({ event, resolve }) => {
	const cache =
		event.request.method === 'GET' &&
		typeof caches !== 'undefined' &&
		ttlFor(event.url.pathname) !== null
			? (caches as unknown as { default: Cache }).default
			: null;
	if (!cache) return resolve(event);
	const ttl = ttlFor(event.url.pathname) as number;
	const key = new Request(event.request.url, { method: 'GET' });
	try {
		const hit = await cache.match(key);
		if (hit) {
			const at = Number(hit.headers.get('x-edge-cached-at') ?? 0);
			if (Date.now() - at < ttl * 1000) return hit;
		}
	} catch {
		/* cache read failure falls through to origin */
	}
	const res = await resolve(event);
	if (res.ok) {
		try {
			const stamped = new Response(res.body, res);
			stamped.headers.set('x-edge-cached-at', String(Date.now()));
			await cache.put(key, stamped.clone());
			return stamped;
		} catch {
			/* cache write failure still returns the origin response */
		}
	}
	return res;
};
