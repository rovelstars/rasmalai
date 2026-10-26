import { json } from '@sveltejs/kit';
import {
	parsePackageName,
	getVersionRow,
	getVersionIndexRow,
	getLatestVersion,
	getChunkBytes,
	getPackageOwner,
	getDependents,
	yankVersion,
	YANK_CAUSES,
	normalizeYankCause,
	UNPUBLISH_WINDOW_SECONDS,
	recordTombstone,
	unpublishVersion,
	specialDeleteAudit,
	specialDeletedVersions,
	mutationAlreadyApplied,
	checkPublishToken,
	purgeUrls,
	purgeConfigured,
	packagePointerUrls
} from '$lib/server/db';
import { safeEqual, sessionUser, readSessionCookie, userScopes, checkBrowserOrigin } from '$lib/server/auth';
import { specHeaders, splitNameVersion } from '$lib/server/registry';
import { sanitizeGuideHtml } from '$lib/server/sanitize';

function headers(extra: Record<string, string> = {}): Record<string, string> {
	return { ...specHeaders(), ...extra };
}

function notFound(message: string) {
	return json({ code: 'not-found', message }, { status: 404, headers: headers({ 'Cache-Control': 'public, max-age=600, s-maxage=600' }) });
}

function withdrawn(version: string, reason: string) {
	return json(
		{ code: 'withdrawn', message: `version ${version} withdrawn${reason ? `: ${reason}` : ''}`, reason },
		{ status: 410, headers: headers({ 'Cache-Control': 'public, max-age=31536000, immutable' }) }
	);
}

function parsePath(path: string): { full: string; rest: string[] } | null {
	const segs = path.split('/').filter(Boolean);
	if (segs.length === 0) return null;
	const scoped = segs[0].startsWith('@');
	if (scoped && segs.length < 2) return null;
	const scopeSeg = scoped ? segs[0] : '';
	const nameSeg = scoped ? segs[1] : segs[0];
	const tail = scoped ? segs.slice(2) : segs.slice(1);
	const nv = splitNameVersion(nameSeg);
	const full = scopeSeg ? `${scopeSeg}/${nv.name}` : nv.name;
	if (!parsePackageName(full)) return null;
	if (nv.version) {
		return { full, rest: [nv.version, ...tail] };
	}
	return { full, rest: tail };
}

function readWithdrawReason(manifestJson: string): string {
	try {
		return String((JSON.parse(manifestJson) as Record<string, unknown>)['withdrawReason'] ?? '');
	} catch {
		return '';
	}
}

interface GuideEntry {
	slug: string;
	title: string;
	html: string;
	source: string;
}

function readGuides(guidesJson: string): GuideEntry[] {
	try {
		const v = JSON.parse(guidesJson) as unknown;
		if (!Array.isArray(v)) return [];
		return (v as GuideEntry[]).filter((g) => typeof g?.slug === 'string');
	} catch {
		return [];
	}
}

