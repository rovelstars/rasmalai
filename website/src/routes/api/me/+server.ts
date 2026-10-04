import { json } from '@sveltejs/kit';
import { sessionUser, readSessionCookie, userScopes } from '$lib/server/auth';
import { specHeaders } from '$lib/server/registry';

export async function GET({ request, platform }) {
	const env = (platform?.env ?? {}) as Record<string, string | undefined>;
	const user = await sessionUser(env, readSessionCookie(request.headers.get('Cookie')));
	if (!user) {
		return json({ user: null }, { headers: specHeaders() });
	}
	const scopes = await userScopes(env, user.id);
	return json({ user: { username: user.username, scopes: scopes.map((s) => `@${s}`) } }, { headers: specHeaders() });
}
