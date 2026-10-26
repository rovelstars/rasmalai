import { json } from '@sveltejs/kit';
import { recordDownloadRollup } from '$lib/server/db';
import { safeEqual } from '$lib/server/auth';
import { specHeaders } from '$lib/server/registry';

// The operator-fed download rollup. The registry never counts
// per-download rows; whatever counter source the deployment wires up
// (edge analytics, log tail, CDN export) lands one batched daily row per
// package here, and the catalog reads the trailing-30-day sums.
export async function POST({ request, platform }) {
	const env = (platform?.env ?? {}) as Record<string, string | undefined>;
	const auth = request.headers.get('Authorization') ?? '';
	const token = auth.startsWith('Bearer ') ? auth.slice(7) : '';
	const configured = env['ADMIN_TOKEN'];
	if (!configured || !safeEqual(token, configured)) {
		return json({ code: 'unauthorized', message: 'invalid admin token' }, { status: 401, headers: specHeaders() });
	}
	let body: Record<string, unknown>;
	try {
		body = (await request.json()) as Record<string, unknown>;
	} catch {
		return json({ code: 'bad-request', message: 'invalid JSON body' }, { status: 400, headers: specHeaders() });
	}
	const day = typeof body['day'] === 'string' ? body['day'] : '';
	const counts = body['counts'];
	if (!/^\d{4}-\d{2}-\d{2}$/.test(day) || !counts || typeof counts !== 'object' || Array.isArray(counts)) {
		return json(
			{ code: 'bad-request', message: 'expected {"day": "YYYY-MM-DD", "counts": {"@scope/name": n}}' },
			{ status: 400, headers: specHeaders() }
		);
	}
	try {
		const recorded = await recordDownloadRollup(env, day, counts as Record<string, number>);
		return json({ success: true, day, recorded }, { headers: specHeaders() });
	} catch (e) {
		const message = e instanceof Error ? e.message : 'rollup failed';
		return json({ code: 'bad-request', message }, { status: 400, headers: specHeaders() });
	}
}
