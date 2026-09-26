//! Command-line options.
//!
//! `vbscout-collector [--out <file|folder>] [--path <folder>]… [--include-unc <\\server\share>]…
//! [--files-only] [--lang <tag>] [--quiet]`, plus `--help` and `--version`.

use std::ffi::OsString;
use std::path::PathBuf;

use lexopt::prelude::*;
use vbs_i18n::Lang;

use crate::engine::{self, Plan};
use crate::platform;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Scan(Options),
    Help(Lang),
    Version,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    pub out: Option<PathBuf>,
    pub paths: Vec<PathBuf>,
    pub network_paths: Vec<PathBuf>,
    pub files_only: bool,
    pub lang: Lang,
    pub quiet: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            out: None,
            paths: Vec::new(),
            network_paths: Vec::new(),
            files_only: false,
            lang: Lang::En,
            quiet: false,
        }
    }
}

/// A command-line problem, reported in the language chosen so far.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliError {
    pub lang: Lang,
    pub kind: CliErrorKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliErrorKind {
    /// Unknown option, missing value, … (message from the parser, in English).
    Usage(String),
    UnsupportedLanguage(String),
}

/// Parses the arguments after the program name.
pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Command, CliError> {
    let mut parser = lexopt::Parser::from_args(args);
    let mut options = Options::default();
    let (mut help, mut version) = (false, false);
    loop {
        let lang = options.lang;
        let usage = |error: lexopt::Error| CliError { lang, kind: CliErrorKind::Usage(error.to_string()) };
        let Some(arg) = parser.next().map_err(usage)? else { break };
        match arg {
            Long("out") if options.out.is_none() => options.out = Some(parser.value().map_err(usage)?.into()),
            Long("path") => options.paths.push(parser.value().map_err(usage)?.into()),
            Long("include-unc") => options.network_paths.push(parser.value().map_err(usage)?.into()),
            Long("files-only") => options.files_only = true,
            Long("lang") => {
                let tag = parser.value().map_err(usage)?.string().map_err(usage)?;
                options.lang = Lang::from_tag(&tag)
                    .ok_or(CliError { lang, kind: CliErrorKind::UnsupportedLanguage(tag.clone()) })?;
            }
            Long("quiet") | Short('q') => options.quiet = true,
            Long("help") | Short('h') | Short('?') => help = true,
            Long("version") | Short('V') => version = true,
            // Windows habit: `/?`.
            Value(value) if value == "/?" => help = true,
            other => return Err(usage(other.unexpected())),
        }
    }
    Ok(if help {
        Command::Help(options.lang)
    } else if version {
        Command::Version
    } else {
        Command::Scan(options)
    })
}

/// A path option that cannot be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathProblem {
    /// `--path` must name an existing local folder.
    Missing(PathBuf),
    /// `--path` got a network path; network paths need `--include-unc`.
    NetworkPath(PathBuf),
    /// `--include-unc` got something that is not `\\server\share…`.
    NotNetworkPath(PathBuf),
}

/// Turns the options into a scan plan. Network paths are only checked for
/// their form here: whether they are reachable shows in the scan's coverage.
pub fn plan(options: &Options) -> Result<Plan, PathProblem> {
    let mut paths = Vec::new();
    for path in &options.paths {
        if platform::is_network_path(path) {
            return Err(PathProblem::NetworkPath(path.clone()));
        }
        let absolute = std::path::absolute(path).unwrap_or_else(|_| path.clone());
        if !absolute.is_dir() {
            return Err(PathProblem::Missing(path.clone()));
        }
        paths.push(absolute);
    }
    for path in &options.network_paths {
        if !platform::is_network_path(path) {
            return Err(PathProblem::NotNetworkPath(path.clone()));
        }
    }
    Ok(Plan {
        paths,
        network_paths: options.network_paths.clone(),
        system_sources: !options.files_only,
        threads: engine::default_threads(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_args(args: &[&str]) -> Result<Command, CliError> {
        parse(args.iter().map(OsString::from))
    }

    #[test]
    fn parses_scan_options() {
        let command = parse_args(&[
            "--out",
            "D:\\results",
            "--path",
            "C:\\Scripts",
            "--include-unc",
            "\\\\srv\\netlogon",
            "--files-only",
            "--lang",
            "de",
            "-q",
        ])
        .unwrap();
        let Command::Scan(options) = command else { panic!("expected a scan") };
        assert_eq!(options.out, Some(PathBuf::from("D:\\results")));
        assert_eq!(options.paths, [PathBuf::from("C:\\Scripts")]);
        assert_eq!(options.network_paths, [PathBuf::from("\\\\srv\\netlogon")]);
        assert!(options.files_only && options.quiet);
        assert_eq!(options.lang, Lang::De);
        assert_eq!(parse_args(&[]).unwrap(), Command::Scan(Options::default()));
    }

    #[test]
    fn help_version_and_errors() {
        assert_eq!(parse_args(&["--lang", "pl", "--help"]).unwrap(), Command::Help(Lang::Pl));
        assert_eq!(parse_args(&["/?"]).unwrap(), Command::Help(Lang::En));
        assert_eq!(parse_args(&["--version"]).unwrap(), Command::Version);
        let unknown = parse_args(&["--lang", "fr", "--delete-everything"]).unwrap_err();
        assert_eq!(unknown.lang, Lang::Fr);
        assert!(matches!(unknown.kind, CliErrorKind::Usage(_)));
        let language = parse_args(&["--lang", "tlh"]).unwrap_err();
        assert_eq!(language.kind, CliErrorKind::UnsupportedLanguage("tlh".into()));
        assert!(parse_args(&["--out", "a", "--out", "b"]).is_err());
        assert!(parse_args(&["stray"]).is_err());
    }

    #[test]
    fn plans_check_paths() {
        let dir = tempfile::tempdir().unwrap();
        let options = Options { paths: vec![dir.path().to_path_buf()], files_only: true, ..Options::default() };
        let plan = plan(&options).unwrap();
        assert!(!plan.system_sources);
        assert_eq!(plan.paths, [dir.path().to_path_buf()]);

        let missing = Options { paths: vec![dir.path().join("missing")], ..Options::default() };
        assert!(matches!(super::plan(&missing), Err(PathProblem::Missing(_))));
        let unc_as_path = Options { paths: vec![PathBuf::from("\\\\srv\\share")], ..Options::default() };
        assert!(matches!(super::plan(&unc_as_path), Err(PathProblem::NetworkPath(_))));
        let local_as_unc = Options { network_paths: vec![PathBuf::from("C:\\x")], ..Options::default() };
        assert!(matches!(super::plan(&local_as_unc), Err(PathProblem::NotNetworkPath(_))));
    }
}
