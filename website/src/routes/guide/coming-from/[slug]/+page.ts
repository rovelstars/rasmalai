import { error, redirect } from '@sveltejs/kit';

const LEGACY: Record<string, string> = {
	rust: 'from-rust',
	go: 'from-go',
	cpp: 'from-cpp',
	typescript: 'from-typescript'
};

export function entries() {
	return Object.keys(LEGACY).map((slug) => ({ slug }));
}

export async function load({ params }) {
	const target = LEGACY[params.slug];
	if (!target) error(404, 'guide not found');
	redirect(308, `/guide/rosetta/${target}`);
}
