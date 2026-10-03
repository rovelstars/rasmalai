import { listPackages } from '$lib/server/db';

export const prerender = false;

export async function load({ platform }) {
	const env = (platform?.env ?? {}) as Record<string, string | undefined>;
	const packages = await listPackages(env);
	return { packages };
}
