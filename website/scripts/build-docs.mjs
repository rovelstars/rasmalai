import { execFileSync } from 'node:child_process';
import { existsSync, mkdirSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const outDir = join(root, 'static', 'data');
const compilerDir = join(root, '..', 'compiler');

mkdirSync(outDir, { recursive: true });

// Regenerates static/data/api.json from the compiler. The file is
// gitignored and served same-origin at /data/api.json, so a missing file
// is fine: pages render a "No API data yet" empty state instead of
// failing the build. Refresh locally with:
//   npm run build:docs
try {
	execFileSync('cargo', ['run', '-q', '-p', 'docgen', '--', outDir], {
		cwd: compilerDir,
		stdio: 'inherit'
	});
	console.log('docs: api.json regenerated');
} catch {
	if (existsSync(join(outDir, 'api.json'))) {
		console.log('docs: cargo unavailable, keeping existing api.json');
	} else {
		console.log('docs: cargo unavailable and no api.json present, continuing without API data');
	}
}
