import { json } from '@sveltejs/kit';

export async function GET({ platform }) {
	const env = (platform?.env ?? {}) as Record<string, string | undefined>;
	const url = env['TURSO_DATABASE_URL'];
	if (!url) {
		return json({ status: 'ok', database: 'mock', timestamp: Date.now() });
	}
	try {
		const { createClient } = await import('@libsql/client/web');
		const db = createClient({ url, authToken: env['TURSO_AUTH_TOKEN'] });
		await db.execute('SELECT 1');
		return json({ status: 'ok', database: 'turso', timestamp: Date.now() });
	} catch {
		return json({ status: 'ok', database: 'mock', timestamp: Date.now() });
	}
}
