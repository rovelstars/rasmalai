import type { Component } from 'svelte';
import {
	Rocket,
	Hash,
	Braces,
	Split,
	Boxes,
	Shapes,
	Type,
	Library,
	KeyRound,
	Recycle,
	TriangleAlert,
	Activity,
	Cpu,
	ShieldAlert,
	FlaskConical,
	BookOpen,
	Terminal,
	Stethoscope,
	Wrench,
	Zap,
	Sprout,
	Sigma,
	HardDrive,
	Timer,
	Dices,
	Lock,
	Globe,
	Binary,
	Monitor,
	Link,
	Sparkles
} from 'lucide-svelte';

export interface GuideMeta {
	slug: string;
	title: string;
	description: string;
	// eslint-disable-next-line @typescript-eslint/no-explicit-any
	icon: any;
}

export interface ChapterMeta extends GuideMeta {
	path: string;
}

export const GUIDE_CHAPTERS: ChapterMeta[] = [
	{ slug: '01-introduction', title: 'Introduction', description: 'Install, run, and the one promise.', icon: Rocket, path: '/guide/01-introduction' },
	{ slug: '02-basics-and-types', title: 'Basics and Types', description: 'Bindings, 64-bit numbers, strings.', icon: Hash, path: '/guide/02-basics-and-types' },
	{ slug: '03-control-flow', title: 'Control Flow', description: 'if, for, while, switch, defer.', icon: Split, path: '/guide/03-control-flow' },
	{ slug: '04-functions-and-closures', title: 'Functions and Closures', description: 'Signatures, lambdas, defaults.', icon: Braces, path: '/guide/04-functions-and-closures' },
	{ slug: '05-data-structures', title: 'Data Structures', description: 'Structs, classes, records, enums.', icon: Boxes, path: '/guide/05-data-structures' },
	{ slug: '06-collections', title: 'Collections', description: 'Arrays, maps, sets, transforms.', icon: Type, path: '/guide/06-collections' },
	{ slug: '07-error-handling', title: 'Error Handling', description: 'Option, throws, try/catch.', icon: TriangleAlert, path: '/guide/07-error-handling' },
	{ slug: '08-modules-and-packages', title: 'Modules and Packages', description: 'Imports, stdlib, manifests.', icon: Library, path: '/guide/08-modules-and-packages' },
	{ slug: '09-editor-setup', title: 'Editor Setup', description: 'LSP and highlighting, rnx setup.', icon: Monitor, path: '/guide/09-editor-setup' },
	{ slug: '10-ai-assistants', title: 'AI Assistants & MCP', description: 'Agents, harnesses, rnx mcp.', icon: Terminal, path: '/guide/10-ai-assistants' },
	{ slug: '11-troubleshooting-and-reinstall', title: 'Troubleshooting & Reinstall', description: 'Diagnose, reinstall, uninstall.', icon: Wrench, path: '/guide/11-troubleshooting-and-reinstall' }
];

export interface ManualMeta extends GuideMeta {
	path: string;
	part: string;
}

export const MANUAL_PARTS = [
	'Syntax and Primitives',
	'Type System and Object Model',
	'Memory, Systems, and Concurrency',
	'Toolchain and Diagnostics'
] as const;

