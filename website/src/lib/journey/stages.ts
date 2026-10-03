export interface CheckError {
	code: string;
	line: number;
	text: string;
}

export interface TreeNode {
	label: string;
	detail?: string;
	cls?: string;
	children?: TreeNode[];
}

export interface LirLine {
	text: string;
	shed?: boolean;
}

export type ArtifactDef =
	| { kind: 'source'; code: string }
	| { kind: 'morph'; before: string; after: string; badge: string }
	| { kind: 'check'; code: string; errors: CheckError[] }
	| { kind: 'lsp'; code: string; word: string; sig: string; doc: string }
	| { kind: 'tokens'; code: string; chips: { text: string; cls: string }[] }
	| { kind: 'ast'; tree: TreeNode[] }
	| { kind: 'lir'; lines: LirLine[]; counter?: { from: number; to: number } }
	| { kind: 'fork' }
	| { kind: 'run'; lines: string[] };

export interface Stage {
	id: string;
	act: string;
	kicker: string;
	title: string;
	body: string;
	artifact: ArtifactDef;
}

const SHADE = `fn shade(sun: Vec4f): Float {
    defer { print("shade done"); }
    let acc = Vec4f.splat(0.0);
    let i = 0;
    while i < 4 {
        acc = acc + sun;
        i = i + 1;
    }
    return acc.x();
}`;

const MESSY = `fn shade(sun:Vec4f):Float{
let acc=Vec4f.splat(0.0);
    let i=0;
while i<4{
acc=acc+sun;i=i+1;}
return acc.x();}`;

const CHECK_SRC = `fn double(x: Int): Int => x * 2;

fn triple(x: Int): Int => x * 3;

print(double(21) + triple(14));`;

const LINT_BEFORE = `class B {
    init() { this.cb = () => this.flush(); }
    fn flush() { }
}`;

const LINT_AFTER = `class B {
    init() { this.cb = fn decay(this) { this.flush() } }
    fn flush() { }
}`;

const LSP_SRC = `let v = Vec4f.splat(1.0);
print(v.x());`;

const TOKEN_SRC = `let total = (x) => x * 2;`;

const K = 'text-aura-purple';
const T = 'text-aura-text';
const M = 'text-aura-muted';
const O = 'text-aura-orange';

