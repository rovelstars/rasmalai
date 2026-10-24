import { json } from '@sveltejs/kit';
import {
	listPackages,
	publishCanonicalVersion,
	checkPublishToken,
	parsePackageName,
	getPackageOwner,
	getVersionIndexRow,
	getLatestVersion,
	getPackageEdges,
	versionReuseBlocked,
	recentVersionCount,
	existingChunkHashes,
	r2PutChunks,
	checkStorageQuota,
	chargeStorageQuota,
	purgeUrls,
	packagePointerUrls,
	validateKeywords,
	extractDepNames,
	MAX_DOC_JSON_BYTES,
	MAX_README_BYTES,
	MAX_VERSIONS_PER_WEEK,
	type CanonicalPublishPayload,
	type CanonicalFact,
	type PackageFilter
} from '$lib/server/db';
import { specHeaders } from '$lib/server/registry';
import { normalizeGuideEntry, normalizeDocJson } from '$lib/server/guides';
import {
	parseManifestPermissions,
	checkCapabilityCoverage,
	scanCanonicalCapabilities,
	type CapabilityHead,
	type DeclaredPermission
} from '$lib/server/permissions';
import {
	chunkCanonicalFiles,
	chunkCountBound,
	manifestDigest,
	sha256Hex,
	type CanonicalFile
} from '$lib/server/chunks';
import {
	PROTOCOL_VERSION,
	PROTOCOL_HEADER,
	planCaps,
	parsePublishFrame,
	fileAllowed
} from '$lib/server/canonical';
import { canonicalizeTree, configFacts, analyzeTree } from '$lib/server/analysis';
import { sessionUser, readSessionCookie, userScopes, checkBrowserOrigin } from '$lib/server/auth';

const MAX_GUIDES_BYTES = 400 * 1024;
const MAX_SNAPSHOT_BYTES = 512 * 1024;

// The request body ceiling. The declared length is checked first against
// the plan-shaped ceiling (source bytes plus metadata headroom): an
// oversized publish is refused without being read.
const BODY_HEADROOM_BYTES = 2 * 1024 * 1024;

function unauthorized(message: string) {
	return json({ success: false, error: message }, { status: 401, headers: specHeaders() });
}

