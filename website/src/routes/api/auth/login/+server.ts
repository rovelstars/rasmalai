import { json } from '@sveltejs/kit';
import { login, sessionCookie, clientIp, isAuthBlocked, auth_attempts } from '$lib/server/auth';
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
	const ip = clientIp(request.headers);
	if (await isAuthBlocked(env, ip, username)) {
		return json({ code: 'rate-limited', message: 'too many failed attempts, try again later' }, { status: 429, headers: specHeaders() });
	}
	try {
		const session = await login(env, username, password);
		await auth_attempts(env, ip, username, true);
		const secure = url.protocol === 'https:';
		return json(
			{ success: true, user: { username: session.user.username } },
			{ headers: { 'Set-Cookie': sessionCookie(session.token, 30 * 86400, secure), ...specHeaders() } }
		);
	} catch {
		const allowed = await auth_attempts(env, ip, username, false);
		if (!allowed) {
			return json({ code: 'rate-limited', message: 'too many failed attempts, try again later' }, { status: 429, headers: specHeaders() });
		}
		return json({ code: 'unauthorized', message: 'invalid username or password' }, { status: 401, headers: specHeaders() });
	}
}
