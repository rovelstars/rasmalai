import { json } from '@sveltejs/kit';
import { signup, login, sessionCookie, validateUsername } from '$lib/server/auth';
import { specHeaders } from '$lib/server/registry';

export async function POST({ request, platform }) {
	const env = (platform?.env ?? {}) as Record<string, string | undefined>;
	let body: Record<string, unknown>;
	try {
		body = (await request.json()) as Record<string, unknown>;
	} catch {
		return json({ code: 'bad-request', message: 'invalid JSON body' }, { status: 400, headers: specHeaders() });
	}
	const username = typeof body['username'] === 'string' ? body['username'] : '';
	const password = typeof body['password'] === 'string' ? body['password'] : '';
	const disclaimerAck = body['disclaimerAck'] === true;
	const problem = validateUsername(username);
	if (problem) {
		return json({ code: 'bad-request', message: `invalid username: ${problem}` }, { status: 400, headers: specHeaders() });
	}
	try {
		const user = await signup(env, username, password, disclaimerAck);
		const session = await login(env, username, password);
		return json(
			{ success: true, user: { username: user.username, scopes: [`@${username}`] } },
			{ status: 201, headers: { 'Set-Cookie': sessionCookie(session.token, 30 * 86400), ...specHeaders() } }
		);
	} catch (e) {
		const msg = e instanceof Error ? e.message : String(e);
		if (msg.startsWith('invalid username') || msg.startsWith('password must') || msg.startsWith('testing-phase')) {
			return json({ code: 'bad-request', message: msg }, { status: 400, headers: specHeaders() });
		}
		if (msg === 'username is taken') {
			return json({ code: 'conflict', message: msg }, { status: 409, headers: specHeaders() });
		}
		return json({ code: 'signup-failed', message: msg }, { status: 500, headers: specHeaders() });
	}
}
