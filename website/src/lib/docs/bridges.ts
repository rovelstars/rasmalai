import type { Dialect } from '$lib/stores/dialect.svelte';

export interface Bridge {
	from: string;
	text: string;
}

type Lang = Exclude<Dialect, 'none'>;

const GENERIC: Record<Lang, Bridge> = {
	rust: {
		from: "borrow checker",
		text: 'Ownership works like Rust without the annotations: scopes own values and free them at the end, so the mental model transfers and the lifetime puzzles stay behind.'
	},
	go: {
		from: 'garbage collector',
		text: 'Expect Go-like build speed and readability, but every value frees deterministically at scope end — there is no collector pause hiding in p99.'
	},
	cpp: {
		from: 'undefined behavior',
		text: 'Out-of-bounds access, null dereference, and use-after-free trap or fail to compile here instead of invoking undefined behavior.'
	},
	typescript: {
		from: 'V8 runtime',
		text: 'The syntax will feel familiar, but every construct below compiles to native code with hardware-sized types — no JIT warmup, no event loop.'
	}
};

const CHAPTERS: Record<string, Partial<Record<Lang, Bridge>>> = {
	variables: {
		rust: {
			from: 'let / mut',
			text: '`let` is mutable by default here — the opposite of Rust. Reach for `const` when you mean immutable, and expect fixed-width `Int` with no `isize` surprises.'
		},
		go: {
			from: ':= and var',
			text: 'Like `:=` everywhere, except the type never wobbles: `Int` is always 64 bits and `Float` is always IEEE-754 double.'
		},
		cpp: {
			from: 'int / auto',
			text: 'There is no platform-dependent `int` and no implicit narrowing. `Int` is 64 bits everywhere; overflow wraps instead of invoking undefined behavior.'
		},
		typescript: {
			from: 'number',
			text: 'There is no single `number` type: `Int`, `Float`, and `FastFloat` are distinct, and mixing the float kinds without `.asFast()` is a compile error instead of a silent precision change.'
		}
	},
	functions: {
		rust: {
			from: 'fn and Drop',
			text: 'Signatures read like Rust, but `defer {}` blocks replace `Drop` impls for cleanup, and `throws` plus `try` replace `Result` plumbing.'
		},
		go: {
			from: 'func and defer',
			text: '`defer` stacks exactly like Go, and multiple return values become either tuples via records or explicit `throws` instead of `(T, error)` pairs.'
		},
		cpp: {
			from: 'free functions',
			text: 'No headers, no forward declarations, no overload resolution puzzles: one definition, called by name, checked at the call site.'
		},
		typescript: {
			from: 'function and arrow fns',
			text: 'Signatures carry real hardware types, and `test fn` blocks turn any function file into a test suite with one keyword.'
		}
	},
	'control-flow': {
		rust: {
			from: 'match and Drop guards',
			text: '`switch` is `match` with auto-break and provable exhaustiveness; `defer` covers what RAII guards did, written inline in LIFO order.'
		},
		go: {
			from: 'for / switch / defer',
			text: 'All three transfer almost verbatim — one `for` form, auto-breaking `switch`, stacking `defer`. New here: `guard let ... else` for bind-or-diverge.'
		},
		cpp: {
			from: 'switch fallthrough',
			text: 'Cases never fall through by accident: crossing cases needs explicit `fallthrough;`, and there is no C-style `for(;;)` — that is an `E110` error.'
		},
		typescript: {
			from: 'for loops and switch',
			text: '`for...in` arrays only, `switch` on patterns with guards, and `defer` for the cleanup you used to scatter across `finally` blocks.'
		}
	},
	structs: {
		rust: {
			from: 'structs and impls',
			text: 'Data plus `impl`-style methods without trait plumbing for the basics; `init` constructs positionally and `static` methods need no receiver.'
		},
		go: {
			from: 'structs and methods',
			text: 'Memberwise construction like a struct literal, methods with receivers like Go — plus `init`/`deinit` lifecycle hooks and real `private`.'
		},
		cpp: {
			from: 'classes and RAII',
			text: 'Construction without header/implementation splits and destruction without manual pairing: `init` builds, `deinit` tears down, ARC frees.'
		},
		typescript: {
			from: 'classes and interfaces',
			text: '`struct` is your interface-shaped data with construction built in; `trait` declares the capability sets that `class ... with` adopts.'
		}
	},
	enums: {
		rust: {
			from: 'enums and match',
			text: 'Payload enums with leading-dot patterns and exhaustiveness checking, minus lifetime parameters on every variant holder.'
		},
		go: {
			from: 'iota and type switches',
			text: 'Real sum types instead of constant blocks: payloads ride with the variant and `switch` proves you handled them all.'
		},
		cpp: {
			from: 'enum class and visitors',
			text: 'No `std::variant` plus visitor boilerplate: variants carry payloads natively and the `switch` is checked exhaustive.'
		},
		typescript: {
			from: 'unions and narrowing',
			text: 'Discriminated-union modeling with compiler-checked narrowing — the exhaustiveness TypeScript only gets via `never` tricks.'
		}
	},
	strings: {
		rust: {
			from: 'String vs &str',
			text: 'No `String`/`&str` split to navigate: one `Str` type, value comparison, and `{expr}` interpolation built into every literal.'
		},
		go: {
			from: 'strings and runes',
			text: 'UTF-8 strings with value equality and inline interpolation, plus arrays whose out-of-bounds access traps instead of panicking.'
		},
		cpp: {
			from: 'std::string',
			text: 'No SSO-vs-heap reasoning and no iterator-pair APIs: concatenate with `+`, compare by value, interpolate inline.'
		},
		typescript: {
			from: 'template literals',
			text: '`{expr}` interpolation will feel like home; the difference is fixed-width siblings (`Array<Int>`) with trapping bounds.'
		}
	},
	modules: {
		rust: {
			from: 'mod and crates',
			text: 'One namespace per file like Rust modules, but imports are always explicit paths — no `mod` tree declarations, no `extern crate`.'
		},
		go: {
			from: 'packages and GOPATH',
			text: 'File-per-namespace imports like Go packages, with `@std/` stdlib embedded in the binary instead of fetched.'
		},
		cpp: {
			from: 'headers and CMake',
			text: 'No headers, no include guards, no CMake: `import` names symbols directly and `Project.config` replaces the build scripts.'
		},
		typescript: {
			from: 'npm imports',
			text: 'Import specifiers like ESM with zero `node_modules`: `@std/` resolves from the compiler and path deps resolve from the manifest.'
		}
	},
	ownership: {
		rust: {
			from: "lifetimes ('a)",
			text: 'This chapter is the payoff: lexical scopes do the job lifetimes did — the closing brace is the free point, with no annotations to write or fight.'
		},
		go: {
			from: 'escape analysis',
			text: 'No escape analysis to second-guess: stack for locals, ARC heap objects otherwise, and borrows that skip retain/release on most calls.'
		},
		cpp: {
			from: 'new / delete and smart pointers',
			text: 'No manual pairing and no `shared_ptr` cycles to audit: scopes free, ARC shares, and the one hard case (cycles) gets the next chapter.'
		},
		typescript: {
			from: 'GC generations',
			text: 'No nursery, no mark phase, no pause histogram: values die when their block ends, deterministically, on the releasing thread.'
		}
	},
	cycles: {
		rust: {
			from: 'Weak<T> and Pin',
			text: '`GenRef<T>` is `Weak` without the upgrade ceremony (`.get()` yields `null` when expired), and `fn decay(this)` replaces `Pin` gymnastics for self-capturing closures.'
		},
		go: {
			from: 'reference cycles under GC',
			text: 'Go never makes you think about cycles; here you think about them exactly once, at the back-edge, by reaching for `GenRef`.'
		},
		cpp: {
			from: 'weak_ptr',
			text: '`GenRef` is `weak_ptr` with expiry as `null` instead of an empty shared pointer — same direction rule (own down, weaken up).'
		},
		typescript: {
			from: 'closure captures',
			text: 'Closures capturing their own object cycle the same way closures over DOM nodes used to leak: `fn decay(this)` is the one-word fix.'
		}
	},
	errors: {
		rust: {
			from: 'Result and ?',
			text: '`throws` plus `try` is `Result` without the generics: propagation by declaring `throws`, handling by `catch` with `switch` + `is` dispatch.'
		},
		go: {
			from: 'if err != nil',
			text: 'The repetitive guard collapses to one `try` at the boundary; intermediate layers just declare `throws` and stay clean.'
		},
		cpp: {
			from: 'exceptions and error codes',
			text: 'Neither exception tables nor integer codes: thrown values with `switch` dispatch, no unwinding tables, no `noexcept` audits.'
		},
		typescript: {
			from: 'try/catch and rejected promises',
			text: 'Synchronous `try`/`catch` with typed dispatch — no `.catch()` chains, no unhandled-rejection warnings, no async coloring.'
		}
	},
	concurrency: {
		rust: {
			from: 'Send/Sync and rayon',
			text: 'Threads share through `byId` primitives instead of `Send` bounds, and `ThreadPool.parallelFor` covers the rayon-shaped workloads.'
		},
		go: {
			from: 'goroutines and channels',
			text: '`Channel` mirrors Go channels including `byId` sharing, but workers compute owned values and return `Int` — no goroutine leaks, no scheduler pauses.'
		},
		cpp: {
			from: 'std::thread and atomics',
			text: '`AtomicInt`/`Mutex`/`RwLock` map directly onto what you know, constructed `byId` for sharing, with `async`/`await` polling for the cooperative side.'
		},
		typescript: {
			from: 'event loop and workers',
			text: 'Real OS threads instead of worker-thread message passing, plus cooperative `async` tasks driven by explicit polling — no microtask mysteries.'
		}
	},
	simd: {
		rust: {
			from: 'std::simd / packed_simd',
			text: '`Vec4f` is the portable-simd idea with stable syntax: lane operators, reductions, and first-class function values on all backends.'
		},
		go: {
			from: 'assembly shims',
			text: 'No assembly stubs or compiler-intrinsic whack-a-mole: `Vec4f` is a value type with operators, in the language, on every backend.'
		},
		cpp: {
			from: '<immintrin.h>',
			text: 'The type is the instruction: no intrinsic spelling bees, no alignment attributes, and lane reads that trap instead of faulting obscurely.'
		},
		typescript: {
			from: 'typed arrays',
			text: 'Like `Float32Array` lanes but register-resident with operators: arithmetic, `dot`, `min`/`max`, `sqrt` — no bounds-check ceremony per access.'
		}
	}
};

export function bridgeFor(slug: string, dialect: Dialect): Bridge | null {
	if (dialect === 'none') return null;
	return CHAPTERS[slug]?.[dialect as Lang] ?? GENERIC[dialect as Lang] ?? null;
}
