import { analyzeTree, configFacts, type DepSourceCache, type ReportOnlyVerdicts } from './analysis.js';
import { fileAdvisory, listActiveAdvisories, type AdvisoryInput } from './advisories.js';
import { getChunkBytesBatched, getScanSnapshots, listPackages } from './db.js';
import type { TarEntryRef } from './chunks.js';

// The registry-key predicate runs on the publish path and over the live
// set through the same function, so the count the operator reads is the
// count the gate enforces. Runtime access is not counted: it fails
// whole-tree analysis outright, so a stored package carrying it could
// only predate enforcement. The scan is read-only: it reassembles stored
// bytes through the chunk table and writes nothing. Advisory filing for
// stored violators lives in ensureRegistriesAdvisories below, never in
// the scan, so a GET cannot mint advisories as a side effect.
//
// Cost: one whole-tree re-analysis per scanned package, all sequential.
// Store reads are batched (one version snapshot round trip and one chunk
// round trip per scan, plus one cached dep load per @std top), so the
// store round trips of a scan stay flat as the package count grows instead
// of dying at the platform subrequest ceiling partway through. Whole-tree
// analysis costs about 0.25 ms per KiB of canonical source; the per-package
// byte cap and the package limit bound a single call; a registry larger
// than the limit reports truncated: true and the operator pages through
// with repeated calls.

export const SCAN_DEFAULT_LIMIT = 200;
export const SCAN_MAX_LIMIT = 1000;
export const SCAN_MAX_PACKAGE_BYTES = 8 * 1024 * 1024;

export interface RegistriesPackageHit {
	name: string;
	version: string;
	keys: string[];
}

export interface SkippedPackage {
	name: string;
	version: string | null;
	reason: string;
}

export interface PublishRulesScan {
	scanned: number;
	skipped: SkippedPackage[];
	truncated: boolean;
	limit: number;
	registries: { count: number; packages: RegistriesPackageHit[] };
	budget: { maxPackages: number; maxPackageBytes: number };
}

export function parseScanLimit(raw: string | null | undefined): number {
	const n = Number(raw ?? '');
	if (!Number.isFinite(n) || n <= 0) return SCAN_DEFAULT_LIMIT;
	return Math.min(Math.floor(n), SCAN_MAX_LIMIT);
}

// The one predicate the scan counts and the publish gate enforces, over
// the same wasm verdict. Anything the scan would count, the gate rejects.
export function registriesKeysIn(reportOnly: ReportOnlyVerdicts | null | undefined): string[] {
	const keys = reportOnly?.registries?.keys ?? [];
	return [...new Set(keys)].sort();
}

// The publish rejection for a violating frame: names the keys present and
// the fix. The fix deletes both keys even when only one tripped, so a
// republish cannot trade one key for the other.
export function registriesRejection(keys: string[]): string | null {
	if (keys.length === 0) return null;
	const named = keys.map((k) => `\`${k}\``).join(' and ');
	return (
		`E108: Project.config carries top-level ${named} ` +
		`(stored packages must not pin registry choice; the installer chooses): ` +
		'delete the `registry` and `registries` keys from Project.config and republish ' +
		'as the next version (versions are immutable)'
	);
}

function advisorySlug(full: string, version: string): string {
	const slug = `${full}@${version}`
		.toLowerCase()
		.replace(/[^a-z0-9._-]+/g, '-')
		.replace(/^-+|-+$/g, '')
		.slice(0, 44);
	return `stored-registries-${slug}`;
}

// The critical advisory shape for a stored violator: cause plus remedy.
// Deterministic id, so repeated filings converge instead of duplicating.
export function registriesAdvisoryFor(full: string, version: string, keys: string[]): AdvisoryInput {
	const named = [...keys].sort().join(', ');
	return {
		id: advisorySlug(full, version),
		package: full,
		affected: version,
		severity: 'critical',
		title: 'published Project.config pins registry choice',
		detail:
			`stored violator ${full}@${version}: Project.config carries top-level ` +
			`keys (${named}), served verbatim at install while registry choice must stay ` +
			'with the installer. Remedy: delete the `registry` and `registries` keys ' +
			'from Project.config and republish as the next version (versions are immutable).'
	};
}

export interface RegistriesAdvisoryOutcome {
	scanned: number;
	limit: number;
	truncated: boolean;
	count: number;
	filed: string[];
	present: string[];
}

// Files the critical advisory for every stored violator that lacks one.
// Read-then-write: the scan itself never files, so the GET stays
// side-effect free and this runs only from the admin POST.
export async function ensureRegistriesAdvisories(
	env: Record<string, string | undefined>,
	opts: { limit?: number } = {}
): Promise<RegistriesAdvisoryOutcome> {
	const scan = await scanPublishReportOnly(env, opts);
	const active = await listActiveAdvisories(env);
	const have = new Set(active.map((a) => a.id));
	const outcome: RegistriesAdvisoryOutcome = {
		scanned: scan.scanned,
		limit: scan.limit,
		truncated: scan.truncated,
		count: scan.registries.count,
		filed: [],
		present: []
	};
	for (const hit of scan.registries.packages) {
		const input = registriesAdvisoryFor(hit.name, hit.version, hit.keys);
		const id = input.id ?? '';
		if (have.has(id)) {
			outcome.present.push(id);
			continue;
		}
		try {
			const filed = await fileAdvisory(env, input);
			have.add(filed.id);
			outcome.filed.push(filed.id);
		} catch (e) {
			if (e instanceof Error && e.message.includes('already exists')) {
				have.add(id);
				outcome.present.push(id);
				continue;
			}
			throw e;
		}
	}
	return outcome;
}

