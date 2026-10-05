import { json } from '@sveltejs/kit';
import { resolveGraph, type HaveEntry } from '$lib/server/db';
import { specHeaders } from '$lib/server/registry';

function err(code: string, message: string, status: number) {
	return json({ code, message }, { status, headers: specHeaders() });
}

// One round trip, depth-leveled, with a have-list for deltas. Pinned
// requirement sets resolve deterministically; ranged sets re-resolve live.
export async function POST({ request, url, platform }) {
	let body: Record<string, unknown>;
	try {
		body = (await request.json()) as Record<string, unknown>;
	} catch {
		return err('bad-request', 'invalid JSON body', 400);
	}
	const reqs = body['requirements'];
	if (!reqs || typeof reqs !== 'object' || Object.keys(reqs).length === 0) {
		return err('bad-request', 'missing requirements map', 400);
	}
	const requirements = reqs as Record<string, string>;
	for (const [name, range] of Object.entries(requirements)) {
		if (typeof range !== 'string') {
			return err('bad-request', `requirement for ${name} must be a version range string`, 400);
		}
	}
	const have: HaveEntry[] = Array.isArray(body['have'])
		? (body['have'] as unknown[]).flatMap((h) => {
				if (h && typeof h === 'object') {
					const o = h as Record<string, unknown>;
					if (typeof o['full'] === 'string' && typeof o['version'] === 'string') {
						return [{ full: o['full'], version: o['version'], integrity: String(o['integrity'] ?? '') }];
					}
				}
				return [];
			})
		: [];
	const env = (platform?.env ?? {}) as Record<string, string | undefined>;
	try {
		const { resolved, levels } = await resolveGraph(env, requirements, have);
		const base = `${url.origin}/api/packages`;
		return json({ resolved, base, levels }, { headers: specHeaders() });
	} catch (e) {
		const msg = e instanceof Error ? e.message : String(e);
		if (msg.startsWith('dependency cycle')) return err('cycle', msg, 422);
		if (msg.includes('does not exist')) return err('not-found', msg, 404);
		if (msg.includes('satisfies') || msg.includes('conflicting')) {
			return err('unsatisfiable', msg, 422);
		}
		if (msg.includes('invalid package name')) return err('bad-request', msg, 400);
		return err('resolve-failed', 'resolution failed', 500);
	}
}
