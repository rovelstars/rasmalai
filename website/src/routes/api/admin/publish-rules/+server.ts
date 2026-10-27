import { json } from '@sveltejs/kit';
import { safeEqual } from '$lib/server/auth';
import { specHeaders } from '$lib/server/registry';
import { parseScanLimit, scanPublishReportOnly } from '$lib/server/publishRules';

// Operator surface for the report-only publish rules (S103 bare
// `__rnx_*` uses, top-level `registry`/`registries` keys). One GET walks
// the live set and counts what the enforcement flip would reject, so the
// migration starts from measured counts instead of guesses. Read-only:
// the scan reassembles stored bytes and writes nothing. Bounded per
// call by `limit` (default 200, max 1000) plus the per-package byte cap
// in publishRules.ts; a larger registry pages through repeated calls.
export async function GET({ request, platform, url }) {
	const env = (platform?.env ?? {}) as Record<string, string | undefined>;
	const auth = request.headers.get('Authorization') ?? '';
	const token = auth.startsWith('Bearer ') ? auth.slice(7) : '';
	const configured = env['ADMIN_TOKEN'];
	if (!configured || !safeEqual(token, configured)) {
		return json({ code: 'unauthorized', message: 'invalid admin token' }, { status: 401, headers: specHeaders() });
	}
	const limit = parseScanLimit(url.searchParams.get('limit'));
	try {
		const scan = await scanPublishReportOnly(env, { limit });
		return json(scan, { headers: specHeaders() });
	} catch (e) {
		const message = e instanceof Error ? e.message : 'scan failed';
		return json({ code: 'scan-failed', message }, { status: 500, headers: specHeaders() });
	}
}