export const STAGES: Stage[] = [
	{
		id: 'write',
		act: 'act 1 - make it right',
		kicker: '01 - write',
		title: 'Your editor already speaks rnx',
		body: 'Keywords, types, and strings light up as you type. Brackets match, blocks fold, docs peek on hover.',
		artifact: { kind: 'source', code: SHADE }
	},
	{
		id: 'fmt',
		act: 'act 1 - make it right',
		kicker: '02 - fmt',
		title: 'One true style, applied not argued',
		body: 'rnx fmt normalizes spacing and indentation across the whole file. Idempotent: the second pass changes nothing.',
		artifact: { kind: 'morph', before: MESSY, after: SHADE, badge: 'rnx fmt - 0 diffs on second pass' }
	},
	{
		id: 'check',
		act: 'act 1 - make it right',
		kicker: '03 - check',
		title: 'Every error, one pass, with a span',
		body: 'The checker reports all syntax errors at once — each with a caret you can click, not one-at-a-time whack-a-mole.',
		artifact: {
			kind: 'check',
			code: CHECK_SRC,
			errors: [
				{ code: 'E105', line: 1, text: 'invalid lambda syntax — named functions need a block' },
				{ code: 'E105', line: 3, text: 'invalid lambda syntax — named functions need a block' }
			]
		}
	},
	{
		id: 'lint',
		act: 'act 1 - make it right',
		kicker: '04 - lint + fix',
		title: 'Lints that fix themselves',
		body: 'A closure stored on its own owner trips W104. Rewrite it with fn decay(this) to capture GenRef(this) instead.',
		artifact: {
			kind: 'morph',
			before: LINT_BEFORE,
			after: LINT_AFTER,
			badge: 'W104 - capture with fn decay(this)'
		}
	},
	{
		id: 'lsp',
		act: 'act 1 - make it right',
		kicker: '05 - lsp',
		title: 'Hover, jump, complete',
		body: 'The language server answers hover, go-to-definition, and completions for any editor speaking the protocol.',
		artifact: {
			kind: 'lsp',
			code: LSP_SRC,
			word: 'splat',
			sig: 'fn Vec4f.splat(v: Float): Vec4f',
			doc: 'Broadcast one scalar to all four lanes. No allocation, no ARC traffic.'
		}
	},
	{
		id: 'parse',
		act: 'act 2 - make it run',
		kicker: '06 - parse',
		title: 'Characters become a tree',
		body: 'The lexer cuts source into tokens; the parser grows them into a typed tree. Desugar lowers the surface syntax first.',
		artifact: {
			kind: 'tokens',
			code: TOKEN_SRC,
			chips: [
				{ text: 'let', cls: K },
				{ text: 'total', cls: T },
				{ text: '=', cls: M },
				{ text: '(', cls: M },
				{ text: 'x', cls: T },
				{ text: ')', cls: M },
				{ text: '=>', cls: K },
				{ text: 'x', cls: T },
				{ text: '*', cls: M },
				{ text: '2', cls: O },
				{ text: ';', cls: M }
			]
		}
	},
	{
		id: 'ast',
		act: 'act 2 - make it run',
		kicker: '07 - tree',
		title: 'The shape of the program',
		body: 'Expand the nodes. This is exactly what the later passes walk — nothing hidden, nothing inferred.',
		artifact: {
			kind: 'ast',
			tree: [
				{
					label: 'Module',
					cls: 'text-aura-pink',
					children: [
						{
							label: 'Fn shade(sun: Vec4f): Float',
							cls: 'text-aura-purple',
							detail: 'decl',
							children: [
								{
									label: 'Block',
									cls: 'text-aura-cyan',
									children: [
										{ label: 'Defer', detail: 'stmt' },
										{ label: 'Let acc = Vec4f.splat(0.0)', detail: 'stmt' },
										{
											label: 'While i < 4',
											detail: 'stmt',
											children: [
												{ label: 'Assign acc', detail: 'stmt' },
												{ label: 'Assign i', detail: 'stmt' }
											]
										},
										{ label: 'Return acc.x()', detail: 'stmt' }
									]
								}
							]
						}
					]
				}
			]
		}
	},
	{
		id: 'lower',
		act: 'act 2 - make it run',
		kicker: '08 - lower',
		title: 'One IR, every backend',
		body: 'The checked tree lowers to LIR — simplified below. The interpreter, Cranelift, and LLVM all read this same text.',
		artifact: {
			kind: 'lir',
			lines: [
				{ text: 'fn shade:' },
				{ text: '  v0 = splat 0.0' },
				{ text: '  v1 = call Vec4f.splat' },
				{ text: 'loop:' },
				{ text: '  v2 = add v0, sun' },
				{ text: '  v3 = lt i, 4' },
				{ text: '  br v3, loop, done' },
				{ text: 'done:' },
				{ text: '  ret v2' }
			]
		}
	},
	{
		id: 'opt',
		act: 'act 2 - make it run',
		kicker: '09 - optimize',
		title: 'Dead code fades out',
		body: 'Inline, dead-code removal, tail calls, and ARC motion run to fixed point. Unused functions simply stop existing.',
		artifact: {
			kind: 'lir',
			lines: [
				{ text: 'fn shade:' },
				{ text: '  v0 = splat 0.0' },
				{ text: '  v1 = call Vec4f.splat' },
				{ text: '  v9 = call debug_format', shed: true },
				{ text: '  v10 = alloc tmp_buf', shed: true },
				{ text: 'loop:' },
				{ text: '  v2 = add v0, sun' },
				{ text: '  v11 = retain v2', shed: true },
				{ text: '  v3 = lt i, 4' },
				{ text: '  br v3, loop, done' },
				{ text: 'done:' },
				{ text: '  ret v2' }
			],
			counter: { from: 1240, to: 310 }
		}
	},
	{
		id: 'fork',
		act: 'act 2 - make it run',
		kicker: '10 - two backends',
		title: 'Pick your hurry',
		body: 'Flip the switch. Dev compiles in milliseconds; release takes seconds and flies. Same checksum either way.',
		artifact: { kind: 'fork' }
	},
	{
		id: 'run',
		act: 'act 2 - make it run',
		kicker: '11 - run',
		title: 'Keystroke to binary',
		body: 'Dev runs it now through the JIT; release ships a stripped binary. Either way the output agrees bit-for-bit.',
		artifact: {
			kind: 'run',
			lines: ['$ rnx build --release', 'built target/shade (ok)', '$ ./shade', 'shade done', '42', 'checksum match']
		}
	}
];
