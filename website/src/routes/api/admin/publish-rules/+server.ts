import { json } from '@sveltejs/kit';
import { safeEqual } from '$lib/server/auth';
import { specHeaders } from '$lib/server/registry';
import { ensureRegistriesAdvisories, parseScanLimit, scanPublishReportOnly } from '$lib/server/publishRules';

// Operator surface for the registry-key rule (top-level `registry`/
// `registries` keys). GET walks the live set and counts what the publish
// gate rejects, so the count reads zero by construction. GET is read-only:
// the scan reassembles stored bytes and writes nothing. POST files the
// critical advisory for every counted violator that lacks one, and
// answers which ids it filed versus which were already present. Both are
// bounded per call by `limit` (default 200, max 1000) plus the
// per-package byte cap in publishRules.ts; a larger registry pages
// through repeated calls.
function adminToken(env: Record<string, string | undefined>, provided: string): boolean {
	const configured = env['ADMIN_TOKEN'];
	if (!configured) return false;
	return safeEqual(provided, configured);
}

export async function GET({ request, platform, url }) {
	const env = (platform?.env ?? {}) as Record<string, string | undefined>;
	const auth = request.headers.get('Authorization') ?? '';
	const token = auth.startsWith('Bearer ') ? auth.slice(7) : '';
	if (!adminToken(env, token)) {
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

export async function POST({ request, platform, url }) {
	const env = (platform?.env ?? {}) as Record<string, string | undefined>;
	const auth = request.headers.get('Authorization') ?? '';
	const token = auth.startsWith('Bearer ') ? auth.slice(7) : '';
	if (!adminToken(env, token)) {
		return json({ code: 'unauthorized', message: 'invalid admin token' }, { status: 401, headers: specHeaders() });
	}
	const limit = parseScanLimit(url.searchParams.get('limit'));
	try {
		const outcome = await ensureRegistriesAdvisories(env, { limit });
		return json(outcome, { headers: specHeaders() });
	} catch (e) {
		const message = e instanceof Error ? e.message : 'advisory filing failed';
		return json({ code: 'filing-failed', message }, { status: 500, headers: specHeaders() });
	}
}
