import { json } from '@sveltejs/kit';
import { resolveGraph, type HaveEntry } from '$lib/server/db';
import {
	specHeaders,
	isValidRange,
	parseSemver,
	isTransientStoreError,
	MAX_RESOLVE_BODY_BYTES,
	MAX_RESOLVE_REQUIREMENTS,
	MAX_RESOLVE_HAVE
} from '$lib/server/registry';
import { clientIp } from '$lib/server/auth';
import {
	checkRateLimit,
	RESOLVE_RATE_LIMIT,
	RESOLVE_RATE_WINDOW_MS
} from '$lib/server/rateLimit';
import { newRequestId } from '$lib/server/requestId';

function err(code: string, message: string, status: number, requestId: string) {
	return json({ code, message, requestId }, { status, headers: specHeaders() });
}

// The body, read with a hard byte ceiling. The declared length is checked
// first, so an oversized upload is refused without being read at all, and
// the stream is cut off at the ceiling for the bodies that arrive without
// a length.
async function readBody(
	request: Request,
	maxBytes: number
): Promise<{ ok: true; text: string } | { ok: false }> {
	const declared = Number(request.headers.get('Content-Length') ?? '');
	if (Number.isFinite(declared) && declared > maxBytes) return { ok: false };
	const body = request.body;
	if (!body) {
		const text = await request.text();
		if (new TextEncoder().encode(text).length > maxBytes) return { ok: false };
		return { ok: true, text };
	}
	const reader = body.getReader();
	const chunks: Uint8Array[] = [];
	let total = 0;
	for (;;) {
		const { done, value } = await reader.read();
		if (done) break;
		total += value.byteLength;
		if (total > maxBytes) {
			await reader.cancel().catch(() => undefined);
			return { ok: false };
		}
		chunks.push(value);
	}
	const merged = new Uint8Array(total);
	let off = 0;
	for (const chunk of chunks) {
		merged.set(chunk, off);
		off += chunk.byteLength;
	}
	return { ok: true, text: new TextDecoder().decode(merged) };
}

