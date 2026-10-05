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
	// Latest-pointer aggregate: moves on every publish, so it stays short
	// (mirrors the /api/packages edge TTL in hooks.server.ts).
	setHeaders({ 'Cache-Control': 'public, max-age=60, s-maxage=300' });
	return { packages };
}
