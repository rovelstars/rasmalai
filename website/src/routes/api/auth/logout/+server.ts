import { json } from '@sveltejs/kit';
import { logout, readSessionCookie, clearSessionCookie } from '$lib/server/auth';
import { specHeaders } from '$lib/server/registry';

export async function POST({ request, platform, url }) {
	const env = (platform?.env ?? {}) as Record<string, string | undefined>;
	await logout(env, readSessionCookie(request.headers.get('Cookie')));
	const secure = url.protocol === 'https:';
	return json({ success: true }, { headers: { 'Set-Cookie': clearSessionCookie(secure), ...specHeaders() } });
}
