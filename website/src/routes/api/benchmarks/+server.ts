import { json } from '@sveltejs/kit';
import {
	getLatestBenchmarks,
	insertBenchmarkRun,
	listBenchmarkRuns,
	checkPublishToken,
	MAX_SNAPSHOT_BYTES
} from '$lib/server/db';
import { validateSnapshot, unwrapEnvelope } from '$lib/server/benchmarks';

// Turso-backed benchmark history. GET is public and edge-cached; POST is
// the monthly workflow ingesting one harness snapshot per run.
export async function GET({ platform, setHeaders, url }) {
	const env = (platform?.env ?? {}) as Record<string, string | undefined>;
	const run = await getLatestBenchmarks(env);
	setHeaders({ 'Cache-Control': 'public, max-age=3600, s-maxage=3600' });
	if (!run) return json({ snapshot: null, runs: [] });
	const limit = Number(url.searchParams.get('history') ?? 0);
	const runs = limit > 0 ? await listBenchmarkRuns(env, limit) : [];
	let snapshot: unknown = null;
	try {
		snapshot = JSON.parse(run.snapshotJson);
	} catch {
		snapshot = null;
	}
	return json({
		snapshot,
		run: {
			id: run.id,
			githubRunId: run.githubRunId,
			commitSha: run.commitSha,
			createdAt: run.createdAt
		},
		runs
	});
}

export async function POST({ request, platform }) {
	const env = (platform?.env ?? {}) as Record<string, string | undefined>;
	const auth = request.headers.get('Authorization') ?? '';
	const token = auth.startsWith('Bearer ') ? auth.slice(7) : '';
	if (!checkPublishToken(env, token)) {
		return json({ success: false, error: 'invalid publisher token' }, { status: 401 });
	}
	let body: Record<string, unknown>;
	try {
		body = (await request.json()) as Record<string, unknown>;
	} catch {
		return json({ success: false, error: 'invalid JSON body' }, { status: 400 });
	}
	const githubRunId = Number(body['githubRunId']);
	const commitSha = typeof body['commitSha'] === 'string' ? body['commitSha'] : '';
	const snapshot = unwrapEnvelope(body['snapshot'] ?? body);
	if (!Number.isInteger(githubRunId) || githubRunId <= 0) {
		return json({ success: false, error: 'missing githubRunId' }, { status: 400 });
	}
	if (!/^[0-9a-f]{4,40}$/i.test(commitSha)) {
		return json({ success: false, error: 'missing commitSha' }, { status: 400 });
	}
	const problem = validateSnapshot(snapshot);
	if (problem) {
		return json({ success: false, error: `invalid snapshot: ${problem}` }, { status: 400 });
	}
	const snapshotJson = JSON.stringify(snapshot);
	if (snapshotJson.length > MAX_SNAPSHOT_BYTES) {
		return json({ success: false, error: 'snapshot exceeds size limits' }, { status: 413 });
	}
	try {
		const run = await insertBenchmarkRun(env, githubRunId, commitSha, snapshotJson);
		return json({ success: true, id: run.id }, { status: 201 });
	} catch (e) {
		const msg = e instanceof Error ? e.message : String(e);
		if (msg.includes('UNIQUE') || msg.includes('unique')) {
			return json({ success: false, error: 'run already ingested' }, { status: 409 });
		}
		return json({ success: false, error: 'ingest failed' }, { status: 500 });
	}
}
