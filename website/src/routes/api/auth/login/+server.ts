import { json } from '@sveltejs/kit';
import { login, sessionCookie } from '$lib/server/auth';
import { specHeaders } from '$lib/server/registry';

export async function POST({ request, platform, url }) {
	const env = (platform?.env ?? {}) as Record<string, string | undefined>;
	let body: Record<string, unknown>;
	try {
		body = (await request.json()) as Record<string, unknown>;
	} catch {
		return json({ code: 'bad-request', message: 'invalid JSON body' }, { status: 400, headers: specHeaders() });
	}
	const username = typeof body['username'] === 'string' ? body['username'] : '';
	const password = typeof body['password'] === 'string' ? body['password'] : '';
	try {
		const session = await login(env, username, password);
		const secure = url.protocol === 'https:';
		return json(
			{ success: true, user: { username: session.user.username } },
			{ headers: { 'Set-Cookie': sessionCookie(session.token, 30 * 86400, secure), ...specHeaders() } }
		);
	} catch {
		return json({ code: 'unauthorized', message: 'invalid username or password' }, { status: 401, headers: specHeaders() });
	}
}