export async function GET({ params, platform, setHeaders, url }) {
	const env = (platform?.env ?? {}) as Record<string, string | undefined>;
	const parsed = parsePath(params.path ?? '');
	if (!parsed) return notFound('invalid package path');
	const { full, rest } = parsed;

	if (rest.length === 0) {
		const latest = await getLatestVersion(env, full);
		if (!latest) return notFound(`package ${full} does not exist`);
		return new Response(null, {
			status: 302,
			headers: {
				Location: `/api/v0/packages/${full}/${latest}`,
				'Cache-Control': 'public, max-age=300, s-maxage=300',
				...specHeaders()
			}
		});
	}

	if (rest.length === 1 && rest[0] === 'dependents') {
		const parsedName = parsePackageName(full);
		if (!parsedName) return notFound('invalid package path');
		const owner = await getPackageOwner(env, parsedName.scope, parsedName.name);
		if (owner === null) return notFound(`package ${full} does not exist`);
		const names = await getDependents(env, full);
		setHeaders({ 'Cache-Control': 'public, max-age=60, s-maxage=3600', ...specHeaders() });
		return json({ dependents: names.map((name) => ({ name })) }, { headers: headers() });
	}

	const [version, sub, slug] = rest;
	// The doc payload is up to 1 MiB per version row, and only the api
	// sub-resource reads it. Every other read here takes the narrow
	// projection and never pays for the doc payload.
	const found =
		sub === 'api'
			? await getVersionRow(env, full, version)
			: await getVersionIndexRow(env, full, version);
	if (!found) {
		// A staff hard removal deletes the row but leaves the audit entry,
		// so the read answers the 410 variant instead of a plain 404. Only
		// a removed number qualifies: a never-published number still 404s.
		const purged = await specialDeletedVersions(env, full, [version]);
		if (purged.has(version)) return withdrawn(version, 'std-purge');
		return notFound(`package ${full}@${version} does not exist`);
	}
	if (found.withdrawn) {
		return withdrawn(version, readWithdrawReason(found.row.manifestJson));
	}
	const { row } = found;
	// Tooling reads transport payloads (manifests, chunks) under a fixed
	// `?origin-direct=1` namespace to escape stale shared-cache
	// generations; answer that namespace no-store so it can never cache
	// one. Normal reads keep the immutable edge TTL.
	const immutable = url.searchParams.has('origin-direct')
		? { 'Cache-Control': 'no-store' }
		: { 'Cache-Control': 'public, max-age=31536000, immutable' };

	if (!sub) {
		setHeaders({ ...immutable, ...specHeaders() });
		return json(
			{
				name: full,
				version: row.version,
				status: row.status,
				yankCause: row.status === 'yanked' && row.yankCause !== '' ? row.yankCause : null,
				checksum: row.checksum,
				tarballSha256: row.tarballSha256,
				artifactDigest: row.artifactDigest === '' ? null : row.artifactDigest,
				engineRange: row.engineRange,
				links: {
					api: `/api/v0/packages/${full}@${version}/api`,
					manifest: `/api/v0/packages/${full}@${version}/manifest`,
					chunks: `/api/v0/packages/${full}@${version}/chunks`,
					guides: `/api/v0/packages/${full}@${version}/guides`
				}
			},
			{ headers: headers() }
		);
	}

	if (sub === 'api') {
		setHeaders({ ...immutable, ...specHeaders() });
		const docJson =
			'docJson' in row && typeof row.docJson === 'string' ? row.docJson : '{"modules":[]}';
		try {
			return json(JSON.parse(docJson), { headers: headers() });
		} catch {
			return json({ modules: [] }, { headers: headers() });
		}
	}

	if (sub === 'manifest') {
		setHeaders({ ...immutable, ...specHeaders() });
		try {
			return json(JSON.parse(row.manifestJson), { headers: headers() });
		} catch {
			return json({}, { headers: headers() });
		}
	}

	if (sub === 'guides') {
		const guides = readGuides(row.guidesJson);
		if (guides.length === 0) {
			return json(
				{ code: 'not-found', message: `package ${full}@${version} ships no guides` },
				{ status: 404, headers: headers({ 'Cache-Control': 'public, max-age=600, s-maxage=600' }) }
			);
		}
		if (!slug) {
			setHeaders({ ...immutable, ...specHeaders() });
			return json({ guides: guides.map((g) => ({ slug: g.slug, title: g.title })) }, { headers: headers() });
		}
		const entry = guides.find((g) => g.slug === slug);
		if (!entry) return notFound(`guide ${slug} not found in ${full}@${version}`);
		const format = url.searchParams.get('format') ?? 'html';
		setHeaders({ ...immutable, ...specHeaders() });
		if (format === 'source') return json({ slug, format, source: sanitizeGuideHtml(entry.source) }, { headers: headers() });
		return json({ slug, format: 'html', html: entry.html }, { headers: headers() });
	}

	if (sub === 'chunks' && rest.length === 2) {
		let entries: unknown = null;
		try {
			entries = JSON.parse(row.tarManifestJson) as unknown;
		} catch {
			entries = null;
		}
		// A version that was published without a tarball has no file index
		// at all: the stored column is the empty array. Saying so is what
		// keeps the client from reporting a checksum mismatch for a package
		// that never shipped one.
		if (!Array.isArray(entries) || entries.length === 0) {
			return json(
				{ code: 'no-file-index', message: `package ${full}@${version} ships no file index` },
				{ status: 404, headers: headers({ 'Cache-Control': 'public, max-age=600, s-maxage=600' }) }
			);
		}
		setHeaders({ ...immutable, ...specHeaders() });
		// The body is served compressed to a client that asks for it, so the
		// cache has to key on the encoding it stored.
		return json({ entries }, { headers: headers({ Vary: 'accept-encoding' }) });
	}

	if (sub === 'chunk' && rest.length === 3) {
		// The client accepts a digest in either case (it lowercases before
		// comparing), so the route accepts the same set and normalizes the
		// digest before the lookup: an uppercase digest is a valid digest,
		// not a missing chunk.
		if (!slug || !/^[0-9a-fA-F]{64}$/.test(slug)) return notFound('invalid chunk hash');
		const hash = slug.toLowerCase();
		let bytes: Uint8Array | null;
		try {
			bytes = await getChunkBytes(env, hash);
		} catch (e) {
			const integrity = e instanceof Error && e.message.includes('failed integrity check');
			return json(
				integrity
					? { code: 'integrity-failed', message: `chunk ${hash} failed its integrity check` }
					: { code: 'chunk-read-failed', message: `chunk ${hash} is temporarily unreadable` },
				{ status: 500, headers: headers({ 'Cache-Control': 'no-store' }) }
			);
		}
		if (!bytes) return notFound(`chunk ${slug} not found`);
		return new Response(bytes as BodyInit, {
			headers: {
				'Content-Type': 'application/octet-stream',
				'Content-Length': String(bytes.length),
				...immutable,
				...specHeaders()
			}
		});
	}

	if ((sub === 'yank' || sub === 'takedown') && rest.length === 2) {
		return json({ code: 'method-not-allowed', message: 'use POST' }, { status: 405, headers: headers() });
	}
	return notFound('unknown sub-resource');
}

