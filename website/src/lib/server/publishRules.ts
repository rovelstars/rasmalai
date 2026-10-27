import { analyzeTree, configFacts } from './analysis.js';
import { getChunkBytes, getLatestVersion, getVersionRow, listPackages } from './db.js';
import type { TarEntryRef } from './chunks.js';

// Report-only rule scan over stored packages. The registry-key predicate
// runs here exactly as it runs on the publish path (one full section-7b
// analysis per package, same wasm entries), so the count the operator
// reads is the count the flip would enforce on. Runtime access is not
// counted: it fails whole-tree analysis outright, so a stored package
// carrying it could only predate enforcement. The scan is read-only: it
// reassembles stored bytes through the chunk table and writes nothing.
//
// Cost: one whole-tree re-analysis per scanned package plus one chunk
// read per stored file, all sequential. Whole-tree analysis costs about
// 0.25 ms per KiB of canonical source (spec section 8c); chunk reads
// are content-addressed and deduplicated. The per-package byte cap and
// the package limit bound a single call; a registry larger than the
// limit reports truncated: true and the operator pages through with
// repeated calls.

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

async function readStoredSources(
	env: Record<string, string | undefined>,
	entries: TarEntryRef[]
): Promise<{ config: string; rnx: Record<string, string> } | null> {
	const text = new TextDecoder();
	let config: string | null = null;
	const rnx: Record<string, string> = {};
	let total = 0;
	const ordered = [...entries].sort((a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0));
	for (const entry of ordered) {
		if (!entry || entry.dir) continue;
		const wantConfig = entry.name === 'Project.config';
		const wantRnx = entry.name.endsWith('.rnx');
		if (!wantConfig && !wantRnx) continue;
		const parts: number[] = [];
		for (const hash of entry.chunks ?? []) {
			const bytes = await getChunkBytes(env, hash);
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
	for (const full of names.slice(0, limit)) {
		let version: string | null = null;
		try {
			version = await getLatestVersion(env, full);
			if (!version) {
				scan.skipped.push({ name: full, version, reason: 'no-live-version' });
				continue;
			}
			const found = await getVersionRow(env, full, version);
			if (!found || found.withdrawn) {
				scan.skipped.push({ name: full, version, reason: 'withdrawn' });
				continue;
			}
			let manifest: TarEntryRef[];
			try {
				manifest = JSON.parse(found.row.tarManifestJson) as TarEntryRef[];
				if (!Array.isArray(manifest)) throw new Error('not an array');
			} catch {
				scan.skipped.push({ name: full, version, reason: 'unreadable-manifest' });
				continue;
			}
			const stored = await readStoredSources(env, manifest);
			if (!stored) {
				scan.skipped.push({ name: full, version, reason: 'over-scan-budget-or-missing-bytes' });
				continue;
			}
			if (Object.keys(stored.rnx).length === 0) {
				scan.skipped.push({ name: full, version, reason: 'no-rnx-sources' });
				continue;
			}
			const facts = await configFacts(stored.config);
			const analysis = await analyzeTree(env, stored.rnx, {
				main: facts.entries.main,
				bins: facts.entries.bins,
				packageName: full,
				configText: stored.config
			});
			scan.scanned += 1;
			if (analysis.reportOnly.registries.keys.length > 0) {
				scan.registries.count += 1;
				scan.registries.packages.push({
					name: full,
					version,
					keys: [...analysis.reportOnly.registries.keys].sort()
				});
			}
		} catch (e) {
			const message = e instanceof Error ? e.message : String(e);
			scan.skipped.push({ name: full, version, reason: `analysis-failed: ${message.slice(0, 160)}` });
		}
	}
	return scan;
}
