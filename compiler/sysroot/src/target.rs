use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::{LazyLock, Mutex};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
pub enum Arch {
    X86_64,
    Aarch64,
    Riscv64,
    Wasm32,
}

impl Arch {
    pub fn as_str(self) -> &'static str {
        match self {
            Arch::X86_64 => "x86_64",
            Arch::Aarch64 => "aarch64",
            Arch::Riscv64 => "riscv64",
            Arch::Wasm32 => "wasm32",
        }
    }
}

impl fmt::Display for Arch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
pub enum Os {
    Linux,
    Windows,
    Macos,
    Uefi,
    Wasi,
    FreeBsd,
}

impl Os {
    pub fn as_str(self) -> &'static str {
        match self {
            Os::Linux => "linux",
            Os::Windows => "windows",
            Os::Macos => "macos",
            Os::Uefi => "uefi",
            Os::Wasi => "wasi",
            Os::FreeBsd => "freebsd",
        }
    }
}

impl fmt::Display for Os {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
pub enum Env {
    Gnu,
    Musl,
    Msvc,
    None,
}

impl Env {
    pub fn as_str(self) -> &'static str {
        match self {
            Env::Gnu => "gnu",
            Env::Musl => "musl",
            Env::Msvc => "msvc",
            Env::None => "none",
        }
    }
}

impl fmt::Display for Env {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
pub enum ObjectFormat {
    Elf,
    Coff,
    MachO,
    Wasm,
}

impl ObjectFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            ObjectFormat::Elf => "elf",
            ObjectFormat::Coff => "coff",
            ObjectFormat::MachO => "macho",
            ObjectFormat::Wasm => "wasm",
        }
    }
}

impl fmt::Display for ObjectFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
pub enum Libc {
    Glibc,
    Musl,
    Msvcrt,
    Mingw,
    LibSystem,
    None,
}

impl Libc {
    pub fn as_str(self) -> &'static str {
        match self {
            Libc::Glibc => "glibc",
            Libc::Musl => "musl",
            Libc::Msvcrt => "msvcrt",
            Libc::Mingw => "mingw",
            Libc::LibSystem => "libsystem",
            Libc::None => "none",
        }
    }
}

impl fmt::Display for Libc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
pub enum Tier {
    One,
    Two,
    Three,
}

impl Tier {
    pub fn as_str(self) -> &'static str {
        match self {
            Tier::One => "1",
            Tier::Two => "2",
            Tier::Three => "3",
        }
    }

    pub fn from_number(number: u8) -> Option<Tier> {
        match number {
            1 => Some(Tier::One),
            2 => Some(Tier::Two),
            3 => Some(Tier::Three),
            _ => None,
        }
    }
}

impl fmt::Display for Tier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
pub enum Endian {
    Little,
    Big,
}

impl Endian {
    pub fn as_str(self) -> &'static str {
        match self {
            Endian::Little => "little",
            Endian::Big => "big",
        }
    }
}

impl fmt::Display for Endian {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
pub enum Capability {
    Alloc,
    Console,
    Clock,
    Fs,
    Log,
    Net,
    Process,
    Random,
    Threads,
    Firmware,
}

impl Capability {
    pub fn as_str(self) -> &'static str {
        match self {
            Capability::Alloc => "alloc",
            Capability::Console => "console",
            Capability::Clock => "clock",
            Capability::Fs => "fs",
            Capability::Log => "log",
            Capability::Net => "net",
            Capability::Process => "process",
            Capability::Random => "random",
            Capability::Threads => "threads",
            Capability::Firmware => "firmware",
        }
    }
}

