const BASIC: Record<number, string> = {
	30: 'ans-muted',
	31: 'ans-red',
	32: 'ans-green',
	33: 'ans-orange',
	34: 'ans-cyan',
	35: 'ans-purple',
	36: 'ans-cyan',
	37: 'ans-text',
	90: 'ans-muted',
	91: 'ans-red',
	92: 'ans-green',
	93: 'ans-orange',
	94: 'ans-cyan',
	95: 'ans-pink',
	96: 'ans-cyan',
	97: 'ans-text'
};

// Truecolor values emitted by the CLI for the Aura palette. Known values
// become theme-aware classes (see app.css); anything else keeps an exact
// inline style.
const TRUECLASS: Record<string, string> = {
	'109,109,109': 'ans-muted',
	'237,236,238': 'ans-text',
	'255,103,103': 'ans-red',
	'97,255,202': 'ans-green',
	'255,202,133': 'ans-orange',
	'130,226,255': 'ans-cyan',
	'162,119,255': 'ans-purple',
	'246,148,255': 'ans-pink'
};

function xterm256(n: number): string {
	if (n < 16) {
		const base = [
			'#000000', '#800000', '#008000', '#808000',
			'#000080', '#800080', '#008080', '#c0c0c0',
			'#808080', '#ff0000', '#00ff00', '#ffff00',
			'#0000ff', '#ff00ff', '#00ffff', '#ffffff'
		];
		return base[n];
	}
	if (n < 232) {
		const i = n - 16;
		const r = Math.floor(i / 36);
		const g = Math.floor((i % 36) / 6);
		const b = i % 6;
		const v = (c: number) => (c === 0 ? 0 : c * 40 + 55);
		return `rgb(${v(r)}, ${v(g)}, ${v(b)})`;
	}
	const g = (n - 232) * 10 + 8;
	return `rgb(${g}, ${g}, ${g})`;
}

function esc(s: string): string {
	return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}

export function ansiToHtml(input: string): string {
	const parts = input.split('\x1b[');
	let out = esc(parts[0]);
	let cls: string | null = null;
	let color: string | null = null;
	let bold = false;
	for (let i = 1; i < parts.length; i++) {
		const m = parts[i].match(/^([0-9;]*)m([\s\S]*)$/);
		if (!m) {
			out += esc('\x1b[' + parts[i]);
			continue;
		}
		const codes = m[1] === '' ? [0] : m[1].split(';').map(Number);
		for (let k = 0; k < codes.length; k++) {
			const c = codes[k];
			if (c === 0) {
				cls = null;
				color = null;
				bold = false;
			} else if (c === 1) {
				bold = true;
			} else if (c === 38 && codes[k + 1] === 2) {
				const key = `${codes[k + 2]},${codes[k + 3]},${codes[k + 4]}`;
				if (TRUECLASS[key] !== undefined) {
					cls = TRUECLASS[key];
					color = null;
				} else {
					cls = null;
					color = `rgb(${key.replace(/,/g, ', ')})`;
				}
				k += 4;
			} else if (c === 38 && codes[k + 1] === 5) {
				cls = null;
				color = xterm256(codes[k + 2]);
				k += 2;
			} else if (BASIC[c] !== undefined) {
				cls = BASIC[c];
				color = null;
			}
		}
		const text = esc(m[2]);
		if (cls || color || bold) {
			const style = `${color ? `color:${color};` : ''}${bold ? 'font-weight:600;' : ''}`;
			const attrs = `${cls ? `class="${cls}"` : ''}${style ? ` style="${style}"` : ''}`;
			out += `<span${attrs ? ` ${attrs}` : ''}>${text}</span>`;
		} else {
			out += text;
		}
	}
	return out;
}