function adminToken(env: Record<string, string | undefined>, provided: string): boolean {
	const configured = env['ADMIN_TOKEN'];
	if (!configured) return false;
	return safeEqual(provided, configured);
}

// The idempotency key of an owner/admin mutation. The publish frame
// carries it as `requestId`; the other mutations accept it as a JSON
// body field or as the `X-RNX-Request-ID` header the manual documents.
async function readRequestId(request: Request): Promise<string> {
	const header = request.headers.get('X-RNX-Request-ID') ?? request.headers.get('x-rnx-request-id') ?? '';
	if (request.method !== 'POST') return header;
	try {
		const body = (await request.clone().json()) as Record<string, unknown>;
		if (typeof body['requestId'] === 'string') return body['requestId'];
	} catch {
		// no JSON body: the header alone decides
	}
	return header;
}

export async function POST({ params, request, platform, url }) {
	const env = (platform?.env ?? {}) as Record<string, string | undefined>;
	const parsed = parsePath(params.path ?? '');
	if (!parsed) return notFound('invalid package path');
	const { full, rest } = parsed;
	const [version, action] = rest;
	if (!version || (action !== 'yank' && action !== 'takedown' && action !== 'unpublish') || rest.length !== 2) {
		return notFound('unknown mutation');
	}
	const auth = request.headers.get('Authorization') ?? '';
	const token = auth.startsWith('Bearer ') ? auth.slice(7) : '';

	if (action === 'unpublish') {
		const tokenOk = checkPublishToken(env, token);
		let user: { id: number; username: string; isAdmin: boolean } | null = null;
		if (!tokenOk) {
			user = await sessionUser(env, readSessionCookie(request.headers.get('Cookie')));
			if (!user) {
				return json({ code: 'unauthorized', message: 'invalid publisher token' }, { status: 401, headers: headers() });
			}
			if (!checkBrowserOrigin(request, url)) {
				return json({ code: 'bad-origin', message: 'bad origin' }, { status: 403, headers: headers() });
			}
		}
		const found = await getVersionIndexRow(env, full, version);
		if (!tokenOk) {
			const name = parsePackageName(full);
			if (name && name.scope) {
				const scopes = await userScopes(env, user!.id);
				if (!scopes.includes(name.scope)) {
					return json({ code: 'forbidden', message: `scope @${name.scope} is not yours` }, { status: 403, headers: headers() });
				}
			} else {
				const owner = await getPackageOwner(env, name?.scope ?? '', name?.name ?? '');
				if (owner !== user!.username) {
					return json({ code: 'forbidden', message: `package ${full} is owned by someone else` }, { status: 403, headers: headers() });
				}
			}
		}
		const unpublishRequestId = await readRequestId(request);
		if (await mutationAlreadyApplied(env, 'unpublish', full, version, unpublishRequestId)) {
			return json({ success: true, version, status: 'unpublished', duplicate: true }, { headers: headers() });
		}
		if (!found) return notFound(`package ${full}@${version} does not exist`);
		if (found.withdrawn) return withdrawn(version, readWithdrawReason(found.row.manifestJson));
		const result = await unpublishVersion(env, full, version, unpublishRequestId);
		if (!result.ok && result.code === 'not-found') return notFound(`package ${full}@${version} does not exist`);
		if (!result.ok && result.code === 'withdrawn') return withdrawn(version, 'tombstoned');
		if (!result.ok && result.code === 'too-old') {
			return json(
				{ code: 'unpublish-window-closed', message: `version ${version} is past the 72-hour unpublish window` },
				{ status: 409, headers: headers() }
			);
		}
		const purge = await purgeUrls(env, packagePointerUrls(url.origin, full, version), {
			fullName: full,
			verify: { url: `${url.origin}/api/v0/packages/${full}@${version}`, status: 404 }
		});
		if (purgeConfigured(env) && !purge.verified) {
			return json(
				{ code: 'unavailable', message: 'the edge still serves the old copy, retry the purge' },
				{ status: 503, headers: headers() }
			);
		}
		return json({ success: true, version, status: 'unpublished' }, { headers: headers() });
	}

	if (action === 'yank') {
		const tokenOk = checkPublishToken(env, token);
		const isAdminToken = adminToken(env, token);
		let user: { id: number; username: string; isAdmin: boolean } | null = null;
		if (!tokenOk && !isAdminToken) {
			user = await sessionUser(env, readSessionCookie(request.headers.get('Cookie')));
			if (!user) {
				return json({ code: 'unauthorized', message: 'invalid publisher token' }, { status: 401, headers: headers() });
			}
			if (!checkBrowserOrigin(request, url)) {
				return json({ code: 'bad-origin', message: 'bad origin' }, { status: 403, headers: headers() });
			}
		}
		const isAdmin = isAdminToken || user?.isAdmin === true;
		const found = await getVersionIndexRow(env, full, version);
		if (!isAdmin && !tokenOk) {
			const name = parsePackageName(full);
			if (name && name.scope) {
				const scopes = await userScopes(env, user!.id);
				if (!scopes.includes(name.scope)) {
					return json({ code: 'forbidden', message: `scope @${name.scope} is not yours` }, { status: 403, headers: headers() });
				}
			} else {
				const owner = await getPackageOwner(env, name?.scope ?? '', name?.name ?? '');
				if (owner !== user!.username) {
					return json({ code: 'forbidden', message: `package ${full} is owned by someone else` }, { status: 403, headers: headers() });
				}
			}
		}
		let causeRaw: unknown = '';
		let yankRequestId = '';
		try {
			const body = (await request.clone().json()) as Record<string, unknown>;
			causeRaw = body['cause'] ?? body['reason'] ?? '';
			if (typeof body['requestId'] === 'string') yankRequestId = body['requestId'];
		} catch {
			causeRaw = '';
		}
		if (!yankRequestId) {
			yankRequestId = request.headers.get('X-RNX-Request-ID') ?? request.headers.get('x-rnx-request-id') ?? '';
		}
		if (await mutationAlreadyApplied(env, 'yank', full, version, yankRequestId)) {
			const stored = found && found.row.status === 'yanked' && found.row.yankCause !== '' ? found.row.yankCause : null;
			return json({ success: true, version, status: 'yanked', yankCause: stored, duplicate: true }, { headers: headers() });
		}
		if (!found) return notFound(`package ${full}@${version} does not exist`);
		if (found.withdrawn) return withdrawn(version, readWithdrawReason(found.row.manifestJson));
		if (isAdmin) {
			const cause = normalizeYankCause(causeRaw);
			if (!cause) {
				return json(
					{
						code: 'invalid-yank-cause',
						message: `admin yank needs a recorded security cause: one of ${YANK_CAUSES.join(' | ')}`
					},
					{ status: 400, headers: headers() }
				);
			}
			const result = await yankVersion(env, full, version, yankRequestId, { cause, allowLate: true });
			if (!result.ok && result.code === 'not-found') return notFound(`package ${full}@${version} does not exist`);
			if (!result.ok && result.code === 'withdrawn') return withdrawn(version, readWithdrawReason(found.row.manifestJson));
			if (!result.ok) {
				return json({ code: 'conflict', message: `version ${version} is not live` }, { status: 409, headers: headers() });
			}
			const purge = await purgeUrls(env, packagePointerUrls(url.origin, full, version), {
				fullName: full,
				verify: { url: `${url.origin}/api/v0/packages/${full}@${version}`, bodyStatus: 'yanked' }
			});
			if (purgeConfigured(env) && !purge.verified) {
				return json(
					{ code: 'unavailable', message: 'the edge still serves the old copy, retry the purge' },
					{ status: 503, headers: headers() }
				);
			}
			return json({ success: true, version, status: 'yanked', yankCause: result.yankCause }, { headers: headers() });
		}
		const result = await yankVersion(env, full, version, yankRequestId);
		if (!result.ok && result.code === 'not-found') return notFound(`package ${full}@${version} does not exist`);
		if (!result.ok && result.code === 'withdrawn') return withdrawn(version, readWithdrawReason(found.row.manifestJson));
		if (!result.ok && result.code === 'too-old') {
			const ageHours = Math.floor((Math.floor(Date.now() / 1000) - Number(result.createdAt ?? 0)) / 3600);
			const windowHours = UNPUBLISH_WINDOW_SECONDS / 3600;
			return json(
				{
					code: 'yank-window-closed',
					message: `version ${version} is ${ageHours} hours old; the owner yank window closed ${windowHours} hours after publish. Ask the registry admin to yank with a recorded security cause (${YANK_CAUSES.join(' | ')}).`
				},
				{ status: 409, headers: headers() }
			);
		}
		if (!result.ok) {
			return json({ code: 'conflict', message: `version ${version} is not live` }, { status: 409, headers: headers() });
		}
		const purge = await purgeUrls(env, packagePointerUrls(url.origin, full, version), {
			fullName: full,
			verify: { url: `${url.origin}/api/v0/packages/${full}@${version}`, bodyStatus: 'yanked' }
		});
		if (purgeConfigured(env) && !purge.verified) {
			// The yank is committed, but the edge still serves the copy that
			// says the version is live. The mutation is not reported as
			// complete until the purge is verified.
			return json(
				{ code: 'unavailable', message: 'the edge still serves the old copy, retry the purge' },
				{ status: 503, headers: headers() }
			);
		}
		return json({ success: true, version, status: 'yanked', yankCause: null }, { headers: headers() });
	}

	if (!adminToken(env, token)) {
		return json({ code: 'unauthorized', message: 'invalid admin token' }, { status: 401, headers: headers() });
	}
	const found = await getVersionIndexRow(env, full, version);
	let reason = '';
	let takedownRequestId = '';
	try {
		const body = (await request.clone().json()) as Record<string, unknown>;
		reason = typeof body['reason'] === 'string' ? body['reason'] : '';
		if (typeof body['requestId'] === 'string') takedownRequestId = body['requestId'];
	} catch {
		reason = '';
	}
	if (!takedownRequestId) {
		takedownRequestId = request.headers.get('X-RNX-Request-ID') ?? request.headers.get('x-rnx-request-id') ?? '';
	}
	if (await mutationAlreadyApplied(env, 'takedown', full, version, takedownRequestId)) {
		return json({ success: true, version, status: 'tombstoned', duplicate: true }, { headers: headers() });
	}
	if (!found) return notFound(`package ${full}@${version} does not exist`);
	if (found.withdrawn) return withdrawn(version, readWithdrawReason(found.row.manifestJson));
	await recordTombstone(env, full, version, reason, takedownRequestId);
	const purge = await purgeUrls(env, packagePointerUrls(url.origin, full, version), {
		fullName: full,
		prefixes: [`${url.origin}/api/v0/packages/${full}@${version}`],
		// A withdrawn version has to answer 410 through the edge, or the
		// edge is still serving the copy that says it is live.
		verify: { url: `${url.origin}/api/v0/packages/${full}@${version}`, status: 410 }
	});
	if (purgeConfigured(env) && !purge.verified) {
		// The takedown is committed, but the edge still serves the copy
		// that says the version is live. The mutation is not reported as
		// complete until the purge is verified.
		return json(
			{ code: 'unavailable', message: 'the edge still serves the old copy, retry the purge' },
			{ status: 503, headers: headers() }
		);
	}
	return json({ success: true, version, status: 'tombstoned' }, { headers: headers() });
}
