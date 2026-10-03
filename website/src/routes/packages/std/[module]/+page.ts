import { redirect } from '@sveltejs/kit';
import { STDLIB } from '$lib/docs/nav';

export function entries() {
	return STDLIB.map((module) => ({ module }));
}

export async function load({ params }) {
	redirect(308, `/packages/@std/${params.module}`);
}
