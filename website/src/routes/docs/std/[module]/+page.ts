import { redirect } from '@sveltejs/kit';
import { STDLIB } from '$lib/docs/nav';

export function entries() {
	return STDLIB.map((module) => ({ module }));
}

export function load({ params }) {
	redirect(308, `/docs/@std/${params.module}/overview`);
}
