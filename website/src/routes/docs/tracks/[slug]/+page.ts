import { redirect } from '@sveltejs/kit';

export function entries() {
	return [{ slug: 'fast-track' }, { slug: 'gentle-ramp' }];
}

export function load({ params }) {
	redirect(308, `/guide/tracks/${params.slug}`);
}