function sortEntries(entries: TarEntryRef[]): TarEntryRef[] {
	return [...entries].sort((a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0));
}

function reassembleSources(
	entries: TarEntryRef[],
	blobs: Map<string, Uint8Array | null>
): { config: string; rnx: Record<string, string> } | null {
	const text = new TextDecoder();
	let config: string | null = null;
	const rnx: Record<string, string> = {};
	let total = 0;
	for (const entry of sortEntries(entries)) {
		if (!entry || entry.dir) continue;
		const wantConfig = entry.name === 'Project.config';
		const wantRnx = entry.name.endsWith('.rnx');
		if (!wantConfig && !wantRnx) continue;
		const parts: number[] = [];
		for (const hash of entry.chunks ?? []) {
			const bytes = blobs.get(hash);
			if (!bytes) return null;
			for (const b of bytes) parts.push(b);
		}
		total += parts.length;
		if (total > SCAN_MAX_PACKAGE_BYTES) return null;
		const decoded = text.decode(new Uint8Array(parts));
		if (wantConfig) config = decoded;
		else rnx[entry.name] = decoded;
	}
	if (config === null) return null;
	return { config, rnx };
}

export async function scanPublishReportOnly(
	env: Record<string, string | undefined>,
	opts: { limit?: number } = {}
): Promise<PublishRulesScan> {
	const limit =
		opts.limit === undefined ? SCAN_DEFAULT_LIMIT : Math.min(Math.max(Math.floor(opts.limit), 1), SCAN_MAX_LIMIT);
	const summaries = await listPackages(env);
	const names = summaries.map((s) => s.name).sort();
	const scan: PublishRulesScan = {
		scanned: 0,
		skipped: [],
		truncated: names.length > limit,
		limit,
		registries: { count: 0, packages: [] },
		budget: { maxPackages: limit, maxPackageBytes: SCAN_MAX_PACKAGE_BYTES }
	};
	const slice = names.slice(0, limit);
	const snapshots = await getScanSnapshots(env, slice);
	const manifests = new Map<string, TarEntryRef[]>();
	const pending: string[] = [];
	for (const full of slice) {
		const snap = snapshots.get(full);
		if (!snap) {
			scan.skipped.push({ name: full, version: null, reason: 'no-live-version' });
			continue;
		}
		if (snap.withdrawn) {
			scan.skipped.push({ name: full, version: snap.version, reason: 'withdrawn' });
			continue;
		}
		let manifest: TarEntryRef[];
		try {
			manifest = JSON.parse(snap.tarManifestJson) as TarEntryRef[];
			if (!Array.isArray(manifest)) throw new Error('not an array');
		} catch {
			scan.skipped.push({ name: full, version: snap.version, reason: 'unreadable-manifest' });
			continue;
		}
		manifests.set(full, manifest);
	}
	const allHashes: string[] = [];
	for (const full of slice) {
		const manifest = manifests.get(full);
		if (!manifest) continue;
		let size = 0;
		const hashes: string[] = [];
		for (const entry of sortEntries(manifest)) {
			if (!entry || entry.dir) continue;
			if (entry.name !== 'Project.config' && !entry.name.endsWith('.rnx')) continue;
			size += entry.size ?? 0;
			for (const hash of entry.chunks ?? []) hashes.push(hash);
		}
		if (size > SCAN_MAX_PACKAGE_BYTES) {
			scan.skipped.push({
				name: full,
				version: snapshots.get(full)?.version ?? null,
				reason: 'over-scan-budget-or-missing-bytes'
			});
			continue;
		}
		allHashes.push(...hashes);
		pending.push(full);
	}
	const blobs = await getChunkBytesBatched(env, allHashes);
	const depCache: DepSourceCache = new Map();
	for (const full of pending) {
		const snap = snapshots.get(full);
		const manifest = manifests.get(full);
		if (!snap || !manifest) continue;
		try {
			const stored = reassembleSources(manifest, blobs);
			if (!stored) {
				scan.skipped.push({ name: full, version: snap.version, reason: 'over-scan-budget-or-missing-bytes' });
				continue;
			}
			if (Object.keys(stored.rnx).length === 0) {
				scan.skipped.push({ name: full, version: snap.version, reason: 'no-rnx-sources' });
				continue;
			}
			const facts = await configFacts(stored.config);
			const analysis = await analyzeTree(
				env,
				stored.rnx,
				{
					main: facts.entries.main,
					bins: facts.entries.bins,
					packageName: full,
					configText: stored.config
				},
				depCache
			);
			scan.scanned += 1;
			const keys = registriesKeysIn(analysis.reportOnly);
			if (keys.length > 0) {
				scan.registries.count += 1;
				scan.registries.packages.push({ name: full, version: snap.version, keys });
			}
		} catch (e) {
			const message = e instanceof Error ? e.message : String(e);
			scan.skipped.push({ name: full, version: snap.version, reason: `analysis-failed: ${message.slice(0, 160)}` });
		}
	}
	return scan;
}
