//! The logic for the Wasmer CLI tool.

#[cfg(target_os = "linux")]
use crate::commands::Binfmt;
#[cfg(feature = "compiler")]
use crate::commands::Compile;
#[cfg(any(feature = "static-artifact-create", feature = "wasmer-artifact-create"))]
use crate::commands::CreateExe;
#[cfg(feature = "static-artifact-create")]
use crate::commands::CreateObj;
#[cfg(feature = "wast")]
use crate::commands::Wast;
use crate::commands::{Cache, Config, Inspect, Run, RunWithoutFile, SelfUpdate, Validate};
use crate::error::PrettyError;
use clap::{CommandFactory, ErrorKind, Parser};
use spinner::SpinnerHandle;
use std::fmt;
use wasmer_registry::{get_all_local_packages, PackageDownloadInfo};

#[derive(Parser, Debug)]
#[cfg_attr(
    not(feature = "headless"),
    clap(
        name = "wasmer",
        about = "WebAssembly standalone runtime.",
        version,
        author
    )
)]
#[cfg_attr(
    feature = "headless",
    clap(
        name = "wasmer-headless",
        about = "WebAssembly standalone runtime (headless).",
        version,
        author
    )
)]
/// The options for the wasmer Command Line Interface
enum WasmerCLIOptions {
    /// List all locally installed packages
    #[clap(name = "list")]
    List,

    /// Run a WebAssembly file. Formats accepted: wasm, wat
    #[clap(name = "run")]
    Run(Run),

    /// Wasmer cache
    #[clap(subcommand, name = "cache")]
    Cache(Cache),

    /// Validate a WebAssembly binary
    #[clap(name = "validate")]
    Validate(Validate),

    /// Compile a WebAssembly binary
    #[cfg(feature = "compiler")]
    #[clap(name = "compile")]
    Compile(Compile),

    /// Compile a WebAssembly binary into a native executable
    ///
    /// To use, you need to set the `WASMER_DIR` environment variable
    /// to the location of your Wasmer installation. This will probably be `~/.wasmer`. It
    /// should include a `lib`, `include` and `bin` subdirectories. To create an executable
    /// you will need `libwasmer`, so by setting `WASMER_DIR` the CLI knows where to look for
    /// header files and libraries.
    ///
    /// Example usage:
    ///
    /// ```text
    /// $ # in two lines:
    /// $ export WASMER_DIR=/home/user/.wasmer/
    /// $ wasmer create-exe qjs.wasm -o qjs.exe # or in one line:
    /// $ WASMER_DIR=/home/user/.wasmer/ wasmer create-exe qjs.wasm -o qjs.exe
    /// $ file qjs.exe
    /// qjs.exe: ELF 64-bit LSB pie executable, x86-64 ...
    /// ```
    ///
    /// ## Cross-compilation
    ///
    /// Accepted target triple values must follow the
    /// ['target_lexicon'](https://crates.io/crates/target-lexicon) crate format.
    ///
    /// The recommended targets we try to support are:
    ///
    /// - "x86_64-linux-gnu"
    /// - "aarch64-linux-gnu"
    /// - "x86_64-apple-darwin"
    /// - "arm64-apple-darwin"
    #[cfg(any(feature = "static-artifact-create", feature = "wasmer-artifact-create"))]
    #[clap(name = "create-exe", verbatim_doc_comment)]
    CreateExe(CreateExe),

    /// Compile a WebAssembly binary into an object file
    ///
    /// To use, you need to set the `WASMER_DIR` environment variable to the location of your
    /// Wasmer installation. This will probably be `~/.wasmer`. It should include a `lib`,
    /// `include` and `bin` subdirectories. To create an object you will need `libwasmer`, so by
    /// setting `WASMER_DIR` the CLI knows where to look for header files and libraries.
    ///
    /// Example usage:
    ///
    /// ```text
    /// $ # in two lines:
    /// $ export WASMER_DIR=/home/user/.wasmer/
    /// $ wasmer create-obj qjs.wasm --object-format symbols -o qjs.obj # or in one line:
    /// $ WASMER_DIR=/home/user/.wasmer/ wasmer create-exe qjs.wasm --object-format symbols -o qjs.obj
    /// $ file qjs.obj
    /// qjs.obj: ELF 64-bit LSB relocatable, x86-64 ...
    /// ```
    ///
    /// ## Cross-compilation
    ///
    /// Accepted target triple values must follow the
    /// ['target_lexicon'](https://crates.io/crates/target-lexicon) crate format.
    ///
    /// The recommended targets we try to support are:
    ///
    /// - "x86_64-linux-gnu"
    /// - "aarch64-linux-gnu"
    /// - "x86_64-apple-darwin"
    /// - "arm64-apple-darwin"
    #[cfg(feature = "static-artifact-create")]
    #[structopt(name = "create-obj", verbatim_doc_comment)]
    CreateObj(CreateObj),