impl fmt::Display for Capability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for Capability {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "alloc" => Ok(Capability::Alloc),
            "console" => Ok(Capability::Console),
            "clock" => Ok(Capability::Clock),
            "fs" => Ok(Capability::Fs),
            "log" => Ok(Capability::Log),
            "net" => Ok(Capability::Net),
            "process" => Ok(Capability::Process),
            "random" => Ok(Capability::Random),
            "threads" => Ok(Capability::Threads),
            "firmware" => Ok(Capability::Firmware),
            _ => Err(format!("unknown capability `{s}`")),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CpuSpec {
    pub model: String,
    pub features: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeSpec {
    pub abi: u32,
    pub file: String,
    pub sha256: Option<String>,
}

impl RuntimeSpec {
    fn native() -> RuntimeSpec {
        RuntimeSpec {
            abi: 1,
            file: "libruntime_native.a".to_string(),
            sha256: None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LinkMap {
    pub search: Vec<String>,
    pub crt: BTreeMap<String, Vec<String>>,
    pub crt_end: BTreeMap<String, Vec<String>>,
    pub libs: BTreeMap<String, Option<String>>,
    pub builtins: Option<String>,
    pub runtime: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub triple: String,
    pub arch: Arch,
    pub os: Os,
    pub env: Env,
    pub object: ObjectFormat,
    pub pointer_width: u8,
    pub endian: Endian,
    pub libc: Libc,
    pub tier: Tier,
    pub capabilities: BTreeSet<Capability>,
    pub prelude: String,
    pub runtime: RuntimeSpec,
    pub link: LinkMap,
    pub loader: Option<String>,
    pub cpu: CpuSpec,
    pub kernel_min: Option<String>,
    pub glibc_baseline: Option<String>,
}

impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.triple)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TargetError {
    message: String,
}

impl TargetError {
    fn new(message: impl Into<String>) -> TargetError {
        TargetError {
            message: message.into(),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for TargetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for TargetError {}

const LINUX_CAPABILITIES: &[Capability] = &[
    Capability::Alloc,
    Capability::Console,
    Capability::Clock,
    Capability::Fs,
    Capability::Log,
    Capability::Net,
    Capability::Process,
    Capability::Random,
    Capability::Threads,
];

const WINDOWS_CAPABILITIES: &[Capability] = &[
    Capability::Alloc,
    Capability::Console,
    Capability::Clock,
    Capability::Fs,
    Capability::Log,
    Capability::Net,
    Capability::Process,
    Capability::Random,
    Capability::Threads,
];

const MACOS_CAPABILITIES: &[Capability] = &[
    Capability::Alloc,
    Capability::Console,
    Capability::Clock,
    Capability::Fs,
    Capability::Log,
    Capability::Net,
    Capability::Process,
    Capability::Random,
    Capability::Threads,
];

const UEFI_CAPABILITIES: &[Capability] = &[
    Capability::Alloc,
    Capability::Console,
    Capability::Clock,
    Capability::Firmware,
];

const WASI_CAPABILITIES: &[Capability] = &[
    Capability::Alloc,
    Capability::Console,
    Capability::Clock,
    Capability::Fs,
    Capability::Log,
    Capability::Random,
];

static ALL: LazyLock<Vec<Target>> = LazyLock::new(|| {
    let mut targets = vec![
        descriptor(
            "aarch64-apple-darwin",
            Arch::Aarch64,
            Os::Macos,
            Env::None,
            ObjectFormat::MachO,
            Libc::LibSystem,
            Tier::Two,
            MACOS_CAPABILITIES,
            "generic",
            None,
            &["System"],
        ),
        descriptor(
            "aarch64-unknown-linux-gnu",
            Arch::Aarch64,
            Os::Linux,
            Env::Gnu,
            ObjectFormat::Elf,
            Libc::Glibc,
            Tier::Two,
            LINUX_CAPABILITIES,
            "generic",
            Some("/lib/ld-linux-aarch64.so.1"),
            &["c", "m", "pthread", "dl", "rt"],
        ),
        descriptor(
            "aarch64-unknown-linux-musl",
            Arch::Aarch64,
            Os::Linux,
            Env::Musl,
            ObjectFormat::Elf,
            Libc::Musl,
            Tier::Two,
            LINUX_CAPABILITIES,
            "generic",
            Some("/lib/ld-musl-aarch64.so.1"),
            &["c", "m", "pthread", "dl", "rt"],
        ),
        descriptor(
            "wasm32-unknown-wasi",
            Arch::Wasm32,
            Os::Wasi,
            Env::None,
            ObjectFormat::Wasm,
            Libc::None,
            Tier::Three,
            WASI_CAPABILITIES,
            "generic",
            None,
            &[],
        ),
        descriptor(
            "x86_64-apple-darwin",
            Arch::X86_64,
            Os::Macos,
            Env::None,
            ObjectFormat::MachO,
            Libc::LibSystem,
            Tier::Two,
            MACOS_CAPABILITIES,
            "x86-64",
            None,
            &["System"],
        ),
        descriptor(
            "x86_64-pc-windows-gnu",
            Arch::X86_64,
            Os::Windows,
            Env::Gnu,
            ObjectFormat::Coff,
            Libc::Mingw,
            Tier::Two,
            WINDOWS_CAPABILITIES,
            "x86-64",
            None,
            &["mingw32", "mingwex", "msvcrt", "ucrt", "kernel32"],
        ),
        descriptor(
            "x86_64-unknown-linux-gnu",
            Arch::X86_64,
            Os::Linux,
            Env::Gnu,
            ObjectFormat::Elf,
            Libc::Glibc,
            Tier::One,
            LINUX_CAPABILITIES,
            "x86-64",
            Some("/lib64/ld-linux-x86-64.so.2"),
            &["c", "m", "pthread", "dl", "rt"],
        ),
        descriptor(
            "x86_64-unknown-linux-musl",
            Arch::X86_64,
            Os::Linux,
            Env::Musl,
            ObjectFormat::Elf,
            Libc::Musl,
            Tier::Two,
            LINUX_CAPABILITIES,
            "x86-64",
            Some("/lib/ld-musl-x86_64.so.1"),
            &["c", "m", "pthread", "dl", "rt"],
        ),
        descriptor(
            "x86_64-unknown-uefi",
            Arch::X86_64,
            Os::Uefi,
            Env::None,
            ObjectFormat::Coff,
            Libc::None,
            Tier::Three,
            UEFI_CAPABILITIES,
            "x86-64",
            None,
            &[],
        ),
    ];
    for target in &mut targets {
        if target.os == Os::Linux && target.libc == Libc::Glibc {
            target.glibc_baseline = Some("2.17".to_string());
        }
        if target.triple == "wasm32-unknown-wasi" {
            target.prelude = "@std/wasi".to_string();
        }
        if target.triple == "x86_64-unknown-uefi" {
            target.prelude = "@std/uefi".to_string();
        }
    }
    targets.sort_by(|a, b| a.triple.cmp(&b.triple));
    targets
});

fn descriptor(
    triple: &str,
    arch: Arch,
    os: Os,
    env: Env,
    object: ObjectFormat,
    libc: Libc,
    tier: Tier,
    capabilities: &[Capability],
    cpu: &str,
    loader: Option<&str>,
    libs: &[&str],
) -> Target {
    let mut link = LinkMap::default();
    for lib in libs {
        link.libs.insert((*lib).to_string(), None);
    }
    Target {
        triple: triple.to_string(),
        arch,
        os,
        env,
        object,
        pointer_width: if arch == Arch::Wasm32 { 32 } else { 64 },
        endian: Endian::Little,
        libc,
        tier,
        capabilities: capabilities.iter().copied().collect(),
        prelude: "@std/prelude".to_string(),
        runtime: RuntimeSpec::native(),
        link,
        loader: loader.map(str::to_string),
        cpu: CpuSpec {
            model: cpu.to_string(),
            features: String::new(),
        },
        kernel_min: None,
        glibc_baseline: None,
    }
}

impl Target {
    pub fn all() -> &'static [Target] {
        &ALL
    }

    /// Linker entry symbol for a PE image of this target.
    ///
    /// UEFI images enter at `efi_main`, which the target runtime archive
    /// defines; Windows images keep the MinGW CRT entry.
    pub fn pe_entry(&self) -> &'static str {
        match self.os {
            Os::Uefi => "efi_main",
            _ => "mainCRTStartup",
        }
    }

    /// PE subsystem for this target: 10 (`IMAGE_SUBSYSTEM_EFI_APPLICATION`)
    /// for UEFI, 3 (`IMAGE_SUBSYSTEM_WINDOWS_CUI`) otherwise.
    pub fn pe_subsystem(&self) -> u16 {
        match self.os {
            Os::Uefi => 10,
            _ => 3,
        }
    }

    pub fn parse(triple: &str) -> Result<Target, TargetError> {
        ALL.iter()
            .find(|target| target.triple == triple)
            .cloned()
            .ok_or_else(|| {
                let accepted = accepted_triples().join(", ");
                TargetError::new(format!(
                    "unsupported target `{triple}`; accepted targets: {accepted}"
                ))
            })
    }

    pub fn host() -> Result<Target, TargetError> {
        let arch = match std::env::consts::ARCH {
            "x86_64" => Arch::X86_64,
            "aarch64" => Arch::Aarch64,
            other => {
                return Err(TargetError::new(format!(
                    "no built-in target descriptor for host architecture `{other}`"
                )));
            }
        };
        let triple = match (arch, std::env::consts::OS) {
            (Arch::X86_64, "linux") => {
                let env = linux_host_env()?;
                match env {
                    Env::Musl => "x86_64-unknown-linux-musl",
                    _ => "x86_64-unknown-linux-gnu",
                }
            }
            (Arch::Aarch64, "linux") => {
                let env = linux_host_env()?;
                match env {
                    Env::Musl => "aarch64-unknown-linux-musl",
                    _ => "aarch64-unknown-linux-gnu",
                }
            }
            (Arch::X86_64, "windows") => "x86_64-pc-windows-gnu",
            (Arch::X86_64, "macos") => "x86_64-apple-darwin",
            (Arch::Aarch64, "macos") => "aarch64-apple-darwin",
            (arch, os) => {
                return Err(TargetError::new(format!(
                    "no built-in target descriptor for host `{arch}-{os}`"
                )));
            }
        };
        Target::parse(triple)
    }
}

fn linux_host_env() -> Result<Env, TargetError> {
    match std::env::var("RNX_TARGET_ENV") {
        Ok(value) if value == "musl" => Ok(Env::Musl),
        Ok(value) if value.is_empty() || value == "gnu" => Ok(Env::Gnu),
        Ok(value) => Err(TargetError::new(format!(
            "RNX_TARGET_ENV=`{value}` has no Linux host target descriptor"
        ))),
        Err(_) => Ok(Env::Gnu),
    }
}

pub fn accepted_triples() -> Vec<&'static str> {
    Target::all()
        .iter()
        .map(|target| target.triple.as_str())
        .collect()
}

static ACTIVE_TARGET: Mutex<Option<Target>> = Mutex::new(None);

pub fn set_active_target(target: Target) {
    *ACTIVE_TARGET.lock().unwrap_or_else(|e| e.into_inner()) = Some(target);
}

pub fn active_target() -> Target {
    let mut active = ACTIVE_TARGET.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(target) = active.as_ref() {
        return target.clone();
    }
    // The descriptor table is a fixed non-empty list, so the fallback only
    // covers hosts without a built-in descriptor.
    let target = Target::host().unwrap_or_else(|_| Target::all()[0].clone());
    *active = Some(target.clone());
    target
}

pub fn select_target(
    requested: Option<&str>,
    project_targets: &[String],
) -> Result<Target, TargetError> {
    if let Some(triple) = requested {
        if !project_targets.is_empty() && !project_targets.iter().any(|t| t == triple) {
            return Err(TargetError::new(format!(
                "target `{triple}` is not listed in `project.targets`; allowed targets: {}",
                project_targets.join(", ")
            )));
        }
        return Target::parse(triple);
    }
    match project_targets.len() {
        0 => Target::host(),
        1 => Target::parse(&project_targets[0]),
        _ => Err(TargetError::new(format!(
            "`project.targets` lists multiple targets ({}); pass --target to choose one",
            project_targets.join(", ")
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn descriptors_parse_and_round_trip() {
        for target in Target::all() {
            let parsed = Target::parse(&target.triple).expect("built-in descriptor parses");
            assert_eq!(parsed, *target);
            assert_eq!(parsed.to_string(), parsed.triple);
        }
    }

    #[test]
    fn descriptors_are_sorted_by_triple() {
        let triples: Vec<&str> = Target::all().iter().map(|t| t.triple.as_str()).collect();
        let mut sorted = triples.clone();
        sorted.sort();
        assert_eq!(triples, sorted);
    }

    #[test]
    fn unknown_triple_names_the_accepted_list() {
        let err = Target::parse("x86_64-pc-windows-msvc").expect_err("msvc is not built in");
        assert!(err.to_string().contains("x86_64-pc-windows-msvc"));
        for triple in accepted_triples() {
            assert!(err.to_string().contains(triple), "missing {triple}");
        }
    }

    #[test]
    fn host_returns_a_supported_descriptor() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let host = Target::host().expect("host target is supported");
        assert_eq!(Target::parse(&host.triple).expect("host parses"), host);
    }

    #[test]
    fn musl_host_env_selects_the_musl_descriptor() {
        if std::env::consts::OS != "linux" {
            return;
        }
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let previous = std::env::var_os("RNX_TARGET_ENV");
        unsafe { std::env::set_var("RNX_TARGET_ENV", "musl") };
        let host = Target::host().expect("musl host target is supported");
        match previous {
            Some(value) => unsafe { std::env::set_var("RNX_TARGET_ENV", value) },
            None => unsafe { std::env::remove_var("RNX_TARGET_ENV") },
        }
        assert_eq!(host.libc, Libc::Musl);
        assert_eq!(host.env, Env::Musl);
        assert!(host.triple.ends_with("-unknown-linux-musl"));
    }

    #[test]
    fn tiers_and_capabilities_match_the_plan() {
        let linux = Target::parse("x86_64-unknown-linux-gnu").unwrap();
        assert_eq!(linux.tier, Tier::One);
        assert_eq!(
            linux
                .capabilities
                .iter()
                .map(|c| c.as_str())
                .collect::<Vec<_>>(),
            vec![
                "alloc", "console", "clock", "fs", "log", "net", "process", "random", "threads"
            ]
        );
        let arm = Target::parse("aarch64-unknown-linux-gnu").unwrap();
        assert_eq!(arm.tier, Tier::Two);
        let uefi = Target::parse("x86_64-unknown-uefi").unwrap();
        assert_eq!(uefi.tier, Tier::Three);
        assert_eq!(
            uefi.capabilities
                .iter()
                .map(|c| c.as_str())
                .collect::<Vec<_>>(),
            vec!["alloc", "console", "clock", "firmware"]
        );
        let wasi = Target::parse("wasm32-unknown-wasi").unwrap();
        assert_eq!(wasi.tier, Tier::Three);
        assert_eq!(
            wasi.capabilities
                .iter()
                .map(|c| c.as_str())
                .collect::<Vec<_>>(),
            vec!["alloc", "console", "clock", "fs", "log", "random"]
        );
    }

    #[test]
    fn select_target_covers_every_project_case() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let requested = Some("aarch64-unknown-linux-musl");
        let listed = vec!["aarch64-unknown-linux-musl".to_string()];
        assert_eq!(
            select_target(requested, &listed).unwrap().triple,
            "aarch64-unknown-linux-musl"
        );

        let unlisted = select_target(requested, &["x86_64-unknown-linux-gnu".to_string()])
            .expect_err("unlisted target is rejected");
        assert!(unlisted.to_string().contains("aarch64-unknown-linux-musl"));
        assert!(unlisted.to_string().contains("x86_64-unknown-linux-gnu"));

        let single = select_target(None, &["x86_64-unknown-uefi".to_string()]).unwrap();
        assert_eq!(single.triple, "x86_64-unknown-uefi");

        let multiple = select_target(
            None,
            &[
                "x86_64-unknown-linux-gnu".to_string(),
                "x86_64-unknown-uefi".to_string(),
            ],
        )
        .expect_err("multiple targets need an explicit choice");
        assert!(multiple.to_string().contains("x86_64-unknown-linux-gnu"));
        assert!(multiple.to_string().contains("x86_64-unknown-uefi"));
        assert!(multiple.to_string().contains("--target"));

        let host = select_target(None, &[]).unwrap();
        assert_eq!(host, Target::host().unwrap());
    }

    #[test]
    fn built_in_targets_default_to_the_stock_prelude() {
        for target in Target::all() {
            if target.triple == "wasm32-unknown-wasi" {
                assert_eq!(target.prelude, "@std/wasi", "{}", target.triple);
                continue;
            }
            if target.triple == "x86_64-unknown-uefi" {
                assert_eq!(target.prelude, "@std/uefi", "{}", target.triple);
                continue;
            }
            assert_eq!(target.prelude, "@std/prelude", "{}", target.triple);
        }
    }

    #[test]
    fn pe_targets_carry_their_subsystem_and_entry() {
        let uefi = Target::parse("x86_64-unknown-uefi").unwrap();
        assert_eq!(uefi.pe_subsystem(), 10);
        assert_eq!(uefi.pe_entry(), "efi_main");
        let windows = Target::parse("x86_64-pc-windows-gnu").unwrap();
        assert_eq!(windows.pe_subsystem(), 3);
        assert_eq!(windows.pe_entry(), "mainCRTStartup");
    }

    #[test]
    fn active_target_is_process_global() {
        let custom = Target::parse("wasm32-unknown-wasi").unwrap();
        set_active_target(custom.clone());
        assert_eq!(active_target(), custom);
        let host = Target::host().unwrap();
        set_active_target(host.clone());
        assert_eq!(active_target(), host);
    }
}
