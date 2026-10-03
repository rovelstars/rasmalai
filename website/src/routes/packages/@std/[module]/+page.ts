import { error } from '@sveltejs/kit';
import { STD_MODULES, ENGINE_VERSION, ensureApi, stdApiModule, stdMeta } from '$lib/docs/stdlib';

export function entries() {
	return STD_MODULES.map((m) => ({ module: m.name }));
}

export async function load({ params, fetch }) {
	if (!STD_MODULES.some((m) => m.name === params.module)) error(404, 'package not found');
	const meta = stdMeta(params.module);
	await ensureApi(fetch);
	const mod = stdApiModule(params.module);
	return { meta, mod, engine: ENGINE_VERSION, noApi: mod === null };
}