// One round trip, depth-leveled, with a have-list for deltas. Pinned
// requirement sets resolve deterministically; ranged sets re-resolve live.
export async function POST({ request, url, platform }) {
	const requestId = newRequestId();
	// The resolve endpoint is anonymous and one request can spend up to 50
	// store round trips, so a single client looping on it burns the shared
	// request budget. The check runs before the body is read: a refused
	// request costs a header read and nothing else. Requests with no
	// attributable address (local dev, tests) are not bucketed.
	const ip = clientIp(request.headers);
	if (ip !== 'unknown') {
		const verdict = checkRateLimit(`resolve:${ip}`, RESOLVE_RATE_LIMIT, RESOLVE_RATE_WINDOW_MS);
		if (!verdict.ok) {
			return json(
				{
					code: 'rate-limited',
					message: `resolve rate limit exceeded (${RESOLVE_RATE_LIMIT}/minute per client)`,
					requestId
				},
				{
					status: 429,
					headers: { ...specHeaders(), 'Retry-After': String(verdict.retryAfterSec) }
				}
			);
		}
	}
	const raw = await readBody(request, MAX_RESOLVE_BODY_BYTES);
	if (!raw.ok) {
		return err('payload-too-large', `request body over ${MAX_RESOLVE_BODY_BYTES} bytes`, 413, requestId);
	}
	let parsed: unknown;
	try {
		parsed = JSON.parse(raw.text);
	} catch {
		return err('bad-request', 'invalid JSON body', 400, requestId);
	}
	if (parsed === null) {
		return err('bad-request', 'invalid JSON body', 400, requestId);
	}
	const body = parsed as Record<string, unknown>;
	const reqs = body['requirements'];
	if (!reqs || typeof reqs !== 'object' || Object.keys(reqs).length === 0) {
		return err('bad-request', 'missing requirements map', 400, requestId);
	}
	const requirements = reqs as Record<string, string>;
	// The count is checked before every name is parsed: validating 100k
	// names is the cost the cap exists to avoid.
	if (Object.keys(requirements).length > MAX_RESOLVE_REQUIREMENTS) {
		return err(
			'payload-too-large',
			`requirements map over ${MAX_RESOLVE_REQUIREMENTS} entries`,
			413,
			requestId
		);
	}
	for (const [name, range] of Object.entries(requirements)) {
		if (typeof range !== 'string') {
			return err('bad-request', `requirement for ${name} must be a version range string`, 400, requestId);
		}
		if (!isValidRange(range)) {
			return err('bad-range', `requirement for ${name} is not a valid range (*, latest, X.Y.Z, ^, ~, >=)`, 400, requestId);
		}
	}
	const rawHave = body['have'];
	if (Array.isArray(rawHave) && rawHave.length > MAX_RESOLVE_HAVE) {
		return err('payload-too-large', `have list over ${MAX_RESOLVE_HAVE} entries`, 413, requestId);
	}
	const have: HaveEntry[] = Array.isArray(rawHave)
		? rawHave.flatMap((h) => {
				if (h && typeof h === 'object') {
					const o = h as Record<string, unknown>;
					if (typeof o['full'] === 'string' && typeof o['version'] === 'string') {
						return [{ full: o['full'], version: o['version'], integrity: String(o['integrity'] ?? '') }];
					}
				}
				return [];
			})
		: [];
	const env = (platform?.env ?? {}) as Record<string, string | undefined>;
	// `chunks` in the request body is the opt-in for inline chunk lists. A
	// client that never learned the capability sends nothing and gets the
	// byte-for-byte response it got before, entries and `chunks` switch
	// included absent.
	const wantChunks = body['chunks'];
	const inlineChunks = wantChunks === true;
	// `engine` is the toolchain version the client resolved with, as an
	// exact X.Y.Z. The server filters every candidate list to the versions
	// that engine accepts; a client that never sends one gets the
	// unfiltered answer it always got.
	const rawEngine = body['engine'];
	let engine: string | undefined;
	if (rawEngine !== undefined) {
		if (typeof rawEngine !== 'string' || parseSemver(rawEngine.trim()) === null) {
			return err('bad-request', 'invalid engine version (expected X.Y.Z)', 400, requestId);
		}
		engine = rawEngine.trim();
	}
	try {
		const { resolved, levels, chunks, chunksDeclined } = await resolveGraph(env, requirements, have, {
			inlineChunks,
			advertiseChunks: wantChunks !== undefined,
			engine
		});
		// base omitted: clients default to the request origin + /api/v0/packages.
		// A future multi-registry response may send an absolute URL (other
		// host) or a path (same host, other prefix) instead.
		const payload: Record<string, unknown> = { resolved, base: '', levels };
		if (chunks) payload['chunks'] = chunks;
		// An inline answer that was refused is named, so the client can fall
		// back to one manifest read per package instead of reading the body
		// switch and wondering why the file lists are missing.
		if (chunks && chunksDeclined) payload['chunksDeclined'] = chunksDeclined;
		return json(payload, { headers: specHeaders() });
	} catch (e) {
		const msg = e instanceof Error ? e.message : String(e);
		console.error(`[resolve ${requestId}] ${msg}`);
		if (msg.startsWith('resolve budget exceeded')) return err('budget-exceeded', msg, 422, requestId);
		if (msg.startsWith('dependency cycle')) return err('cycle', msg, 422, requestId);
		if (msg.includes('resolve raced')) {
			return err('unavailable', 'resolution raced with a mutation, retry', 503, requestId);
		}
		if (msg.includes('withdrawn (std-purge)')) {
			return err('withdrawn', msg, 410, requestId);
		}
		if (msg.includes('does not exist')) return err('not-found', msg, 404, requestId);
		if (msg.includes('satisfies') || msg.includes('conflicting')) {
			return err('unsatisfiable', msg, 422, requestId);
		}
		if (msg.includes('invalid package name')) return err('bad-request', msg, 400, requestId);
		if (msg.includes('invalid engine version')) return err('bad-request', msg, 400, requestId);
		if (isTransientStoreError(msg)) {
			return err('unavailable', 'resolution store unavailable, retry', 503, requestId);
		}
		return err('resolve-failed', 'resolution failed', 500, requestId);
	}
}