    /// Get various configuration information needed
    /// to compile programs which use Wasmer
    #[clap(name = "config")]
    Config(Config),

    /// Update wasmer to the latest version
    #[clap(name = "self-update")]
    SelfUpdate(SelfUpdate),

    /// Inspect a WebAssembly file
    #[clap(name = "inspect")]
    Inspect(Inspect),

    /// Run spec testsuite
    #[cfg(feature = "wast")]
    #[clap(name = "wast")]
    Wast(Wast),

    /// Unregister and/or register wasmer as binfmt interpreter
    #[cfg(target_os = "linux")]
    #[clap(name = "binfmt")]
    Binfmt(Binfmt),
}

impl WasmerCLIOptions {
    fn execute(&self) -> Result<(), anyhow::Error> {
        match self {
            Self::Run(options) => options.execute(),
            Self::SelfUpdate(options) => options.execute(),
            Self::Cache(cache) => cache.execute(),
            Self::Validate(validate) => validate.execute(),
            #[cfg(feature = "compiler")]
            Self::Compile(compile) => compile.execute(),
            #[cfg(any(feature = "static-artifact-create", feature = "wasmer-artifact-create"))]
            Self::CreateExe(create_exe) => create_exe.execute(),
            #[cfg(feature = "static-artifact-create")]
            Self::CreateObj(create_obj) => create_obj.execute(),
            Self::Config(config) => config.execute(),
            Self::Inspect(inspect) => inspect.execute(),
            Self::List => print_packages(),
            #[cfg(feature = "wast")]
            Self::Wast(wast) => wast.execute(),
            #[cfg(target_os = "linux")]
            Self::Binfmt(binfmt) => binfmt.execute(),
        }
    }
}

/// The main function for the Wasmer CLI tool.
pub fn wasmer_main() {
    // We allow windows to print properly colors
    #[cfg(windows)]
    colored::control::set_virtual_terminal(true).unwrap();

    PrettyError::report(wasmer_main_inner())
}

fn wasmer_main_inner() -> Result<(), anyhow::Error> {
    // We try to run wasmer with the normal arguments.
    // Eg. `wasmer <SUBCOMMAND>`
    // In case that fails, we fallback trying the Run subcommand directly.
    // Eg. `wasmer myfile.wasm --dir=.`
    //
    // In case we've been run as wasmer-binfmt-interpreter myfile.wasm args,
    // we assume that we're registered via binfmt_misc
    let args = std::env::args().collect::<Vec<_>>();
    let binpath = args.get(0).map(|s| s.as_ref()).unwrap_or("");

    let firstarg = args.get(1).map(|s| s.as_str());
    let secondarg = args.get(2).map(|s| s.as_str());

    match (firstarg, secondarg) {
        (None, _) | (Some("help"), _) | (Some("--help"), _) => {
            return print_help(true);
        }
        (Some("-h"), _) => {
            return print_help(false);
        }
        (Some("-vV"), _)
        | (Some("version"), Some("--verbose"))
        | (Some("--version"), Some("--verbose")) => {
            return print_version(true);
        }

        (Some("-v"), _) | (Some("-V"), _) | (Some("version"), _) | (Some("--version"), _) => {
            return print_version(false);
        }
        _ => {}
    }

    let command = args.get(1);
    let options = if cfg!(target_os = "linux") && binpath.ends_with("wasmer-binfmt-interpreter") {
        WasmerCLIOptions::Run(Run::from_binfmt_args())
    } else {
        match command.unwrap_or(&"".to_string()).as_ref() {
            "cache" | "compile" | "config" | "create-exe" | "help" | "inspect" | "run"
            | "self-update" | "validate" | "wast" | "binfmt" | "list" => WasmerCLIOptions::parse(),
            _ => {
                WasmerCLIOptions::try_parse_from(args.iter()).unwrap_or_else(|e| {
                    match e.kind() {
                        // This fixes a issue that:
                        // 1. Shows the version twice when doing `wasmer -V`
                        // 2. Shows the run help (instead of normal help) when doing `wasmer --help`
                        ErrorKind::DisplayVersion | ErrorKind::DisplayHelp => e.exit(),
                        _ => WasmerCLIOptions::Run(Run::parse()),
                    }
                })
            }
        }
    };

    // Check if the file is a package name
    if let WasmerCLIOptions::Run(r) = &options {
        return try_run_package_or_file(&args, r);
    }

    options.execute()
}

