import { listPackages } from '$lib/server/db';

export const prerender = false;

export async function load({ platform, setHeaders }) {
	const env = (platform?.env ?? {}) as Record<string, string | undefined>;
	let packages: Awaited<ReturnType<typeof listPackages>> = [];
	try {
		packages = await listPackages(env);
	} catch {
		packages = [];
	}
	// Catalog aggregate: edge holds an hour, browsers a minute. Freshness
	// on publish comes from purging, not TTL (see packagePointerUrls).
	setHeaders({ 'Cache-Control': 'public, max-age=60, s-maxage=3600' });
	return { packages };
}
