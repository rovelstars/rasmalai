export type CapabilityTier = 'pure' | 'delegated' | 'ambient' | 'hazard';

export interface CallTraceNode {
	file: string;
	line: number;
	col: number;
	symbol: string;
	expression_snippet: string;
}

export interface CallChain {
	capability: string;
	is_delegated: boolean;
	nodes: CallTraceNode[];
}

export interface PackageCapabilityManifest {
	name: string;
	version: string;
	tier: CapabilityTier;
	has_hazard: boolean;
	capabilities: string[];
	traces?: CallChain[];
	summary?: string;
}

export interface TierMeta {
	label: string;
	blurb: string;
	pill: string;
}

export const TIER_META: Record<CapabilityTier, TierMeta> = {
	pure: {
		label: 'PURE',
		blurb: 'Zero capabilities required',
		pill: 'bg-emerald-50 text-emerald-700 border-emerald-200'
	},
	delegated: {
		label: 'DELEGATED',
		blurb: 'I/O strictly on arguments passed by caller',
		pill: 'bg-amber-50 text-amber-700 border-amber-200'
	},
	ambient: {
		label: 'AMBIENT',
		blurb: 'Static endpoints, ambient files, or environment variables',
		pill: 'bg-orange-50 text-orange-700 border-orange-200'
	},
	hazard: {
		label: 'HAZARD',
		blurb: 'Spawns child processes or uses raw memory/FFI',
		pill: 'bg-red-50 text-red-700 border-red-200'
	}
};

const TIERS: CapabilityTier[] = ['pure', 'delegated', 'ambient', 'hazard'];

export function isTier(value: unknown): value is CapabilityTier {
	return typeof value === 'string' && (TIERS as string[]).includes(value);
}

export function isHazardCapability(capability: string): boolean {
	return capability.startsWith('sys:exec:') || capability.startsWith('unsafe:');
}

export function nodeLocation(node: CallTraceNode): string {
	return `${node.file}:${node.line}:${node.col}`;
}

export function validateManifest(input: unknown): PackageCapabilityManifest {
	if (typeof input !== 'object' || input === null) {
		throw new Error('capability manifest must be an object');
	}
	const m = input as Record<string, unknown>;
	if (typeof m.name !== 'string' || m.name === '') {
		throw new Error('capability manifest needs a string `name`');
	}
	if (typeof m.version !== 'string' || m.version === '') {
		throw new Error('capability manifest needs a string `version`');
	}
	if (!isTier(m.tier)) {
		throw new Error('capability manifest needs a valid `tier`');
	}
	if (!Array.isArray(m.capabilities) || !m.capabilities.every((c) => typeof c === 'string')) {
		throw new Error('capability manifest needs a string array `capabilities`');
	}
	if (m.traces !== undefined) {
		if (!Array.isArray(m.traces)) {
			throw new Error('capability manifest `traces` must be an array');
		}
		for (const t of m.traces) {
			if (typeof t !== 'object' || t === null) {
				throw new Error('capability manifest `traces` entries must be objects');
			}
			const chain = t as Record<string, unknown>;
			if (typeof chain.capability !== 'string' || typeof chain.is_delegated !== 'boolean') {
				throw new Error('trace entries need `capability` and `is_delegated`');
			}
			if (!Array.isArray(chain.nodes)) {
				throw new Error('trace entries need a `nodes` array');
			}
		}
	}
	return {
		name: m.name,
		version: m.version,
		tier: m.tier,
		has_hazard: m.has_hazard === true || m.capabilities.some((c) => isHazardCapability(c as string)),
		capabilities: m.capabilities as string[],
		traces: m.traces as CallChain[] | undefined,
		summary: typeof m.summary === 'string' ? m.summary : undefined
	};
}
