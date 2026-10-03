import { execFileSync, execSync } from 'node:child_process';
import { existsSync, mkdtempSync, mkdirSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const compilerDir = join(root, '..', 'compiler');
const dataFile = join(root, 'static', 'data', 'cli-demos.json');

function rnxBin() {
	try {
		execFileSync('cargo', ['build', '-q', '-p', 'cli'], { cwd: compilerDir, stdio: 'pipe' });
	} catch {
		throw new Error('cargo build failed');
	}
	const candidates = [
		join(compilerDir, '..', 'target', 'debug', 'rnx'),
		join(compilerDir, 'target', 'debug', 'rnx')
	];
	const hit = candidates.find((p) => existsSync(p));
	if (!hit) throw new Error('rnx binary not found in workspace or compiler target dir');
	return hit;
}

function makeFixture(withError) {
	const dir = mkdtempSync(join(tmpdir(), 'rnx-demo-'));
	mkdirSync(join(dir, 'src'), { recursive: true });
	writeFileSync(
		join(dir, 'Project.config'),
		'export default {\n    project: {\n        name: "demo",\n        version: "0.1.0"\n    }\n}\n'
	);
	writeFileSync(
		join(dir, 'src', 'main.rnx'),
		'fn Main(): Int {\n    print("demo ok");\n    return 0;\n}\n'
	);
	if (withError) {
		writeFileSync(
			join(dir, 'src', 'bad.rnx'),
			'fn broken(): Int {\n    return nosuchvar;\n}\n'
		);
	} else {
		mkdirSync(join(dir, 'tests'), { recursive: true });
		writeFileSync(
			join(dir, 'tests', 'demo.rnx'),
			'test fn adds_up() {\n    assert(1 + 1 == 2, "math");\n}\ntest fn shadows() {\n    let x = 1;\n    let x = x + 1;\n    assert(x == 2, "shadow");\n}\n'
		);
	}
	return dir;
}

function capture(bin, dir, args) {
	const cmd = [bin, ...args].map((a) => `'${a.replace(/'/g, "'\\''")}'`).join(' ');
	const env = {
		...process.env,
		COLORTERM: 'truecolor',
		TERM: 'xterm-256color'
	};
	delete env.NO_COLOR;
	try {
		const out = execSync(`script -q -e -c ${JSON.stringify(cmd)} /dev/null`, {
			cwd: dir,
			env,
			maxBuffer: 1024 * 1024
		});
		return out.toString();
	} catch (e) {
		return ((e.stdout ?? Buffer.alloc(0)).toString() || (e.message ?? '')).toString();
	}
}

function stripCarriage(s) {
	return s.replace(/\r\n/g, '\n').replace(/\r/g, '\n');
}

try {
	const bin = rnxBin();
	const errDir = makeFixture(true);
	const okDir = makeFixture(false);
	try {
		const demos = {
			check: stripCarriage(capture(bin, errDir, ['check', 'src/bad.rnx'])),
			test: stripCarriage(capture(bin, okDir, ['test'])),
			lint: stripCarriage(capture(bin, okDir, ['lint', 'tests/demo.rnx']))
		};
		mkdirSync(dirname(dataFile), { recursive: true });
		writeFileSync(dataFile, JSON.stringify(demos, null, 2) + '\n');
		console.log('demos: cli-demos.json regenerated');
	} finally {
		rmSync(errDir, { recursive: true, force: true });
		rmSync(okDir, { recursive: true, force: true });
	}
} catch (e) {
	console.log(`demos: skipped (${e instanceof Error ? e.message : e})`);
}
