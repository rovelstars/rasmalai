import { redirect } from '@sveltejs/kit';
import { LEGACY_GUIDE_REDIRECTS } from '$lib/docs/nav';

export function entries() {
	return Object.keys(LEGACY_GUIDE_REDIRECTS).map((slug) => ({ slug }));
}

export function load({ params }) {
	redirect(308, LEGACY_GUIDE_REDIRECTS[params.slug] ?? '/manual');
}
