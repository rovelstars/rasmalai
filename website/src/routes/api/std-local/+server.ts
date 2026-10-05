import { json } from '@sveltejs/kit';
import { dev } from '$app/environment';
import { localStdSnapshot } from '$lib/server/std-local';

// Dev-only live stdlib snapshot: same `rnx doc` output the prod seed
// uploads, served straight from working-copy sources so doc edits show
// without a seed, a push, or a Turso round-trip. Prod 404s here and uses
// the versioned static files instead.
export async function GET() {
	if (!dev) {
		return json({ code: 'not-found', message: 'dev only' }, { status: 404 });
	}
	const snap = localStdSnapshot();
	if (!snap) {
		return json({ code: 'unavailable', message: 'stdlib snapshot failed to generate' }, { status: 503 });
	}
	return json(
		{ version: snap.version, modules: snap.modules },
		{ headers: { 'Cache-Control': 'no-store' } }
	);
}
