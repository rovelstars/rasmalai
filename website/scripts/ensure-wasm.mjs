import { execFileSync } from 'node:child_process';
import { existsSync, readdirSync, statSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const repo = join(root, '..');
const wasm = join(root, 'src', 'lib', 'wasm', 'wasm_playground_bg.wasm');
const inputs = [
	join(root, 'Cargo.toml'),
	join(root, 'package-lock.json'),
	join(repo, 'Cargo.toml'),
	join(repo, 'Cargo.lock'),
	join(repo, 'compiler', 'stdlib', 'src', 'lib.rs')
];

function collect(dir) {
	let entries;
	try {
		entries = readdirSync(dir, { withFileTypes: true });
	} catch {
		return;
	}
	for (const entry of entries) {
		if (entry.name.startsWith('.') || entry.name === 'target') continue;
		const path = join(dir, entry.name);
		if (entry.isDirectory()) collect(path);
		else if (/\.(rs|toml)$/.test(entry.name)) inputs.push(path);
	}
}

collect(join(repo, 'compiler', 'wasm_playground', 'src'));
collect(join(repo, 'compiler', 'frontend', 'src'));
collect(join(repo, 'compiler', 'runtime', 'src'));

if (existsSync(wasm)) {
	const built = statSync(wasm).mtimeMs;
	const newest = inputs.reduce((latest, path) => {
		try {
			return Math.max(latest, statSync(path).mtimeMs);
		} catch {
			return latest;
		}
	}, 0);
	if (built >= newest) process.exit(0);
}

console.log('playground engine is stale; rebuilding with wasm-pack...');
execFileSync(process.platform === 'win32' ? 'npm.cmd' : 'npm', ['run', 'build:wasm:ci'], {
	cwd: root,
	stdio: 'inherit'
});
