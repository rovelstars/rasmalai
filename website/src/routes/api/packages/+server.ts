import { json } from '@sveltejs/kit';
import {
	listPackages,
	publishPackage,
	checkPublishToken,
	parsePackageName,
	recentVersionCount,
	MAX_DOC_JSON_BYTES,
	MAX_README_BYTES,
	MAX_VERSIONS_PER_WEEK,
	type PublishPayload
} from '$lib/server/db';

const VERSION_RE = /^\d+\.\d+\.\d+$/;

function unauthorized(message: string) {
	return json({ success: false, error: message }, { status: 401 });
}

// Public catalog for search and listings. Metadata only, never doc payloads.
// Short edge cache: the index moves on every publish.
export async function GET({ platform, setHeaders }) {
	const env = (platform?.env ?? {}) as Record<string, string | undefined>;
	const packages = await listPackages(env);
	setHeaders({ 'Cache-Control': 'public, max-age=60, s-maxage=60' });
	return json({
		packages: packages.map((p) => ({
			name: p.name,
			description: p.description,
			latest: p.latest,
			versionCount: p.versionCount,
			updatedAt: p.updatedAt
		}))
	});
}

export async function POST({ request, platform }) {
	const env = (platform?.env ?? {}) as Record<string, string | undefined>;
	const auth = request.headers.get('Authorization') ?? '';
	const token = auth.startsWith('Bearer ') ? auth.slice(7) : '';
	if (!checkPublishToken(env, token)) {
		return unauthorized('invalid publisher token');
	}

	const rawName = request.headers.get('X-RNX-Package-Name') ?? '';
	const version = request.headers.get('X-RNX-Package-Version') ?? '';
	const checksum = request.headers.get('X-RNX-Checksum') ?? '';
	const parsed = parsePackageName(rawName);
	if (!parsed) {
		return json(
			{ success: false, error: 'invalid package name (expected name or @scope/name)' },
			{ status: 400 }
		);
	}
	if (!VERSION_RE.test(version)) {
		return json(
			{ success: false, error: 'invalid package version (expected X.Y.Z)' },
			{ status: 400 }
		);
	}
	if (!/^[0-9a-f]{64}$/.test(checksum)) {
		return json({ success: false, error: 'invalid SHA-256 checksum' }, { status: 400 });
	}

	let meta: Record<string, unknown> = {};
	const contentType = request.headers.get('Content-Type') ?? '';
	if (contentType.includes('application/json')) {
		try {
			meta = (await request.json()) as Record<string, unknown>;
		} catch {
			return json({ success: false, error: 'invalid JSON metadata' }, { status: 400 });
		}
	} else {
		await request.arrayBuffer().catch(() => new ArrayBuffer(0));
	}

	const str = (v: unknown, fallback = ''): string =>
		typeof v === 'string' ? v : fallback;
	const readme = str(meta['readme'], `# ${parsed.full}\n`);
	const docJson = str(meta['docJson'], '{"modules":[]}');
	if (readme.length > MAX_README_BYTES || docJson.length > MAX_DOC_JSON_BYTES) {
		return json({ success: false, error: 'payload exceeds size limits' }, { status: 413 });
	}
	const recent = await recentVersionCount(env, parsed.full, 7 * 24 * 3600);
	if (recent >= MAX_VERSIONS_PER_WEEK) {
		return json(
			{ success: false, error: `version rate limit exceeded (${MAX_VERSIONS_PER_WEEK}/week)` },
			{ status: 429 }
		);
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
		checksum
	};

	try {
		const result = await publishPackage(env, payload);
		if (!result.created) {
			return json(
				{ success: false, error: `version ${version} already published` },
				{ status: 409 }
			);
		}
		return json({ success: true, url: `/packages/${parsed.full}`, version }, { status: 201 });
	} catch (e) {
		return json(
			{ success: false, error: e instanceof Error ? e.message : 'ingest failed' },
			{ status: 500 }
		);
	}
}