export const MANUAL_SECTIONS: ManualMeta[] = [
	{ slug: '01-lexicon-and-structure', title: 'Lexicon and Structure', description: 'Encoding, comments, statements, keywords.', icon: Type, path: '/manual/01-lexicon-and-structure', part: MANUAL_PARTS[0] },
	{ slug: '01a-prelude-and-intrinsics', title: 'Prelude and Intrinsics', description: 'Implicit scope, root intrinsics, core types.', icon: Sparkles, path: '/manual/01a-prelude-and-intrinsics', part: MANUAL_PARTS[0] },
	{ slug: '02-numeric-model', title: 'Numeric Model', description: 'Unified 64-bit semantics.', icon: Sigma, path: '/manual/02-numeric-model', part: MANUAL_PARTS[0] },
	{ slug: '03-bindings-and-scope', title: 'Bindings and Scope', description: 'let, const, shadowing, destructuring.', icon: KeyRound, path: '/manual/03-bindings-and-scope', part: MANUAL_PARTS[0] },
	{ slug: '04-control-flow', title: 'Control Flow', description: 'Branches, loops, switch, defer, guard.', icon: Split, path: '/manual/04-control-flow', part: MANUAL_PARTS[0] },
	{ slug: '05-functions-and-closures', title: 'Functions and Closures', description: 'Signatures, defaults, throws, channels.', icon: Braces, path: '/manual/05-functions-and-closures', part: MANUAL_PARTS[0] },
	{ slug: '06-structs-and-records', title: 'Structs and Records', description: 'Value types and anonymous records.', icon: Boxes, path: '/manual/06-structs-and-records', part: MANUAL_PARTS[1] },
	{ slug: '07-classes-and-objects', title: 'Classes and Objects', description: 'Heap identity, extends, methods.', icon: Shapes, path: '/manual/07-classes-and-objects', part: MANUAL_PARTS[1] },
	{ slug: '08-traits-and-interfaces', title: 'Traits and Interfaces', description: 'with composition, dispatch, is.', icon: Library, path: '/manual/08-traits-and-interfaces', part: MANUAL_PARTS[1] },
	{ slug: '09-extensions-and-operators', title: 'Extensions and Operators', description: 'Receiver desugaring, op_* hooks.', icon: Zap, path: '/manual/09-extensions-and-operators', part: MANUAL_PARTS[1] },
	{ slug: '10-enums-and-matching', title: 'Enums and Matching', description: 'Tagged unions, exhaustiveness.', icon: Dices, path: '/manual/10-enums-and-matching', part: MANUAL_PARTS[1] },
	{ slug: '11-memory-and-arc', title: 'Memory and ARC', description: 'Deterministic ARC, write-dominance.', icon: HardDrive, path: '/manual/11-memory-and-arc', part: MANUAL_PARTS[2] },
	{ slug: '12-cycles-and-handles', title: 'Cycles and Handles', description: 'GenRef, byId, decay.', icon: Recycle, path: '/manual/12-cycles-and-handles', part: MANUAL_PARTS[2] },
	{ slug: '13-hardware-and-ffi', title: 'Hardware and FFI', description: 'unsafe, native, files, processes.', icon: Cpu, path: '/manual/13-hardware-and-ffi', part: MANUAL_PARTS[2] },
	{ slug: '14-concurrency-and-threads', title: 'Concurrency and Threads', description: 'Pools, barriers, async.', icon: Activity, path: '/manual/14-concurrency-and-threads', part: MANUAL_PARTS[2] },
	{ slug: '15-vectorization-and-simd', title: 'Vectorization and SIMD', description: 'Vec4f lanes and reductions.', icon: Timer, path: '/manual/15-vectorization-and-simd', part: MANUAL_PARTS[2] },
	{ slug: '16-project-and-toolchain', title: 'Project and Toolchain', description: 'Project.config, rnx CLI, tests.', icon: Terminal, path: '/manual/16-project-and-toolchain', part: MANUAL_PARTS[3] },
	{ slug: '17-diagnostics-directory', title: 'Diagnostics Directory', description: 'Every code, cause, and fix.', icon: Stethoscope, path: '/manual/17-diagnostics-directory', part: MANUAL_PARTS[3] }
];

export const LEGACY_GUIDE_REDIRECTS: Record<string, string> = {
	'getting-started': '/manual/16-project-and-toolchain',
	variables: '/manual/03-bindings-and-scope',
	functions: '/manual/05-functions-and-closures',
	'control-flow': '/manual/04-control-flow',
	structs: '/manual/06-structs-and-records',
	enums: '/manual/10-enums-and-matching',
	strings: '/manual/01-lexicon-and-structure',
	modules: '/manual/16-project-and-toolchain',
	ownership: '/manual/11-memory-and-arc',
	cycles: '/manual/12-cycles-and-handles',
	errors: '/manual/05-functions-and-closures',
	concurrency: '/manual/14-concurrency-and-threads',
	simd: '/manual/15-vectorization-and-simd',
	unsafe: '/manual/13-hardware-and-ffi',
	testing: '/manual/16-project-and-toolchain',
	docs: '/manual/16-project-and-toolchain',
	cli: '/manual/16-project-and-toolchain',
	diagnostics: '/manual/17-diagnostics-directory',
	syntax: '/manual/03-bindings-and-scope',
	memory: '/manual/11-memory-and-arc'
};

export const LEGACY_MANUAL_REDIRECTS: Record<string, string> = {
	'01-numeric-model': '/manual/02-numeric-model',
	'02-memory-and-arc': '/manual/11-memory-and-arc',
	'03-cycles-and-handles': '/manual/12-cycles-and-handles',
	'04-protocols-and-operators': '/manual/09-extensions-and-operators',
	'05-binary-and-io': '/manual/13-hardware-and-ffi',
	'06-concurrency-model': '/manual/14-concurrency-and-threads',
	'07-toolchain-and-config': '/manual/16-project-and-toolchain'
};

