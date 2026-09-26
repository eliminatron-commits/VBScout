//! Collector entry point.
//!
//! Exit codes: 0 = result written, 1 = error (nothing or no complete result
//! written), 2 = invalid command line.

use std::ffi::OsString;
use std::path::Path;
use std::process::ExitCode;
use std::time::Instant;

use time::OffsetDateTime;
use vbs_collector::cli::{self, CliErrorKind, Command, PathProblem};
use vbs_collector::console::{Console, language_list};
use vbs_collector::output::{self, OutputError};
use vbs_collector::{engine, modules, platform};
use vbs_i18n::{Lang, t_args};

fn main() -> ExitCode {
    let mut args = std::env::args_os();
    let program = args
        .next()
        .as_deref()
        .and_then(|arg0| Path::new(arg0).file_name())
        .map_or_else(|| OsString::from("vbscout-collector"), |name| name.to_os_string());
    match cli::parse(args) {
        Ok(Command::Help(lang)) => {
            println!("{}", banner(lang));
            println!();
            let program = program.to_string_lossy();
            let extension = vbs_core::file_extension();
            let languages = language_list();
            println!(
                "{}",
                t_args(
                    lang,
                    "collector.help",
                    &[("program", &program), ("extension", &extension), ("languages", &languages)]
                )
            );
            ExitCode::SUCCESS
        }
        Ok(Command::Version) => {
            println!("{} {}", vbs_config::product().name, env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Ok(Command::Scan(options)) => run(&options),
        Err(error) => {
            let console = Console::new(error.lang, false);
            match error.kind {
                CliErrorKind::Usage(message) => console.error("collector.error.usage", &[("message", &message)]),
                CliErrorKind::UnsupportedLanguage(tag) => {
                    let languages = language_list();
                    console.error("collector.error.language", &[("tag", &tag), ("languages", &languages)]);
                }
            }
            ExitCode::from(2)
        }
    }
}

fn banner(lang: Lang) -> String {
    t_args(lang, "collector.banner", &[("version", &env!("CARGO_PKG_VERSION"))])
}

fn run(options: &cli::Options) -> ExitCode {
    let console = Console::new(options.lang, options.quiet);
    let plan = match cli::plan(options) {
        Ok(plan) => plan,
        Err(problem) => {
            match problem {
                PathProblem::Missing(path) => {
                    console.error("collector.error.pathMissing", &[("path", &path.display())])
                }
                PathProblem::NetworkPath(path) => {
                    console.error("collector.error.networkPath", &[("path", &path.display())]);
                }
                PathProblem::NotNetworkPath(path) => {
                    console.error("collector.error.notNetworkPath", &[("path", &path.display())]);
                }
            }
            return ExitCode::from(2);
        }
    };

    let host = platform::host();
    // Decide the output path before scanning, so a wrong --out fails in seconds, not after the scan.
    let now = OffsetDateTime::now_utc();
    let target =
        match output::resolve(options.out.as_deref(), &host.machine.hostname, now, host.env.system_root.as_deref()) {
            Ok(target) => target,
            Err(error) => return report_output_error(&console, error),
        };

    if !options.quiet {
        println!("{}", banner(console.lang()));
    }
    if host.supported && !host.env.elevated {
        console.info("collector.limited", &[]);
    }
    let roots: Vec<String> =
        plan.local_roots(&host).iter().chain(&plan.network_paths).map(|root| root.display().to_string()).collect();
    console.info("collector.scanning", &[("roots", &roots.join(", "))]);

    let clock = Instant::now();
    let result = engine::scan(&plan, &host, &modules::all());
    let bytes = match vbs_core::to_bytes(&result) {
        Ok(bytes) => bytes,
        Err(error) => {
            console.error("collector.error.write", &[("path", &target.display()), ("message", &error)]);
            return ExitCode::FAILURE;
        }
    };
    if let Err(error) = output::write_result(&target, &bytes) {
        return report_output_error(&console, error);
    }

    let entries: u64 = result.coverage.sources.iter().map(|source| source.entries).sum();
    let seconds = format!("{:.1}", clock.elapsed().as_secs_f64());
    let findings = result.findings.len();
    console.info("collector.done", &[("seconds", &seconds), ("entries", &entries), ("findings", &findings)]);
    let shown = std::path::absolute(&target).unwrap_or(target);
    console.info("collector.written", &[("path", &shown.display())]);
    ExitCode::SUCCESS
}

fn report_output_error(console: &Console, error: OutputError) -> ExitCode {
    match error {
        OutputError::Exists(path) => console.error("collector.error.outputExists", &[("path", &path.display())]),
        OutputError::DirMissing(path) => {
            console.error("collector.error.outputDirMissing", &[("path", &path.display())]);
        }
        OutputError::SystemFolder(path) => {
            console.error("collector.error.systemFolder", &[("path", &path.display())]);
            return ExitCode::from(2);
        }
        OutputError::Io(path, error) => {
            console.error("collector.error.write", &[("path", &path.display()), ("message", &error)]);
        }
    }
    ExitCode::FAILURE
}
