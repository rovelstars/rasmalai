import { json } from '@sveltejs/kit';
import {
	listPackages,
	publishPackage,
	checkPublishToken,
	parsePackageName,
	packageId,
	getVersionRow,
	versionReuseBlocked,
	recentVersionCount,
	putChunk,
	linkVersionChunks,
	MAX_DOC_JSON_BYTES,
	MAX_README_BYTES,
	MAX_VERSIONS_PER_WEEK,
	MAX_TARBALL_BYTES,
	type PublishPayload
} from '$lib/server/db';
import { specHeaders } from '$lib/server/registry';
import { chunkTarball, sha256Hex } from '$lib/server/chunks';
import { sessionUser, readSessionCookie, userScopes } from '$lib/server/auth';

const VERSION_RE = /^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$/;
const MAX_GUIDES_BYTES = 400 * 1024;
const MAX_SNAPSHOT_BYTES = 512 * 1024;

function unauthorized(message: string) {
	return json({ success: false, error: message }, { status: 401, headers: specHeaders() });
}

// Public catalog for search and listings. Metadata only, never doc payloads.
// Short edge cache: the index moves on every publish.
export async function GET({ platform, setHeaders }) {
	const env = (platform?.env ?? {}) as Record<string, string | undefined>;
	const packages = await listPackages(env);
	setHeaders({ 'Cache-Control': 'public, max-age=60, s-maxage=60', ...specHeaders() });
	return json(
		{
			packages: packages.map((p) => ({
				name: p.name,
				description: p.description,
				latest: p.latest,
				versionCount: p.versionCount,
				updatedAt: p.updatedAt
			}))
		},
		{ headers: specHeaders() }
	);
}

function badScript(s: string): boolean {
	return /<script[\s>]/i.test(s);
}

