import { error, redirect } from '@sveltejs/kit';

export const prerender = false;

const NAME_RE = /^[a-z0-9][a-z0-9-]*$/;

const RESERVED = new Set(['tracks', 'coming-from']);

export function load({ params }) {
	const name = params.name ?? '';
	if (!NAME_RE.test(name) || RESERVED.has(name)) error(404, 'package not found');
	redirect(308, `/docs/${name}/overview`);
}