async function readBody(
	request: Request,
	maxBytes: number
): Promise<{ ok: true; bytes: Uint8Array } | { ok: false }> {
	const declared = Number(request.headers.get('Content-Length') ?? '');
	if (Number.isFinite(declared) && declared > maxBytes) return { ok: false };
	const body = request.body;
	if (!body) {
		const buf = await request.arrayBuffer().catch(() => new ArrayBuffer(0));
		if (buf.byteLength > maxBytes) return { ok: false };
		return { ok: true, bytes: new Uint8Array(buf) };
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
	return { ok: true, bytes: merged };
}

// Public catalog for search and listings. Metadata only, never doc payloads.
export async function GET({ platform, setHeaders, url }) {
	const env = (platform?.env ?? {}) as Record<string, string | undefined>;
	const num = (v: string | null): number | undefined => {
		if (v === null || v.trim() === '') return undefined;
		const n = Number(v);
		return Number.isFinite(n) && n >= 0 ? n : undefined;
	};
	const filters: PackageFilter = {};
	const search = url.searchParams.get('search');
	const keyword = url.searchParams.get('keyword');
	const license = url.searchParams.get('license');
	if (search && search.trim() !== '') filters.search = search;
	if (keyword && keyword.trim() !== '') filters.keyword = keyword;
	if (license && license !== '') filters.license = license;
	const minDownloads = num(url.searchParams.get('minDownloads'));
	if (minDownloads !== undefined) filters.minDownloads = Math.floor(minDownloads);
	const updatedSince = num(url.searchParams.get('updatedSince'));
	if (updatedSince !== undefined) filters.updatedSinceDays = updatedSince;
	let packages: Awaited<ReturnType<typeof listPackages>> = [];
	try {
		packages = await listPackages(env, filters);
	} catch {
		packages = [];
	}
	setHeaders({ 'Cache-Control': 'public, max-age=3600, s-maxage=3600', ...specHeaders() });
	return json(
		{
			packages: packages.map((p) => ({
				name: p.name,
				description: p.description,
				latest: p.latest,
				versionCount: p.versionCount,
				updatedAt: p.updatedAt,
				keywords: p.keywords,
				downloads: p.downloads,
				dependents: p.dependents
			}))
		},
		{ headers: specHeaders() }
	);
}

function frameError(status: number, message: string) {
	return json({ success: false, error: message }, { status, headers: specHeaders() });
}

export async function POST({ request, platform, url }) {
	const env = (platform?.env ?? {}) as Record<string, string | undefined>;
	const caps = planCaps(env);
	const protocol = request.headers.get(PROTOCOL_HEADER) ?? request.headers.get(PROTOCOL_HEADER.toLowerCase()) ?? '';
	if (protocol !== PROTOCOL_VERSION) {
		return frameError(400, `unsupported publish protocol (expected \`${PROTOCOL_HEADER}: ${PROTOCOL_VERSION}\`)`);
	}
	const auth = request.headers.get('Authorization') ?? '';
	const token = auth.startsWith('Bearer ') ? auth.slice(7) : '';
	const orgTokenOk = checkPublishToken(env, token);
	let actorOwner = 'org';
	let actorId = -1;
	if (!orgTokenOk) {
		const user = await sessionUser(env, readSessionCookie(request.headers.get('Cookie')));
		if (!user) {
			return unauthorized('invalid publisher token');
		}
		if (!checkBrowserOrigin(request, url)) {
			return json({ success: false, error: 'bad origin', code: 'bad-origin' }, { status: 403, headers: specHeaders() });
		}
		actorOwner = user.username;
		actorId = user.id;
	}

	const declared = Number(request.headers.get('Content-Length') ?? '');
	if (Number.isFinite(declared) && declared > caps.sourceBytes + caps.manifestBytes + BODY_HEADROOM_BYTES) {
		return frameError(413, 'publish body exceeds the plan limit');
	}
	const body = await readBody(request, caps.sourceBytes + caps.manifestBytes + BODY_HEADROOM_BYTES);
	if (!body.ok) {
		return frameError(413, 'publish body exceeds the plan limit');
	}
	let raw: unknown;
	try {
		raw = JSON.parse(new TextDecoder().decode(body.bytes));
	} catch {
		return frameError(400, 'invalid JSON publish body');
	}
	const parsed = parsePublishFrame(raw, caps);
	if (!parsed.ok) return frameError(parsed.error.status, parsed.error.message);
	const frame = parsed.frame;

	if (!orgTokenOk) {
		const scopes = await userScopes(env, actorId);
		const name = parsePackageName(frame.name);
		if (name && name.scope && !scopes.includes(name.scope)) {
			return frameError(403, `scope @${name.scope} is not yours`);
		}
	}

	const guidesRaw = (raw as Record<string, unknown>)['guides'];
	const guides: Array<Record<string, string | null>> = [];
	if (guidesRaw !== undefined) {
		if (!Array.isArray(guidesRaw)) return frameError(400, 'invalid guides (expected an array)');
		for (const g of guidesRaw) {
			const entry = normalizeGuideEntry(g);
			if (!entry.ok) return frameError(400, entry.error);
			guides.push({ ...entry.entry });
		}
	}
	const guidesJson = JSON.stringify(guides);
	let docJson = '{"modules":[]}';
	const docsRaw = (raw as Record<string, unknown>)['docs'];
	if (docsRaw !== undefined && docsRaw !== null) {
		const normalized = normalizeDocJson(docsRaw);
		if (!normalized.ok) {
			return frameError(normalized.status, `invalid docs: ${normalized.error}`);
		}
		docJson = normalized.docJson;
	}
	const storedBytes = frame.readme.length + docJson.length + guidesJson.length;
	if (frame.readme.length > MAX_README_BYTES || docJson.length > MAX_DOC_JSON_BYTES) {
		return frameError(413, 'payload exceeds size limits');
	}
	if (guidesJson.length > MAX_GUIDES_BYTES || storedBytes > MAX_SNAPSHOT_BYTES) {
		return frameError(413, 'stored snapshot exceeds 512 KiB cap');
	}

	if (await versionReuseBlocked(env, frame.name, frame.version)) {
		const existing = await getVersionIndexRow(env, frame.name, frame.version);
		if (existing && frame.requestId && existing.row.requestId === frame.requestId) {
			return json({ success: true, url: `/packages/${frame.name}`, version: frame.version, duplicate: true }, { headers: specHeaders() });
		}
		return frameError(409, `version ${frame.version} already published (versions are immutable)`);
	}
	const recent = await recentVersionCount(env, frame.name, 7 * 24 * 3600);
	if (recent >= MAX_VERSIONS_PER_WEEK) {
		return frameError(429, `version rate limit exceeded (${MAX_VERSIONS_PER_WEEK}/week)`);
	}

	// First publish of a name claims owner; later session publishes need an
	// owner match. The org token skips this check as operator break-glass.
	const name = parsePackageName(frame.name);
	const claimedOwner = await getPackageOwner(env, name?.scope ?? '', name?.name ?? '');
	if (claimedOwner !== null && claimedOwner !== '' && !orgTokenOk && claimedOwner !== actorOwner) {
		return frameError(403, `package ${frame.name} is owned by someone else`);
	}
	const keywords = validateKeywords(frame.keywords ?? []);
	if (!keywords) {
		return frameError(400, 'invalid keywords (lowercase letters, digits and hyphens, max 12)');
	}

	// The server reads the package identity out of Project.config with its
	// own parser, then checks the frame against it. A lying client can at
	// most cause a rejection, never a stored lie.
	const sources: Record<string, string> = {};
	for (const f of frame.files) sources[f.path] = f.content;
	const configText = sources['Project.config'];
	let facts: Awaited<ReturnType<typeof configFacts>>;
	try {
		facts = await configFacts(env, configText ?? '', url.origin);
	} catch (e) {
		const msg = e instanceof Error ? e.message : String(e);
		return frameError(400, msg);
	}
	if (facts.name !== frame.name) {
		return frameError(400, `Project.config names \`${facts.name}\`, frame names \`${frame.name}\``);
	}
	if (facts.version !== frame.version) {
		return frameError(400, `Project.config versions \`${facts.version}\`, frame versions \`${frame.version}\``);
	}
	const entryTargets = new Set<string>();
	if (facts.entries.main) entryTargets.add(facts.entries.main);
	if (facts.entries.lib) entryTargets.add(facts.entries.lib);
	for (const target of Object.values(facts.entries.bins)) entryTargets.add(target);
	for (const target of entryTargets) {
		if (!sources[target]) {
			return frameError(400, `entry \`${target}\` is listed but not shipped`);
		}
	}
	for (const path of Object.keys(sources)) {
		if (!fileAllowed(path, entryTargets, facts.entries.docs)) {
			return frameError(400, `file \`${path}\` is outside the published file classes`);
		}
	}

	// Canonicalization: `.rnx` files go through the server's own minifier,
	// every other file is stored verbatim. The client never supplies stored
	// bytes under any path.
	let canonical: Record<string, string>;
	try {
		canonical = (await canonicalizeTree(env, sources, url.origin)).files;
	} catch (e) {
		const msg = e instanceof Error ? e.message : String(e);
		return frameError(400, msg);
	}
	canonical['Project.config'] = facts.redacted;
	const files: CanonicalFile[] = Object.keys(canonical)
		.sort()
		.map((path) => ({ name: path, bytes: new TextEncoder().encode(canonical[path]) }));
	const canonicalBytes = files.reduce((n, f) => n + f.bytes.length, 0);
	const bound = chunkCountBound(files);
	if (bound > caps.newChunks) {
		return frameError(413, `package needs ${bound} chunks, over the ${caps.newChunks} new-chunk limit`);
	}
	const chunked = await chunkCanonicalFiles(files);
	if (chunked.units.length > caps.chunksPerVersion) {
		return frameError(413, `package needs ${chunked.units.length} chunks, over the ${caps.chunksPerVersion} chunk limit`);
	}
	const digest = await manifestDigest(chunked.entries);

	// The storage quota is enforced before the analysis and before any
	// write, so a publish that cannot be stored costs a quota check and
	// nothing else. Only a stored version counts against the budget.
	try {
		await checkStorageQuota(env, canonicalBytes);
	} catch (e) {
		if (e instanceof Error && e.message.startsWith('quota-exceeded')) {
			return frameError(507, 'storage quota exceeded');
		}
		throw e;
	}

	// An identical chunk list means identical canonical bytes (every chunk
	// is hash-pinned), so the verdict cannot have changed: version-only
	// publishes become metadata writes with zero PUTs and no analysis.
	let verdictKey: string;
	let capabilities: DeclaredPermission[];
	let factRows: CanonicalFact[];
	let edges: { edges: string[]; permsDigest: string; files: Array<{ path: string; hash: string }> };
	const manifestJson = JSON.stringify({
		deps: facts.dependencies,
		permissions: facts.permissions.map((p) => (p.reason ? { perm: p.perm, reason: p.reason } : p.perm))
	});
	try {
		capabilities = parseManifestPermissions(manifestJson);
	} catch (e) {
		const msg = e instanceof Error ? e.message : String(e);
		return frameError(400, `invalid permissions: ${msg}`);
	}
	const latest = await getLatestVersion(env, frame.name);
	const latestRow = latest ? await getVersionIndexRow(env, frame.name, latest) : null;
	if (latestRow && !latestRow.withdrawn && latestRow.row.manifestDigest === digest) {
		verdictKey = latestRow.row.verdictKey;
		factRows = [];
		edges = (await getPackageEdges(env, frame.name)) ?? { edges: [], permsDigest: '', files: [] };
	} else {
		let analysis: Awaited<ReturnType<typeof analyzeTree>>;
		try {
			const rnxFiles: Record<string, string> = {};
			for (const [path, text] of Object.entries(canonical)) {
				if (path.endsWith('.rnx')) rnxFiles[path] = text;
			}
			analysis = await analyzeTree(env, rnxFiles, {
				main: facts.entries.main,
				bins: facts.entries.bins
			}, url.origin);
		} catch (e) {
			const msg = e instanceof Error ? e.message : String(e);
			return frameError(400, msg);
		}
		const used = new Map<string, { head: CapabilityHead; symbol: string; file: string }>();
		for (const cap of analysis.capabilities) {
			used.set(cap, { head: cap.split(':')[0] as CapabilityHead, symbol: cap, file: 'project' });
		}
		const unreachableFiles = canonicalFilesFor(canonical, analysis.unreachable);
		if (unreachableFiles.length > 0) {
			for (const hit of scanCanonicalCapabilities(unreachableFiles)) {
				const key = `${hit.head}:${hit.symbol}:${hit.file}`;
				if (!used.has(key)) used.set(key, hit);
			}
		}
		const coverage = checkCapabilityCoverage(capabilities, [...used.values()]);
		if (!coverage.ok) {
			return frameError(400, `undeclared capabilities: ${coverage.error}`);
		}
		const permsDigest = await sha256Hex(
			new TextEncoder().encode(
				[...new Set(capabilities.map((d) => `${d.domain}:${d.arg}`))].sort().join('\n')
			)
		);
		const sortedCaps = [...new Set(analysis.capabilities)].sort();
		verdictKey = await sha256Hex(
			new TextEncoder().encode(`${digest}\n${permsDigest}\n${sortedCaps.join('\n')}`)
		);
		const fileHashes = new Map<string, string>();
		for (const f of files) fileHashes.set(f.name, await sha256Hex(f.bytes));
		const entryChunks = new Map(chunked.entries.map((e) => [e.name, e.chunks] as const));
		factRows = files.map((f) => ({
			canonicalHash: fileHashes.get(f.name) ?? '',
			chunks: entryChunks.get(f.name) ?? [],
			imports: analysis.perFile[f.name]?.imports ?? [],
			evidence: analysis.perFile[f.name]?.capabilities ?? []
		}));
		const edgePairs: string[] = [];
		for (const [from, detail] of Object.entries(analysis.perFile)) {
			for (const imp of detail.imports) edgePairs.push(`${from}<-${imp}`);
		}
		edgePairs.sort();
		edges = {
			edges: edgePairs,
			permsDigest,
			files: files.map((f) => ({ path: f.name, hash: fileHashes.get(f.name) ?? '' }))
		};
	}
	const existing = await existingChunkHashes(env, chunked.units.map((u) => u.hash));
	const missing = chunked.units.filter((u) => !existing.has(u.hash));
	await r2PutChunks(env, missing);
	const payload: CanonicalPublishPayload = {
		name: frame.name,
		version: frame.version,
		description: frame.description || `${frame.name} package`,
		author: 'anonymous',
		license: frame.license,
		tags: frame.tags,
		keywords,
		readme: frame.readme,
		docJson,
		checksum: digest,
		engineRange: frame.engineRange,
		manifestJson,
		guidesJson,
		requestId: frame.requestId,
		owner: actorOwner,
		capabilities,
		entries: chunked.entries,
		manifestDigest: digest,
		verdictKey,
		units: chunked.units,
		facts: factRows,
		edges
	};
	try {
		const result = await publishCanonicalVersion(env, payload);
		if (!result.created) {
			return frameError(409, `version ${frame.version} already published`);
		}
		chargeStorageQuota(env, canonicalBytes);
	} catch (e) {
		const msg = e instanceof Error ? e.message : String(e);
		if (msg.includes('UNIQUE') || msg.includes('already published')) {
			return frameError(409, `version ${frame.version} already published`);
		}
		return frameError(500, 'ingest failed');
	}
	await purgeUrls(env, packagePointerUrls(url.origin, frame.name, frame.version), { fullName: frame.name });
	for (const dep of extractDepNames(manifestJson)) {
		await purgeUrls(env, [`${url.origin}/api/v0/packages/${dep}/dependents`], { fullName: dep });
	}
	return json({ success: true, url: `/packages/${frame.name}`, version: frame.version, manifestDigest: digest }, { status: 201, headers: specHeaders() });
}

function canonicalFilesFor(canonical: Record<string, string>, names: string[]): CanonicalFile[] {
	const enc = new TextEncoder();
	const out: CanonicalFile[] = [];
	for (const name of names) {
		const text = canonical[name];
		if (text === undefined || !name.endsWith('.rnx')) continue;
		out.push({ name, bytes: enc.encode(text) });
	}
	out.sort((a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0));
	return out;
}