fn try_run_package_or_file(args: &[String], r: &Run) -> Result<(), anyhow::Error> {
    // Check "r.path" is a file or a package / command name
    if r.path.exists() {
        if r.path.is_dir() && r.path.join("wapm.toml").exists() {
            let mut args_without_package = args.to_vec();
            if args_without_package.get(1) == Some(&format!("{}", r.path.display())) {
                let _ = args_without_package.remove(1);
            } else if args_without_package.get(2) == Some(&format!("{}", r.path.display())) {
                let _ = args_without_package.remove(1);
                let _ = args_without_package.remove(1);
            }
            return RunWithoutFile::try_parse_from(args_without_package.iter())?
                .into_run_args(r.path.clone(), r.command_name.as_deref())?
                .execute();
        }
        return r.execute();
    }

    let package = format!("{}", r.path.display());

    let mut is_fake_sv = false;
    let mut sv = match split_version(&package) {
        Ok(o) => o,
        Err(_) => {
            let mut fake_sv = SplitVersion {
                original: package.to_string(),
                registry: None,
                package: package.to_string(),
                version: None,
                command: None,
            };
            is_fake_sv = true;
            match try_lookup_command(&mut fake_sv) {
                Ok(o) => SplitVersion {
                    original: format!("{}@{}", o.package, o.version),
                    registry: None,
                    package: o.package,
                    version: Some(o.version),
                    command: r.command_name.clone(),
                },
                Err(e) => {
                    return Err(
                        anyhow::anyhow!("No package for command {package:?} found, file {package:?} not found either")
                        .context(e)
                        .context(anyhow::anyhow!("{}", r.path.display()))
                    );
                }
            }
        }
    };

    if sv.command.is_none() {
        sv.command = r.command_name.clone();
    }

    if sv.command.is_none() && is_fake_sv {
        sv.command = Some(package);
    }

    let mut package_download_info = None;
    if !sv.package.contains('/') {
        if let Ok(o) = try_lookup_command(&mut sv) {
            package_download_info = Some(o);
        }
    }

    match try_execute_local_package(args, &sv) {
        Ok(o) => return Ok(o),
        Err(ExecuteLocalPackageError::DuringExec(e)) => return Err(e),
        _ => {}
    }

    println!("finding local package {} failed", sv);
    // else: local package not found - try to download and install package
    try_autoinstall_package(args, &sv, package_download_info, r.force_install)
}

fn try_lookup_command(sv: &mut SplitVersion) -> Result<PackageDownloadInfo, anyhow::Error> {
    use std::io::Write;
    let sp = start_spinner(format!("Looking up command {} ...", sv.package));

    for registry in wasmer_registry::get_all_available_registries().unwrap_or_default() {
        let result = wasmer_registry::query_command_from_registry(&registry, &sv.package);
        print!("\r");
        let _ = std::io::stdout().flush();
        let command = sv.package.clone();
        if let Ok(o) = result {
            sv.package = o.package.clone();
            sv.version = Some(o.version.clone());
            sv.command = Some(command);
            return Ok(o);
        }
    }

    sp.close();
    print!("\r");
    let _ = std::io::stdout().flush();
    Err(anyhow::anyhow!("command {sv} not found"))
}

// We need to distinguish between errors that happen
// before vs. during execution
enum ExecuteLocalPackageError {
    BeforeExec(anyhow::Error),
    DuringExec(anyhow::Error),
}

fn try_execute_local_package(
    args: &[String],
    sv: &SplitVersion,
) -> Result<(), ExecuteLocalPackageError> {
    let package = wasmer_registry::get_local_package(None, &sv.package, sv.version.as_deref())
        .ok_or_else(|| {
            ExecuteLocalPackageError::BeforeExec(anyhow::anyhow!("no local package {sv:?} found"))
        })?;

    let package_dir = package
        .get_path()
        .map_err(|e| ExecuteLocalPackageError::BeforeExec(anyhow::anyhow!("{e}")))?;

    // Try finding the local package
    let mut args_without_package = args.to_vec();

    // remove either "run" or $package
    args_without_package.remove(1);

    // "wasmer package arg1 arg2" => "wasmer arg1 arg2"
    if (args_without_package.get(1).is_some() && args_without_package[1].starts_with(&sv.original))
        || (sv.command.is_some() && args_without_package[1].ends_with(sv.command.as_ref().unwrap()))
    {
        args_without_package.remove(1);
    }

    RunWithoutFile::try_parse_from(args_without_package.iter())
        .map_err(|e| ExecuteLocalPackageError::DuringExec(e.into()))?
        .into_run_args(package_dir, sv.command.as_deref())
        .map_err(ExecuteLocalPackageError::DuringExec)?
        .execute()
        .map_err(|e| ExecuteLocalPackageError::DuringExec(e.context(anyhow::anyhow!("{}", sv))))
}

