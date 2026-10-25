import { json } from '@sveltejs/kit';
import {
	fileAdvisory,
	listActiveAdvisories,
	signFeed,
	type AdvisoryInput
} from '$lib/server/advisories';
import { PROTOCOL_HEADER, PROTOCOL_VERSION } from '$lib/server/canonical';
import { specHeaders } from '$lib/server/registry';
import { safeEqual } from '$lib/server/auth';

function headers(extra: Record<string, string> = {}): Record<string, string> {
	return { ...specHeaders(), ...extra };
}

function adminToken(env: Record<string, string | undefined>, provided: string): boolean {
	const configured = env['ADMIN_TOKEN'];
	if (!configured) return false;
	return safeEqual(provided, configured);
}

export async function GET({ platform }) {
	const env = (platform?.env ?? {}) as Record<string, string | undefined>;
	try {
		const advisories = await listActiveAdvisories(env);
		const feed = await signFeed(env, advisories);
		return json(feed, {
			headers: headers({ 'Cache-Control': 'public, max-age=300, s-maxage=300' })
		});
	} catch (e) {
		const message = e instanceof Error ? e.message : String(e);
		return json({ code: 'unavailable', message }, { status: 503, headers: headers() });
	}
}

export async function POST({ request, platform }) {
	const env = (platform?.env ?? {}) as Record<string, string | undefined>;
	const auth = request.headers.get('Authorization') ?? '';
	const token = auth.startsWith('Bearer ') ? auth.slice(7) : '';
	if (!adminToken(env, token)) {
		return json({ code: 'unauthorized', message: 'invalid admin token' }, { status: 401, headers: headers() });
	}
	const protocol = request.headers.get(PROTOCOL_HEADER) ?? request.headers.get(PROTOCOL_HEADER.toLowerCase()) ?? '';
	if (protocol !== PROTOCOL_VERSION) {
		return json(
			{ code: 'bad-protocol', message: `unsupported advisory protocol (expected \`${PROTOCOL_HEADER}: ${PROTOCOL_VERSION}\`)` },
			{ status: 400, headers: headers() }
		);
	}
	let body: unknown;
	try {
		body = await request.json();
	} catch {
		return json({ code: 'bad-request', message: 'invalid JSON body' }, { status: 400, headers: headers() });
	}
	const input = (body ?? {}) as Record<string, unknown>;
	const advisory: AdvisoryInput = {
		id: typeof input['id'] === 'string' ? input['id'] : undefined,
		package: typeof input['package'] === 'string' ? input['package'] : '',
		affected: typeof input['affected'] === 'string' ? input['affected'] : undefined,
		artifacts: Array.isArray(input['artifacts']) ? input['artifacts'].map(String) : undefined,
		severity: typeof input['severity'] === 'string' ? input['severity'] : '',
		title: typeof input['title'] === 'string' ? input['title'] : undefined,
		detail: typeof input['detail'] === 'string' ? input['detail'] : undefined
	};
	try {
		const filed = await fileAdvisory(env, advisory);
		return json({ success: true, advisory: filed }, { status: 201, headers: headers() });
	} catch (e) {
		const message = e instanceof Error ? e.message : String(e);
		if (message.includes('already exists')) {
			return json({ code: 'conflict', message }, { status: 409, headers: headers() });
		}
		if (message.includes('missing database')) {
			return json({ code: 'unavailable', message }, { status: 503, headers: headers() });
		}
		return json({ code: 'bad-request', message }, { status: 400, headers: headers() });
	}
}
