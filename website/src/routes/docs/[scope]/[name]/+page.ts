import { STD_MODULES } from '$lib/docs/stdlib';

export function entries() {
	return STD_MODULES.map((m) => ({ scope: '@std', name: m.name }));
}

export async function load({ data }) {
	return data;
}
