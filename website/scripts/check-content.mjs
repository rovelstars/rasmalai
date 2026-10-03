import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..', 'src', 'content');
const dirs = ['tracks', 'coming-from', 'guide', 'manual'];
let failures = 0;

function fail(msg) {
	console.error('FAIL: ' + msg);
	failures++;
}

function collect(dir) {
	const out = [];
	for (const file of readdirSync(dir)) {
		const full = join(dir, file);
		if (statSync(full).isDirectory()) {
			out.push(...collect(full));
		} else if (file.endsWith('.md')) {
			out.push(full);
		}
	}
	return out;
}

for (const dir of dirs) {
	for (const path of collect(join(root, dir))) {
		const src = readFileSync(path, 'utf8');
		const id = path.slice(root.length + 1);
		const fm = src.match(/^---\n([\s\S]*?)\n---\n/);
		if (!fm) {
			fail(`${id}: missing frontmatter`);
			continue;
		}
		const meta = {};
		for (const line of fm[1].split('\n')) {
			const i = line.indexOf(':');
			if (i > 0) meta[line.slice(0, i).trim()] = line.slice(i + 1).trim();
		}
		for (const key of ['title', 'description']) {
			if (!meta[key]) fail(`${id}: frontmatter missing ${key}`);
		}
		if (dir === 'tracks' && !meta['track']) fail(`${id}: frontmatter missing track`);
		if (dir === 'coming-from' && !meta['sourceLang']) fail(`${id}: frontmatter missing sourceLang`);
		if (id.includes('guide/rosetta/') && !meta['sourceLang'])
			fail(`${id}: frontmatter missing sourceLang`);
		const fences = (src.match(/```/g) ?? []).length;
		if (fences % 2 !== 0) fail(`${id}: unbalanced fences (${fences})`);
		const blocks = [...src.matchAll(/```rnx\n([\s\S]*?)```/g)];
		if (blocks.length === 0) fail(`${id}: no rnx blocks`);
		for (const b of blocks) {
			if (b[1].trim().length === 0) fail(`${id}: empty rnx block`);
		}
		console.log(`ok ${id}: ${(meta.title ?? '').slice(0, 40)} (${blocks.length} rnx blocks)`);
	}
}

if (failures > 0) process.exit(1);
console.log('content check passed');
