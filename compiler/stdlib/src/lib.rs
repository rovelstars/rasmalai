pub const MODULES: &[&str] = &[
    "prelude",
    "simd",
    "collections",
    "math",
    "fs",
    "bytes",
    "time",
    "random",
    "sync",
    "env",
    "process",
    "os",
    "testing",
    "web",
    "net",
    "net/http",
    "json",
    "io",
];

pub fn source(_name: &str) -> Option<&'static str> {
    None
}
