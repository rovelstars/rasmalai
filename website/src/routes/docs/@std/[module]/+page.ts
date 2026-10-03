import { redirect } from '@sveltejs/kit';
import { STD_MODULES } from '$lib/docs/stdlib';

export function entries() {
	return STD_MODULES.map((m) => ({ module: m.name }));
}

export async function load({ params }) {
	redirect(308, `/docs/@std/${params.module}/overview`);
}