export interface RosettaMeta {
	slug: string;
	title: string;
	label: string;
	dialect: 'rust' | 'go' | 'cpp' | 'typescript';
	blurb: string;
	path: string;
}

export const GUIDES: GuideMeta[] = [
	{ slug: 'getting-started', title: 'Getting Started', description: 'Install, scaffold, run.', icon: Rocket },
	{ slug: 'variables', title: 'Variables & Numbers', description: 'Bindings, widths, Float vs FastFloat.', icon: Hash },
	{ slug: 'functions', title: 'Functions', description: 'Signatures, methods, throws, tests.', icon: Braces },
	{ slug: 'control-flow', title: 'Control Flow', description: 'if, for, while, switch, defer.', icon: Split },
	{ slug: 'structs', title: 'Structs & Records', description: 'Data, classes, traits.', icon: Boxes },
	{ slug: 'enums', title: 'Enums & Pattern Matching', description: 'Shapes and exhaustive switch.', icon: Shapes },
	{ slug: 'strings', title: 'Strings & Arrays', description: 'Text, interpolation, sequences.', icon: Type },
	{ slug: 'modules', title: 'Modules & Packages', description: 'Imports, manifests, workspaces.', icon: Library },
	{ slug: 'ownership', title: 'Ownership', description: 'Scopes, stack, heap, ARC.', icon: KeyRound },
	{ slug: 'cycles', title: 'Cycles, GenRef & Destructors', description: 'Weak handles and decay.', icon: Recycle },
	{ slug: 'errors', title: 'Error Handling', description: 'throws, try/catch, dispatch.', icon: TriangleAlert },
	{ slug: 'concurrency', title: 'Concurrency', description: 'Threads, sync, async.', icon: Activity },
	{ slug: 'simd', title: 'SIMD & Numerics', description: 'Vec4f lanes and reductions.', icon: Cpu },
	{ slug: 'unsafe', title: 'Unsafe & Native Code', description: 'Raw memory and C-ABI.', icon: ShieldAlert },
	{ slug: 'testing', title: 'Testing & Benchmarking', description: 'test fn, bench, asserts.', icon: FlaskConical },
	{ slug: 'docs', title: 'Documentation', description: 'Doc comments and rnx doc.', icon: BookOpen },
	{ slug: 'cli', title: 'Toolchain Reference', description: 'Every rnx subcommand.', icon: Terminal },
	{ slug: 'diagnostics', title: 'Diagnostics', description: 'Error codes and fixes.', icon: Stethoscope }
];

export const STDLIB = [
	'prelude',
	'simd',
	'collections',
	'math',
	'fs',
	'bytes',
	'time',
	'random',
	'sync',
	'env',
	'process',
	'os',
	'testing',
	'web',
	'json',
	'net',
	'io'
];

// eslint-disable-next-line @typescript-eslint/no-explicit-any
export const STDLIB_ICONS: Record<string, any> = {
	prelude: Sparkles,
	simd: Cpu,
	collections: Boxes,
	math: Sigma,
	file: HardDrive,
	bytes: Binary,
	time: Timer,
	random: Dices,
	sync: Lock,
	env: Globe,
	process: Terminal,
	os: Monitor,
	testing: FlaskConical,
	web: Link
};

export interface TrackMeta {
	slug: string;
	title: string;
	description: string;
	// eslint-disable-next-line @typescript-eslint/no-explicit-any
	icon: any;
}

export const TRACKS: TrackMeta[] = [
	{ slug: 'fast-track', title: 'The 15-Minute Tour', description: 'Memory, engines, SIMD, ABI.', icon: Zap },
	{ slug: 'gentle-ramp', title: 'Gentle Ramp', description: 'Zero to systems.', icon: Sprout }
];

export interface RosettaMeta {
	slug: string;
	title: string;
	label: string;
	dialect: 'rust' | 'go' | 'cpp' | 'typescript';
	blurb: string;
}

export const ROSETTA: RosettaMeta[] = [
	{ slug: 'rust', title: 'From Rust', label: 'Rust', dialect: 'rust', blurb: 'Lifetimes become lexical scopes.', path: '/guide/rosetta/from-rust' },
	{ slug: 'go', title: 'From Go', label: 'Go', dialect: 'go', blurb: 'Goroutine clarity, no GC pauses.', path: '/guide/rosetta/from-go' },
	{ slug: 'cpp', title: 'From C / C++', label: 'C++', dialect: 'cpp', blurb: '-O3 speed, no UB.', path: '/guide/rosetta/from-cpp' },
	{ slug: 'typescript', title: 'From TypeScript', label: 'TypeScript', dialect: 'typescript', blurb: 'Modules to bare metal.', path: '/guide/rosetta/from-typescript' }
];