export async function POST({ request, platform }) {
	const env = (platform?.env ?? {}) as Record<string, string | undefined>;
	const auth = request.headers.get('Authorization') ?? '';
	const token = auth.startsWith('Bearer ') ? auth.slice(7) : '';
	const orgTokenOk = checkPublishToken(env, token);
	if (!orgTokenOk) {
		const user = await sessionUser(env, readSessionCookie(request.headers.get('Cookie')));
		if (!user) {
			return unauthorized('invalid publisher token');
		}
		const scopes = await userScopes(env, user.id);
		const rawNameEarly = request.headers.get('X-RNX-Package-Name') ?? '';
		const parsedEarly = parsePackageName(rawNameEarly);
		if (!parsedEarly) {
			return json(
				{ success: false, error: 'invalid package name (expected name or @scope/name)' },
				{ status: 400, headers: specHeaders() }
			);
		}
		if (parsedEarly.scope && !scopes.includes(parsedEarly.scope)) {
			return json(
				{ success: false, error: `scope @${parsedEarly.scope} is not yours` },
				{ status: 403, headers: specHeaders() }
			);
		}
	}

	const rawName = request.headers.get('X-RNX-Package-Name') ?? '';
	const version = request.headers.get('X-RNX-Package-Version') ?? '';
	const checksum = request.headers.get('X-RNX-Checksum') ?? '';
	const requestId = request.headers.get('X-RNX-Request-ID') ?? '';
	const parsed = parsePackageName(rawName);
	if (!parsed) {
		return json(
			{ success: false, error: 'invalid package name (expected name or @scope/name)' },
			{ status: 400, headers: specHeaders() }
		);
	}
	if (!VERSION_RE.test(version)) {
		return json(
			{ success: false, error: 'invalid package version (expected X.Y.Z)' },
			{ status: 400, headers: specHeaders() }
		);
	}
	if (checksum && !/^[0-9a-f]{64}$/.test(checksum)) {
		return json({ success: false, error: 'invalid SHA-256 checksum' }, { status: 400, headers: specHeaders() });
	}

	let meta: Record<string, unknown> = {};
	let tarball: Uint8Array | null = null;
	const contentType = request.headers.get('Content-Type') ?? '';
	if (contentType.includes('application/json')) {
		try {
			meta = (await request.json()) as Record<string, unknown>;
		} catch {
			return json({ success: false, error: 'invalid JSON metadata' }, { status: 400, headers: specHeaders() });
		}
	} else {
		const buf = await request.arrayBuffer().catch(() => new ArrayBuffer(0));
		if (buf.byteLength > 0) {
			if (buf.byteLength > MAX_TARBALL_BYTES) {
				return json({ success: false, error: 'tarball exceeds 512 KiB' }, { status: 413, headers: specHeaders() });
			}
			tarball = new Uint8Array(buf);
		}
		try {
			meta = JSON.parse(request.headers.get('X-RNX-Meta') ?? '{}') as Record<string, unknown>;
		} catch {
			meta = {};
		}
	}

	const str = (v: unknown, fallback = ''): string => (typeof v === 'string' ? v : fallback);
	const readme = str(meta['readme'], `# ${parsed.full}\n`);
	const docJson = str(meta['docJson'], '{"modules":[]}');
	const engineRange = str(meta['engineRange'], '');
	const guides = Array.isArray(meta['guides']) ? (meta['guides'] as unknown[]) : [];
	for (const g of guides) {
		if (!g || typeof g !== 'object') {
			return json({ success: false, error: 'invalid guide entry' }, { status: 400, headers: specHeaders() });
		}
		const e = g as Record<string, unknown>;
		if (typeof e['slug'] !== 'string' || typeof e['title'] !== 'string') {
			return json({ success: false, error: 'guide needs slug and title' }, { status: 400, headers: specHeaders() });
		}
		const html = typeof e['html'] === 'string' ? e['html'] : '';
		const source = typeof e['source'] === 'string' ? e['source'] : '';
		if (badScript(html) || badScript(source)) {
			return json({ success: false, error: 'guide content must be pre-sanitized (no script tags)' }, { status: 400, headers: specHeaders() });
		}
	}
	const guidesJson = JSON.stringify(guides);
	let manifestJson = '{"deps":{}}';
	if (meta['manifest'] && typeof meta['manifest'] === 'object') {
		manifestJson = JSON.stringify(meta['manifest']);
	}
	const storedBytes = readme.length + docJson.length + guidesJson.length + manifestJson.length;
	if (readme.length > MAX_README_BYTES || docJson.length > MAX_DOC_JSON_BYTES) {
		return json({ success: false, error: 'payload exceeds size limits' }, { status: 413, headers: specHeaders() });
	}
	if (guidesJson.length > MAX_GUIDES_BYTES || storedBytes > MAX_SNAPSHOT_BYTES) {
		return json({ success: false, error: 'stored snapshot exceeds 512 KiB cap' }, { status: 413, headers: specHeaders() });
	}

	if (await versionReuseBlocked(env, parsed.full, version)) {
		const existing = await getVersionRow(env, parsed.full, version);
		if (existing && requestId && existing.row.requestId === requestId) {
			return json({ success: true, url: `/packages/${parsed.full}`, version, duplicate: true }, { headers: specHeaders() });
		}
		return json(
			{ success: false, error: `version ${version} already published (versions are immutable)` },
			{ status: 409, headers: specHeaders() }
		);
	}
	const recent = await recentVersionCount(env, parsed.full, 7 * 24 * 3600);
	if (recent >= MAX_VERSIONS_PER_WEEK) {
		return json(
			{ success: false, error: `version rate limit exceeded (${MAX_VERSIONS_PER_WEEK}/week)` },
			{ status: 429, headers: specHeaders() }
		);
	}

	let tarballSha256 = typeof meta['tarballSha256'] === 'string' ? (meta['tarballSha256'] as string) : '';
	if (tarball) {
		tarballSha256 = await sha256Hex(tarball);
		if (checksum && checksum !== tarballSha256) {
			return json({ success: false, error: 'checksum does not match tarball bytes' }, { status: 400, headers: specHeaders() });
		}
	}
	const payload: PublishPayload = {
		name: parsed.full,
		version,
		description: str(meta['description'], `${parsed.full} package`),
		author: str(meta['author'], 'anonymous'),
		license: str(meta['license'], 'MIT'),
		tags: Array.isArray(meta['tags'])
			? (meta['tags'] as unknown[]).filter((t): t is string => typeof t === 'string')
			: [],
		readme,
		docJson,
		checksum: tarballSha256 || checksum,
		engineRange,
		manifestJson,
		guidesJson,
		tarballSha256,
		requestId
	};

	try {
		const result = await publishPackage(env, payload);
		if (!result.created) {
			return json(
				{ success: false, error: `version ${version} already published` },
				{ status: 409, headers: specHeaders() }
			);
		}
		if (tarball) {
			const units = await chunkTarball(tarball);
			for (const u of units) await putChunk(env, u.hash, u.size, u.bytes);
			await linkVersionChunks(env, `ver_${packageId(parsed.scope, parsed.name)}_${version}`, units.map((u) => u.hash));
		}
		return json({ success: true, url: `/packages/${parsed.full}`, version }, { status: 201, headers: specHeaders() });
	} catch (e) {
		return json(
			{ success: false, error: e instanceof Error ? e.message : 'ingest failed' },
			{ status: 500, headers: specHeaders() }
		);
	}
}
