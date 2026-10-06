import { json } from '@sveltejs/kit';
import { deleteChunks, findOrphanChunks, reverifyChunks } from '$lib/server/db';
import { safeEqual } from '$lib/server/auth';
import { r2Bucket, r2Delete } from '$lib/server/r2';
import { specHeaders } from '$lib/server/registry';
import { gcCutoffSec, parseGcLimit } from '$lib/server/gc';
import { newRequestId } from '$lib/server/requestId';

export async function POST({ request, platform, url }) {
	const env = (platform?.env ?? {}) as Record<string, string | undefined>;
	const auth = request.headers.get('Authorization') ?? '';
	const token = auth.startsWith('Bearer ') ? auth.slice(7) : '';
	const configured = env['ADMIN_TOKEN'];
	if (!configured || !safeEqual(token, configured)) {
		return json({ code: 'unauthorized', message: 'invalid admin token' }, { status: 401, headers: specHeaders() });
	}
	let bodyLimit: unknown;
	try {
		const body = (await request.json()) as Record<string, unknown>;
		bodyLimit = body['limit'];
	} catch {
		bodyLimit = undefined;
	}
	const limit = parseGcLimit(url.searchParams.get('limit') ?? bodyLimit);
	const requestId = newRequestId();
	try {
		const cutoff = gcCutoffSec(Math.floor(Date.now() / 1000));
		const candidates = await findOrphanChunks(env, cutoff, limit);
		const live = await reverifyChunks(env, candidates);
		const orphans = candidates.filter((h) => !live.has(h));
		let r2Deleted = 0;
		const bucket = r2Bucket(env as unknown as Record<string, unknown>);
		if (bucket) {
			for (const hash of orphans) {
				try {
					await r2Delete(bucket, hash);
					r2Deleted += 1;
				} catch (e) {
					console.error(`[gc ${requestId}] r2 delete failed for ${hash}:`, e instanceof Error ? e.message : String(e));
				}
			}
		}
		const deleted = await deleteChunks(env, orphans);
		return json({ scanned: candidates.length, orphans: orphans.length, deleted, r2Deleted }, { headers: specHeaders() });
	} catch (e) {
		console.error(`[gc ${requestId}] sweep failed:`, e instanceof Error ? e.message : String(e));
		return json({ code: 'gc-failed', message: 'garbage collection failed', requestId }, { status: 500, headers: specHeaders() });
	}
}
