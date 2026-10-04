import { json } from '@sveltejs/kit';
import {
	parsePackageName,
	getVersionRow,
	getLatestVersion,
	getVersionBytes,
	yankVersion,
	recordTombstone,
	checkPublishToken
} from '$lib/server/db';
import { specHeaders } from '$lib/server/registry';

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
	if (segs[0].startsWith('@')) {
		if (segs.length < 2) return null;
		const full = `${segs[0]}/${segs[1]}`;
		if (!parsePackageName(full)) return null;
		return { full, rest: segs.slice(2) };
	}
	if (!parsePackageName(segs[0])) return null;
	return { full: segs[0], rest: segs.slice(1) };
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
		setHeaders({ 'Cache-Control': 'public, max-age=300, s-maxage=300', ...specHeaders() });
		return json({ name: full, latest, url: `/api/packages/${full}/${latest}` }, { status: 302, headers: headers() });
	}

	const [version, sub, slug] = rest;
	const found = await getVersionRow(env, full, version);
	if (!found) return notFound(`package ${full}@${version} does not exist`);
	if (found.withdrawn) {
		let reason = '';
		try {
			reason = String((JSON.parse(found.row.manifestJson) as Record<string, unknown>)['withdrawReason'] ?? '');
		} catch {
			reason = '';
		}
		return withdrawn(version, reason);
	}
	const { row } = found;
	const immutable = { 'Cache-Control': 'public, max-age=31536000, immutable' };

	if (!sub) {
		setHeaders({ ...immutable, ...specHeaders() });
		return json(
			{
				name: full,
				version: row.version,
				status: row.status,
				checksum: row.checksum,
				tarballSha256: row.tarballSha256,
				engineRange: row.engineRange,
				links: {
					download: `/api/packages/${full}/${version}/download`,
					api: `/api/packages/${full}/${version}/api`,
					guides: `/api/packages/${full}/${version}/guides`
				}
			},
			{ headers: headers() }
		);
	}

	if (sub === 'download') {
		const bytes = await getVersionBytes(env, row.id);
		if (!bytes) return notFound(`no stored content for ${full}@${version}`);
		return new Response(bytes as BodyInit, {
			headers: {
				'Content-Type': 'application/octet-stream',
				'Content-Length': String(bytes.length),
				...immutable,
				...specHeaders()
			}
		});
	}

	if (sub === 'api') {
		setHeaders({ ...immutable, ...specHeaders() });
		try {
			return json(JSON.parse(row.manifestJson), { headers: headers() });
		} catch {
			return json({}, { headers: headers() });
		}
	}

	if (sub === 'guides') {
		const guides = readGuides(row.guidesJson);
		if (!slug) {
			setHeaders({ ...immutable, ...specHeaders() });
			return json({ guides: guides.map((g) => ({ slug: g.slug, title: g.title })) }, { headers: headers() });
		}
		const entry = guides.find((g) => g.slug === slug);
		if (!entry) return notFound(`guide ${slug} not found in ${full}@${version}`);
		const format = url.searchParams.get('format') ?? 'html';
		setHeaders({ ...immutable, ...specHeaders() });
		if (format === 'source') return json({ slug, format, source: entry.source }, { headers: headers() });
		return json({ slug, format: 'html', html: entry.html }, { headers: headers() });
	}

	if ((sub === 'yank' || sub === 'takedown') && rest.length === 2) {
		return json({ code: 'method-not-allowed', message: 'use POST' }, { status: 405, headers: headers() });
	}
	return notFound('unknown sub-resource');
}

function adminToken(env: Record<string, string | undefined>, provided: string): boolean {
	const configured = env['ADMIN_TOKEN'];
	if (!configured) return false;
	return provided === configured && provided.length > 0;
}

export async function POST({ params, request, platform }) {
	const env = (platform?.env ?? {}) as Record<string, string | undefined>;
	const parsed = parsePath(params.path ?? '');
	if (!parsed) return notFound('invalid package path');
	const { full, rest } = parsed;
	const [version, action] = rest;
	if (!version || (action !== 'yank' && action !== 'takedown') || rest.length !== 2) {
		return notFound('unknown mutation');
	}
	const auth = request.headers.get('Authorization') ?? '';
	const token = auth.startsWith('Bearer ') ? auth.slice(7) : '';

	if (action === 'yank') {
		if (!checkPublishToken(env, token)) {
			return json({ code: 'unauthorized', message: 'invalid publisher token' }, { status: 401, headers: headers() });
		}
		const found = await getVersionRow(env, full, version);
		if (!found) return notFound(`package ${full}@${version} does not exist`);
		if (found.withdrawn) return withdrawn(version, '');
		const ok = await yankVersion(env, full, version);
		if (!ok) {
			return json({ code: 'conflict', message: `version ${version} is not live` }, { status: 409, headers: headers() });
		}
		return json({ success: true, version, status: 'yanked' }, { headers: headers() });
	}

	if (!adminToken(env, token)) {
		return json({ code: 'unauthorized', message: 'invalid admin token' }, { status: 401, headers: headers() });
	}
	const found = await getVersionRow(env, full, version);
	if (!found) return notFound(`package ${full}@${version} does not exist`);
	if (found.withdrawn) return withdrawn(version, '');
	let reason = '';
	try {
		const body = (await request.json()) as Record<string, unknown>;
		reason = typeof body['reason'] === 'string' ? body['reason'] : '';
	} catch {
		reason = '';
	}
	await recordTombstone(env, full, version, reason);
	return json({ success: true, version, status: 'tombstoned' }, { headers: headers() });
}
