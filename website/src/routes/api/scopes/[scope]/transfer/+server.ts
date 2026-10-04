import { json } from '@sveltejs/kit';
import { transferScope } from '$lib/server/db';
import { specHeaders } from '$lib/server/registry';

export async function POST({ params, request, platform }) {
	const env = (platform?.env ?? {}) as Record<string, string | undefined>;
	const auth = request.headers.get('Authorization') ?? '';
	const token = auth.startsWith('Bearer ') ? auth.slice(7) : '';
	const configured = env['ADMIN_TOKEN'];
	if (!configured || token !== configured) {
		return json({ code: 'unauthorized', message: 'invalid admin token' }, { status: 401, headers: specHeaders() });
	}
	const scope = params.scope ?? '';
	if (!/^[a-z0-9][a-z0-9-]{0,31}$/.test(scope)) {
		return json({ code: 'bad-request', message: 'invalid scope' }, { status: 400, headers: specHeaders() });
	}
	let body: Record<string, unknown>;
	try {
		body = (await request.json()) as Record<string, unknown>;
	} catch {
		return json({ code: 'bad-request', message: 'invalid JSON body' }, { status: 400, headers: specHeaders() });
	}
	const newOwner = typeof body['newOwner'] === 'string' ? body['newOwner'] : '';
	const force = body['force'] === true;
	if (!/^[a-z0-9][a-z0-9-]{0,31}$/.test(newOwner)) {
		return json({ code: 'bad-request', message: 'invalid newOwner' }, { status: 400, headers: specHeaders() });
	}
	try {
		await transferScope(env, scope, newOwner, force);
		return json({ success: true, scope, newOwner }, { headers: specHeaders() });
	} catch (e) {
		const msg = e instanceof Error ? e.message : String(e);
		if (msg.includes('live versions')) {
			return json({ code: 'conflict', message: msg }, { status: 409, headers: specHeaders() });
		}
		return json({ code: 'transfer-failed', message: msg }, { status: 500, headers: specHeaders() });
	}
}