fn try_autoinstall_package(
    args: &[String],
    sv: &SplitVersion,
    package: Option<PackageDownloadInfo>,
    force_install: bool,
) -> Result<(), anyhow::Error> {
    use std::io::Write;
    let sp = start_spinner(format!("Installing package {} ...", sv.package));
    let v = sv.version.as_deref();
    let result = wasmer_registry::install_package(
        sv.registry.as_deref(),
        &sv.package,
        v,
        package,
        force_install,
    );
    sp.close();
    print!("\r");
    let _ = std::io::stdout().flush();
    let (_, package_dir) = match result {
        Ok(o) => o,
        Err(e) => {
            return Err(anyhow::anyhow!("{e}"));
        }
    };

    // Try auto-installing the remote package
    let mut args_without_package = args.to_vec();
    args_without_package.remove(1);

    let mut run_args = RunWithoutFile::try_parse_from(args_without_package.iter())?;
    run_args.command_name = sv.command.clone();

    run_args
        .into_run_args(package_dir, sv.command.as_deref())?
        .execute()
}

fn start_spinner(msg: String) -> SpinnerHandle {
    spinner::SpinnerBuilder::new(msg)
        .spinner(vec![
            "⣾", "⣽", "⣻", "⢿", "⡿", "⣟", "⣯", "⣷", " ", "⠁", "⠂", "⠄", "⡀", "⢀", "⠠", "⠐", "⠈",
        ])
        .start()
}

#[derive(Debug, Clone, PartialEq, Default)]
struct SplitVersion {
    original: String,
    registry: Option<String>,
    package: String,
    version: Option<String>,
    command: Option<String>,
}

impl fmt::Display for SplitVersion {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let version = self.version.as_deref().unwrap_or("latest");
        let command = self
            .command
            .as_ref()
            .map(|s| format!(":{s}"))
            .unwrap_or_default();
        write!(f, "{}@{version}{command}", self.package)
    }
}

#[test]
fn test_split_version() {
    assert_eq!(
        split_version("registry.wapm.io/graphql/python/python").unwrap(),
        SplitVersion {
            registry: Some("https://registry.wapm.io/graphql".to_string()),
            package: "python/python".to_string(),
            version: None,
            command: None,
        }
    );
    assert_eq!(
        split_version("registry.wapm.io/python/python").unwrap(),
        SplitVersion {
            registry: Some("https://registry.wapm.io/graphql".to_string()),
            package: "python/python".to_string(),
            version: None,
            command: None,
        }
    );
    assert_eq!(
        split_version("namespace/name@version:command").unwrap(),
        SplitVersion {
            registry: None,
            package: "namespace/name".to_string(),
            version: Some("version".to_string()),
            command: Some("command".to_string()),
        }
    );
    assert_eq!(
        split_version("namespace/name@version").unwrap(),
        SplitVersion {
            registry: None,
            package: "namespace/name".to_string(),
            version: Some("version".to_string()),
            command: None,
        }
    );
    assert_eq!(
        split_version("namespace/name").unwrap(),
        SplitVersion {
            registry: None,
            package: "namespace/name".to_string(),
            version: None,
            command: None,
        }
    );
    assert_eq!(
        split_version("registry.wapm.io/namespace/name").unwrap(),
        SplitVersion {
            registry: Some("https://registry.wapm.io/graphql".to_string()),
            package: "namespace/name".to_string(),
            version: None,
            command: None,
        }
    );
    assert_eq!(
        format!("{}", split_version("namespace").unwrap_err()),
        "Invalid package version: \"namespace\"".to_string(),
    );
}

