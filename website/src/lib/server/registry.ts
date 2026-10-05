export const SPEC_VERSION = 1;
export const CAPABILITIES = ['tombstones', 'guides', 'jsdoc', 'yank', 'transfer'];

export function specHeaders(): Record<string, string> {
	return { 'rnx-registry-spec': String(SPEC_VERSION) };
}

export interface Semver {
	major: number;
	minor: number;
	patch: number;
	prerelease: string;
	raw: string;
}

// Supported range grammar: `*` | `latest` | `X.Y.Z` | `^X.Y.Z` | `~X.Y.Z` | `>=X.Y.Z`.
const SEMVER_RE = /^(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?$/;

export function isValidRange(range: string): boolean {
	const r = range.trim();
	if (r === '' || r === '*' || r === 'latest') return true;
	if (r.startsWith('^') || r.startsWith('~')) return parseSemver(r.slice(1).trim()) !== null;
	if (r.startsWith('>=')) return parseSemver(r.slice(2).trim()) !== null;
	return parseSemver(r) !== null;
}

export function parseSemver(v: string): Semver | null {
	const m = SEMVER_RE.exec(v);
	if (!m) return null;
	return {
		major: Number(m[1]),
		minor: Number(m[2]),
		patch: Number(m[3]),
		prerelease: m[4] ?? '',
		raw: v
	};
}

function comparePrerelease(a: string, b: string): number {
	if (a === b) return 0;
	if (a === '') return 1;
	if (b === '') return -1;
	const pa = a.split('.');
	const pb = b.split('.');
	for (let i = 0; i < Math.max(pa.length, pb.length); i++) {
		const x = pa[i];
		const y = pb[i];
		if (x === undefined) return -1;
		if (y === undefined) return 1;
		if (x === y) continue;
		const nx = /^\d+$/.test(x) ? Number(x) : null;
		const ny = /^\d+$/.test(y) ? Number(y) : null;
		if (nx !== null && ny !== null) return nx - ny;
		return x < y ? -1 : 1;
	}
	return 0;
}

export function compareSemver(a: Semver, b: Semver): number {
	if (a.major !== b.major) return a.major - b.major;
	if (a.minor !== b.minor) return a.minor - b.minor;
	if (a.patch !== b.patch) return a.patch - b.patch;
	return comparePrerelease(a.prerelease, b.prerelease);
}

export function satisfiesRange(version: string, range: string): boolean {
	const v = parseSemver(version);
	if (!v) return false;
	const r = range.trim();
	if (r === '' || r === '*' || r === 'latest') return v.prerelease === '';
	if (v.prerelease !== '' && !rangeAdmitsPrerelease(r, v)) return false;
	if (r.startsWith('^')) {
		const base = parseSemver(r.slice(1).trim());
		if (!base) return false;
		if (compareSemver(v, base) < 0) return false;
		if (base.major > 0) return v.major === base.major;
		if (base.minor > 0) return v.major === 0 && v.minor === base.minor;
		return v.major === 0 && v.minor === 0 && v.patch === base.patch;
	}
	if (r.startsWith('~')) {
		const base = parseSemver(r.slice(1).trim());
		if (!base) return false;
		if (compareSemver(v, base) < 0) return false;
		return v.major === base.major && v.minor === base.minor;
	}
	if (r.startsWith('>=')) {
		const base = parseSemver(r.slice(2).trim());
		return !!base && compareSemver(v, base) >= 0;
	}
	const exact = parseSemver(r);
	return !!exact && compareSemver(v, exact) === 0;
}

function rangeAdmitsPrerelease(range: string, v: Semver): boolean {
	let base: Semver | null = null;
	if (range.startsWith('^') || range.startsWith('~')) base = parseSemver(range.slice(1).trim());
	else if (range.startsWith('>=')) base = parseSemver(range.slice(2).trim());
	else if (range === '' || range === '*' || range === 'latest') return false;
	else base = parseSemver(range);
	if (!base || base.prerelease === '') return false;
	return base.major === v.major && base.minor === v.minor && base.patch === v.patch;
}

export function maxSatisfying(versions: string[], range: string): string | null {
	let best: Semver | null = null;
	for (const s of versions) {
		if (!satisfiesRange(s, range)) continue;
		const v = parseSemver(s);
		if (v && (!best || compareSemver(v, best) > 0)) best = v;
	}
	return best ? best.raw : null;
}

export function selectLatestVersion(versions: Array<{ version: string; status: string }>): string | null {
	const live: Semver[] = [];
	for (const v of versions) {
		if (v.status !== 'live') continue;
		const s = parseSemver(v.version);
		if (s) live.push(s);
	}
	if (live.length === 0) return null;
	const stable = live.filter((s) => s.prerelease === '');
	const pool = stable.length > 0 ? stable : live;
	let best = pool[0];
	for (const s of pool) if (compareSemver(s, best) > 0) best = s;
	return best.raw;
}

export interface DepNode {
	id: string;
	deps: string[];
}

export function levelize(nodes: DepNode[]): string[][] {
	const byId = new Map(nodes.map((n) => [n.id, n]));
	const state = new Map<string, number>();
	const depth = new Map<string, number>();
	const visit = (id: string, stack: string[]): number => {
		const s = state.get(id) ?? 0;
		if (s === 2) return depth.get(id) ?? 0;
		if (s === 1) throw new Error(`dependency cycle: ${[...stack, id].join(' -> ')}`);
		state.set(id, 1);
		const node = byId.get(id);
		let d = 0;
		if (node) {
			for (const dep of node.deps) {
				if (byId.has(dep)) d = Math.max(d, visit(dep, [...stack, id]) + 1);
			}
		}
		state.set(id, 2);
		depth.set(id, d);
		return d;
	};
	for (const n of nodes) visit(n.id, []);
	const levels: string[][] = [];
	for (const n of nodes) {
		const d = depth.get(n.id) ?? 0;
		while (levels.length <= d) levels.push([]);
		levels[d].push(n.id);
	}
	return levels.reverse();
}
