import { error, redirect } from '@sveltejs/kit';
import { STD_MODULES } from '$lib/docs/stdlib';

export function entries() {
	return STD_MODULES.map((m) => ({ scope: '@std', name: m.name }));
}

const SCOPE_RE = /^@[a-z0-9][a-z0-9-]*$/;
const NAME_RE = /^[a-z0-9][a-z0-9-]*$/;

export async function load({ params }) {
	const scope = params.scope ?? '';
	const name = params.name ?? '';
	if (!SCOPE_RE.test(scope) || !NAME_RE.test(name)) error(404, 'package not found');
	redirect(308, `/docs/${scope}/${name}/overview`);
}
