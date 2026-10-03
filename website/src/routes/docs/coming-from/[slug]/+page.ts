import { redirect } from '@sveltejs/kit';

export function entries() {
	return [{ slug: 'rust' }, { slug: 'go' }, { slug: 'cpp' }, { slug: 'typescript' }];
}

export function load({ params }) {
	redirect(308, `/guide/rosetta/from-${params.slug}`);
}
