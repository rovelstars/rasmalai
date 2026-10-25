import { json } from '@sveltejs/kit';
import { withdrawAdvisory } from '$lib/server/advisories';
import { PROTOCOL_HEADER, PROTOCOL_VERSION } from '$lib/server/canonical';
import { specHeaders } from '$lib/server/registry';
import { safeEqual } from '$lib/server/auth';

function adminToken(env: Record<string, string | undefined>, provided: string): boolean {
	const configured = env['ADMIN_TOKEN'];
	if (!configured) return false;
	return safeEqual(provided, configured);
}

export async function POST({ request, params, platform }) {
	const env = (platform?.env ?? {}) as Record<string, string | undefined>;
	const auth = request.headers.get('Authorization') ?? '';
	const token = auth.startsWith('Bearer ') ? auth.slice(7) : '';
	if (!adminToken(env, token)) {
		return json({ code: 'unauthorized', message: 'invalid admin token' }, { status: 401, headers: specHeaders() });
	}
	const protocol = request.headers.get(PROTOCOL_HEADER) ?? request.headers.get(PROTOCOL_HEADER.toLowerCase()) ?? '';
	if (protocol !== PROTOCOL_VERSION) {
		return json(
			{ code: 'bad-protocol', message: `unsupported advisory protocol (expected \`${PROTOCOL_HEADER}: ${PROTOCOL_VERSION}\`)` },
			{ status: 400, headers: specHeaders() }
		);
	}
	const id = params.id ?? '';
	try {
		const withdrawn = await withdrawAdvisory(env, id);
		if (!withdrawn) {
			return json(
				{ code: 'not-found', message: `advisory ${id} does not exist or is already withdrawn` },
				{ status: 404, headers: specHeaders() }
			);
		}
		return json({ success: true, advisory: withdrawn }, { headers: specHeaders() });
	} catch (e) {
		const message = e instanceof Error ? e.message : String(e);
		return json({ code: 'unavailable', message }, { status: 503, headers: specHeaders() });
	}
}
