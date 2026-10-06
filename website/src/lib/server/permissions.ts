// Declared-permission parsing for the registry.
//
// Capability grammar mirror. Source of truth is the compiler:
// compiler/frontend/src/capabilities.rs (`Capability::from_str`).
// Keep CAPABILITY_DOMAINS and splitPermission in sync when that grammar
// changes. The server never re-derives capabilities here (that is the
// wasm analyzer track); it only validates and stores what publishers
// declare, split into a fixed domain head plus an opaque remainder.

export const CAPABILITY_DOMAINS = [
	'fs:read',
	'fs:write',
	'fs:delegated',
	'net:http',
	'net:ws',
	'net:delegated',
	'sys:exec',
	'env:read',
	'env:dump',
	'term:write',
	'term:read',
	'term:raw',
	'unsafe:ffi',
	'unsafe:raw_memory'
] as const;

export type CapabilityDomain = (typeof CAPABILITY_DOMAINS)[number];

const BARE_DOMAINS = new Set<string>([
	'fs:delegated',
	'net:delegated',
	'unsafe:ffi',
	'unsafe:raw_memory',
	'term:write',
	'term:read',
	'term:raw',
	'env:dump'
]);

const PARAMETRIC_DOMAINS = new Set<string>([
	'fs:read',
	'fs:write',
	'net:http',
	'net:ws',
	'sys:exec',
	'env:read'
]);

export interface DeclaredPermission {
	domain: string;
	arg: string;
	reason: string | null;
}

export function splitPermission(perm: string): { domain: string; arg: string } {
	if (typeof perm !== 'string' || perm === '') {
		throw new Error(`malformed permission \`${String(perm)}\``);
	}
	if (BARE_DOMAINS.has(perm)) return { domain: perm, arg: '' };
	const first = perm.indexOf(':');
	const second = first < 0 ? -1 : perm.indexOf(':', first + 1);
	if (first <= 0 || second <= first + 1) {
		throw new Error(`malformed permission \`${perm}\``);
	}
	const domain = perm.slice(0, second);
	if (!PARAMETRIC_DOMAINS.has(domain)) {
		throw new Error(`unknown permission \`${perm}\``);
	}
	return { domain, arg: perm.slice(second + 1) };
}

function toDeclared(item: unknown): DeclaredPermission {
	if (typeof item === 'string') {
		const { domain, arg } = splitPermission(item);
		return { domain, arg, reason: null };
	}
	if (typeof item === 'object' && item !== null && !Array.isArray(item)) {
		const table = item as Record<string, unknown>;
		if (typeof table['perm'] !== 'string') {
			throw new Error('permission entries need a string `perm`');
		}
		const { domain, arg } = splitPermission(table['perm']);
		const reason = table['reason'];
		if (reason !== undefined && reason !== null && typeof reason !== 'string') {
			throw new Error('permission `reason` must be a string');
		}
		return { domain, arg, reason: typeof reason === 'string' ? reason : null };
	}
	throw new Error('permissions must be strings or `{ perm, reason }` tables');
}

export function parseManifestPermissions(manifestJson: string): DeclaredPermission[] {
	let manifest: Record<string, unknown>;
	try {
		manifest = JSON.parse(manifestJson) as Record<string, unknown>;
	} catch {
		return [];
	}
	const raw = manifest['permissions'];
	if (raw === undefined) return [];
	if (!Array.isArray(raw)) {
		throw new Error('manifest `permissions` must be an array');
	}
	return raw.map(toDeclared);
}
