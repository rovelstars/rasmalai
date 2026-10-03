export type Dialect = 'none' | 'rust' | 'go' | 'cpp' | 'typescript';

const STORAGE_KEY = 'rnx_dialect_lens';

let current: Dialect = 'none';

if (typeof window !== 'undefined') {
	try {
		const saved = window.localStorage.getItem(STORAGE_KEY);
		if (saved === 'rust' || saved === 'go' || saved === 'cpp' || saved === 'typescript') {
			current = saved;
		}
	} catch {
		current = 'none';
	}
}

let state = $state<Dialect>(current);

function persist(d: Dialect) {
	try {
		window.localStorage.setItem(STORAGE_KEY, d);
	} catch {
		/* storage unavailable */
	}
}

export function getDialect(): Dialect {
	return state;
}

export function setDialect(d: Dialect): void {
	state = d;
	if (typeof window !== 'undefined') persist(d);
}

export function isDialectActive(d: Dialect): boolean {
	return state === d;
}