fn split_version(s: &str) -> Result<SplitVersion, anyhow::Error> {
    let command = WasmerCLIOptions::command();
    let mut prohibited_package_names = command.get_subcommands().map(|s| s.get_name());

    let re1 = regex::Regex::new(r#"(.*)/(.*)@(.*):(.*)"#).unwrap();
    let re2 = regex::Regex::new(r#"(.*)/(.*)@(.*)"#).unwrap();
    let re3 = regex::Regex::new(r#"(.*)/(.*)"#).unwrap();
    let re4 = regex::Regex::new(r#"(.*)/(.*):(.*)"#).unwrap();

    let mut no_version = false;

    let captures = if re1.is_match(s) {
        re1.captures(s)
            .map(|c| {
                c.iter()
                    .flatten()
                    .map(|m| m.as_str().to_owned())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    } else if re2.is_match(s) {
        re2.captures(s)
            .map(|c| {
                c.iter()
                    .flatten()
                    .map(|m| m.as_str().to_owned())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    } else if re4.is_match(s) {
        no_version = true;
        re4.captures(s)
            .map(|c| {
                c.iter()
                    .flatten()
                    .map(|m| m.as_str().to_owned())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    } else if re3.is_match(s) {
        re3.captures(s)
            .map(|c| {
                c.iter()
                    .flatten()
                    .map(|m| m.as_str().to_owned())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    } else {
        return Err(anyhow::anyhow!("Invalid package version: {s:?}"));
    };

    let mut namespace = match captures.get(1).cloned() {
        Some(s) => s,
        None => {
            return Err(anyhow::anyhow!(
                "Invalid package version: {s:?}: no namespace"
            ))
        }
    };

    let name = match captures.get(2).cloned() {
        Some(s) => s,
        None => return Err(anyhow::anyhow!("Invalid package version: {s:?}: no name")),
    };

    let mut registry = None;
    if namespace.contains('/') {
        let (r, n) = namespace.rsplit_once('/').unwrap();
        let mut real_registry = r.to_string();
        if !real_registry.ends_with("graphql") {
            real_registry = format!("{real_registry}/graphql");
        }
        if !real_registry.contains("://") {
            real_registry = format!("https://{real_registry}");
        }
        registry = Some(real_registry);
        namespace = n.to_string();
    }

    let sv = SplitVersion {
        original: s.to_string(),
        registry,
        package: format!("{namespace}/{name}"),
        version: if no_version {
            None
        } else {
            captures.get(3).cloned()
        },
        command: captures.get(if no_version { 3 } else { 4 }).cloned(),
    };

    let svp = sv.package.clone();
    anyhow::ensure!(
        !prohibited_package_names.any(|s| s == sv.package.trim()),
        "Invalid package name {svp:?}"
    );

    Ok(sv)
}

fn print_packages() -> Result<(), anyhow::Error> {
    use prettytable::{format, row, Table};

    let rows = get_all_local_packages(None)
        .into_iter()
        .filter_map(|pkg| {
            let package_root_path = pkg.get_path().ok()?;
            let (manifest, _) =
                wasmer_registry::get_executable_file_from_path(&package_root_path, None).ok()?;
            let commands = manifest
                .command
                .unwrap_or_default()
                .iter()
                .map(|c| c.get_name())
                .collect::<Vec<_>>()
                .join(" \r\n");

            Some(row![pkg.registry, pkg.name, pkg.version, commands])
        })
        .collect::<Vec<_>>();

    let empty_table = rows.is_empty();
    let mut table = Table::init(rows);
    table.set_titles(row!["Registry", "Package", "Version", "Commands"]);
    table.set_format(*format::consts::FORMAT_NO_LINESEP_WITH_TITLE);
    table.set_format(*format::consts::FORMAT_NO_COLSEP);
    if empty_table {
        table.add_empty_row();
    }
    let _ = table.printstd();

    Ok(())
}

fn print_help(verbose: bool) -> Result<(), anyhow::Error> {
    let mut cmd = WasmerCLIOptions::command();
    if verbose {
        let _ = cmd.print_long_help();
    } else {
        let _ = cmd.print_help();
    }
    Ok(())
}

#[allow(unused_mut, clippy::vec_init_then_push)]
fn print_version(verbose: bool) -> Result<(), anyhow::Error> {
    if !verbose {
        println!("wasmer {}", env!("CARGO_PKG_VERSION"));
    } else {
        println!(
            "wasmer {} ({} {})",
            env!("CARGO_PKG_VERSION"),
            env!("WASMER_BUILD_GIT_HASH_SHORT"),
            env!("WASMER_BUILD_DATE")
        );
        println!("binary: {}", env!("CARGO_PKG_NAME"));
        println!("commit-hash: {}", env!("WASMER_BUILD_GIT_HASH"));
        println!("commit-date: {}", env!("WASMER_BUILD_DATE"));
        println!("host: {}", target_lexicon::HOST);
        println!("compiler: {}", {
            let mut s = Vec::<&'static str>::new();

            #[cfg(feature = "singlepass")]
            s.push("singlepass");
            #[cfg(feature = "cranelift")]
            s.push("cranelift");
            #[cfg(feature = "llvm")]
            s.push("llvm");

            s.join(",")
        });
    }
    Ok(())
}
