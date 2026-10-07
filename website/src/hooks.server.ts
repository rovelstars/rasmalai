import type { Handle } from '@sveltejs/kit';

// Edge cache for registry-backed responses (Cloudflare Cache API —
// built-in, no binding, no dashboard rule). Turso is single-region, so
// every origin read is a transatlantic round-trip; this keeps repeat
// views global and instant.
//
// No purge credentials exist, so lifetimes follow row mutability instead:
// versioned rows are write-once (a publish inserts a new row; only the
// yank status can flip, which the route headers already bound to a day),
// while the latest-pointer (unpinned package pages, catalog listing)
// moves on every publish and stays short at 5 minutes. First match wins,
// so the pinned-version rule precedes the /packages/ prefix. POST/PUT/
// DELETE and preview tokens bypass (no match).
//
// No stale-while-revalidate: the Cache API has no SWR primitive, and
// background revalidation would need waitUntil, which is not available in
// every runtime this handle runs in (prerender, dev). Tiered TTLs give
// the same shape: immutable rows cache long, pointers revalidate fast.
const TTL: Array<[RegExp, number]> = [
	[/^\/packages\/.+\/\d+\.\d+\.\d+($|\/)/, 86400],
	[/^\/packages\//, 300],
	[/^\/api\/packages(\?|$)/, 300],
	[/^\/api\/benchmarks(\?|$)/, 3600],
	[/^\/api\/version(\?|$)/, 3600]
];

function ttlFor(path: string): number | null {
	for (const [re, ttl] of TTL) {
		if (re.test(path)) return ttl;
	}
	return null;
}

export const handle: Handle = async ({ event, resolve }) => {
	const isApi = event.url.pathname.startsWith('/api/');
	if (event.request.method === 'OPTIONS' && isApi) {
		return new Response(null, {
			status: 204,
			headers: {
				'Access-Control-Allow-Origin': '*',
				'Access-Control-Allow-Methods': 'GET, HEAD, OPTIONS',
				'Access-Control-Allow-Headers': 'Content-Type',
				'Access-Control-Max-Age': '86400'
			}
		});
	}
	const stampCors = (res: Response): Response => {
		if (event.request.method !== 'GET' || !isApi) return res;
		const out = new Response(res.body, res);
		out.headers.set('Access-Control-Allow-Origin', '*');
		return out;
	};
	const cache =
		event.request.method === 'GET' &&
		typeof caches !== 'undefined' &&
		ttlFor(event.url.pathname) !== null
			? (caches as unknown as { default: Cache }).default
			: null;
	if (!cache) return stampCors(await resolve(event));
	const ttl = ttlFor(event.url.pathname) as number;
	const key = new Request(event.request.url, { method: 'GET' });
	try {
		const hit = await cache.match(key);
		if (hit) {
			const at = Number(hit.headers.get('x-edge-cached-at') ?? 0);
			if (Date.now() - at < ttl * 1000) return stampCors(hit);
		}
	} catch {
		/* cache read failure falls through to origin */
	}
	const res = await resolve(event);
	if (res.ok) {
		try {
			const stamped = new Response(res.body, res);
			stamped.headers.set('x-edge-cached-at', String(Date.now()));
			stamped.headers.set('Access-Control-Allow-Origin', '*');
			await cache.put(key, stamped.clone());
			return stamped;
		} catch {
			/* cache write failure still returns the origin response */
		}
	}
	return stampCors(res);
};
