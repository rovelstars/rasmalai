import { json } from '@sveltejs/kit';
import {
	parsePackageName,
	getVersionIndexRow,
	specialDeleteVersion,
	purgeUrls,
	purgeConfigured,
	packagePointerUrls
} from '$lib/server/db';
import { safeEqual } from '$lib/server/auth';
import { specHeaders, splitNameVersion } from '$lib/server/registry';

function headers(extra: Record<string, string> = {}): Record<string, string> {
	return { ...specHeaders(), ...extra };
}

// Staff-only hard removal. It exists for one scope only (`@std/*`), needs
// the org admin token plus a second approval token held by a second
// admin, and has no CLI path: the server checks the scope itself, never a
// client flag. The row is hard-deleted, the purge broadcasts, and the
// audit entry survives, so later reads answer the 410 `withdrawn
// (std-purge)` variant and the number stays burned.
export async function DELETE({ params, request, platform, url }) {
	const env = (platform?.env ?? {}) as Record<string, string | undefined>;
	const scope = params.scope ?? '';
	const nv = splitNameVersion(params.target ?? '');
	const parsed = nv.version ? parsePackageName(scope ? `@${scope}/${nv.name}` : nv.name) : null;
	if (!parsed || !parsed.scope || !nv.version) {
		return json({ code: 'not-found', message: 'invalid package path' }, { status: 404, headers: headers() });
	}
	if (parsed.scope !== 'std') {
		return json(
			{ code: 'forbidden', message: 'special delete is @std-only' },
			{ status: 403, headers: headers() }
		);
	}
	const auth = request.headers.get('Authorization') ?? '';
	const token = auth.startsWith('Bearer ') ? auth.slice(7) : '';
	const configured = env['ADMIN_TOKEN'];
	const second = env['ADMIN_DELETE_TOKEN'];
	if (!configured || !safeEqual(token, configured)) {
		return json({ code: 'unauthorized', message: 'invalid admin token' }, { status: 401, headers: headers() });
	}
	const approval = request.headers.get('X-RNX-Second-Approval') ?? '';
	if (!second || second === configured || !safeEqual(approval, second)) {
		return json(
			{ code: 'forbidden', message: 'special delete needs a second approval token' },
			{ status: 403, headers: headers() }
		);
	}
	const full = parsed.full;
	const version = nv.version;
	const found = await getVersionIndexRow(env, full, version);
	if (!found) {
		return json(
			{ code: 'not-found', message: `package ${full}@${version} does not exist` },
			{ status: 404, headers: headers() }
		);
	}
	const deleted = await specialDeleteVersion(env, full, version, 'org-admin');
	if (!deleted) {
		return json(
			{ code: 'not-found', message: `package ${full}@${version} does not exist` },
			{ status: 404, headers: headers() }
		);
	}
	const purge = await purgeUrls(env, packagePointerUrls(url.origin, full, version), {
		fullName: full,
		prefixes: [`${url.origin}/api/v0/packages/${full}@${version}`]
	});
	if (purgeConfigured(env) && !purge.verified) {
		return json(
			{ code: 'unavailable', message: 'the edge still serves the old copy, retry the purge' },
			{ status: 503, headers: headers() }
		);
	}
	return json({ success: true, version, status: 'special-deleted' }, { headers: headers() });
}
