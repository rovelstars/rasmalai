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
	Sparkles,
	Package
} from 'lucide-svelte';

import { buildChapters } from './chapters';

const CHAPTER_ICONS = {
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
	Sparkles,
	Package
};

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

export interface ManualMeta extends GuideMeta {
	path: string;
	part: string;
}

function withIcons(chapters: { slug: string; title: string; description: string; icon: string | null; part: string | null; path: string }[], withPart: true): ManualMeta[];
function withIcons(chapters: { slug: string; title: string; description: string; icon: string | null; part: string | null; path: string }[], withPart: false): ChapterMeta[];
function withIcons(chapters: { slug: string; title: string; description: string; icon: string | null; part: string | null; path: string }[], withPart: boolean) {
	return chapters.map((c) => ({
		slug: c.slug,
		title: c.title,
		description: c.description,
		// eslint-disable-next-line @typescript-eslint/no-explicit-any
		icon: (c.icon && (CHAPTER_ICONS as Record<string, any>)[c.icon]) ?? null,
		path: c.path,
		...(withPart ? { part: c.part ?? '' } : {})
	}));
}

const guideFiles = import.meta.glob('/src/content/guide/*.md', { query: '?raw', import: 'default', eager: true }) as Record<string, string>;
const manualFiles = import.meta.glob('/src/content/manual/*.md', { query: '?raw', import: 'default', eager: true }) as Record<string, string>;

export const GUIDE_CHAPTERS: ChapterMeta[] = withIcons(buildChapters(guideFiles, '/guide'), false);

export const MANUAL_SECTIONS: ManualMeta[] = withIcons(buildChapters(manualFiles, '/manual'), true);

export const MANUAL_PARTS: string[] = [...new Set(MANUAL_SECTIONS.map((m) => m.part))];

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
