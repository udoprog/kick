//! The `kick inspect` command, which reports the configuration kick loaded
//! and the repos it would act on.

use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use std::process::ExitCode;

use anyhow::Result;
use clap::Parser;
use relative_path::RelativePath;
use serde::Serialize;

use crate::cli::deploy::{
    Choice, LOCAL_HOST, choose_profile, default_bin_dir, default_unit_dir, trim_dir,
};
use crate::config::{
    Build, Config, ConfigCommand, ConfigSource, Deploy, DeployKind, Os, Section, SourceState,
    SystemdScope, UnitTemplate,
};
use crate::ctxt::Paths;
use crate::model::{Repo, RepoSource};
use crate::systemd::{self, Directives, UnitKind};
use crate::{Exclusion, GITHUB_TOKEN, KICK_TOML, RepoOptions};

#[derive(Default, Debug, Parser)]
pub(crate) struct Opts {
    /// Print the report as JSON.
    #[arg(long)]
    json: bool,
    /// The profile to report the `[install]` and `[deploy]` configuration
    /// for, the same as `--to` for `kick install` and `kick deploy`.
    #[arg(long, value_name = "PROFILE")]
    to: Option<String>,
}

/// What is collected about loading configuration and selecting repos while
/// `kick inspect` runs.
#[derive(Default)]
pub(crate) struct Collected {
    /// Every configuration file which was looked for, in load order.
    pub(crate) sources: Vec<ConfigSource>,
    /// Errors raised while loading configuration.
    pub(crate) errors: Vec<anyhow::Error>,
    /// The error which selecting repos raised, which makes other commands
    /// refuse to run.
    pub(crate) selection_error: Option<String>,
    /// Why each repo is excluded from the selection, in repo order.
    pub(crate) exclusions: Vec<Option<Exclusion>>,
}

/// Everything `kick inspect` reports on.
pub(crate) struct Inspect<'a> {
    pub(crate) paths: Paths<'a>,
    pub(crate) worktree: Option<(&'a RelativePath, &'a RelativePath)>,
    pub(crate) config: &'a Config<'a>,
    pub(crate) repos: &'a [Repo],
    pub(crate) from_group: bool,
    pub(crate) in_repo_path: bool,
    pub(crate) repo_opts: &'a RepoOptions,
    pub(crate) os: &'a Os,
    pub(crate) collected: Collected,
}

pub(crate) fn entry(cx: &Inspect<'_>, opts: &Opts) -> Result<ExitCode> {
    let report = build(cx, opts);

    let mut o = io::stdout().lock();

    if opts.json {
        serde_json::to_writer_pretty(&mut o, &report)?;
        writeln!(o)?;
    } else {
        write_text(&mut o, &report)?;
    }

    if report.errors.is_empty() && report.selection.error.is_none() {
        Ok(ExitCode::SUCCESS)
    } else {
        Ok(ExitCode::FAILURE)
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct Report {
    root: PathBuf,
    current: Option<String>,
    worktree: Option<WorktreeReport>,
    config: Vec<SourceReport>,
    user_config: UserConfigReport,
    errors: Vec<String>,
    selection: SelectionReport,
    repos_from: &'static str,
    repos: Vec<RepoReport>,
}

#[derive(Debug, Serialize)]
struct WorktreeReport {
    repo: String,
    path: PathBuf,
}

#[derive(Debug, Serialize)]
struct SourceReport {
    /// What the source is, `defaults` or `file`.
    kind: &'static str,
    /// The source as it is referred to elsewhere in the report.
    name: String,
    path: Option<PathBuf>,
    /// The repo the source configures, or `None` if it applies to every repo.
    applies_to: Option<String>,
    /// `built-in`, `loaded`, `missing` or `invalid`.
    state: &'static str,
    keys: Vec<String>,
    repo_sections: Vec<RepoSectionReport>,
    errors: Vec<String>,
}

#[derive(Debug, Serialize)]
struct RepoSectionReport {
    path: String,
    keys: Vec<String>,
}

#[derive(Debug, Serialize)]
struct UserConfigReport {
    dir: Option<PathBuf>,
    files: Vec<FileReport>,
}

#[derive(Debug, Serialize)]
struct FileReport {
    path: PathBuf,
    exists: bool,
}

#[derive(Debug, Serialize)]
struct SelectionReport {
    all: bool,
    paths: Vec<String>,
    sets: Vec<String>,
    supported_os: bool,
    os: String,
    /// The repo-relative current directory which limits the selection to the
    /// repo containing it.
    current_dir: Option<String>,
    /// Why the selection fails, in which case other commands refuse to run.
    error: Option<String>,
}

#[derive(Debug, Serialize)]
struct RepoReport {
    path: String,
    sources: Vec<String>,
    urls: Vec<String>,
    selected: bool,
    excluded: Option<String>,
    layers: Vec<LayerReport>,
    effective: Option<EffectiveReport>,
}

#[derive(Debug, Serialize)]
struct LayerReport {
    source: String,
    state: &'static str,
    keys: Vec<KeyReport>,
}

#[derive(Debug, Serialize)]
struct KeyReport {
    key: String,
    /// The earlier layers which also set this key, which this layer is merged
    /// over.
    merges_over: Vec<String>,
}

#[derive(Debug, Serialize)]
struct EffectiveReport {
    name: Option<String>,
    cargo_toml: Option<String>,
    branch: Option<String>,
    os: Vec<String>,
    build: BuildReport,
    install: SectionReport,
    deploy: SectionReport,
}

#[derive(Debug, Serialize)]
struct BuildReport {
    binary: Option<String>,
    package: Option<String>,
    profile: Option<String>,
    features: Vec<String>,
    pre_build: Vec<String>,
    commands: Vec<String>,
}

#[derive(Debug, Serialize)]
struct SectionReport {
    /// Whether any configuration source sets this section.
    configured: bool,
    profiles: Vec<String>,
    /// The profile which would be used, if any.
    profile: Option<String>,
    /// Why no profile can be picked without `--to`.
    profile_error: Option<String>,
    kind: String,
    hosts: Vec<String>,
    user: Option<String>,
    port: Option<u16>,
    sudo: Option<bool>,
    bin_dir: Option<String>,
    unit_dir: Option<String>,
    staging_dir: Option<String>,
    build: BuildReport,
    commands: Vec<String>,
    files: Vec<DeployFileReport>,
    post_install: Vec<String>,
    post_start: Vec<String>,
    systemd: Option<SystemdReport>,
}

#[derive(Debug, Serialize)]
struct DeployFileReport {
    source: String,
    /// The source resolved against the repo directory.
    resolved: PathBuf,
    dest: String,
    mode: Option<String>,
}

#[derive(Debug, Serialize)]
struct SystemdReport {
    /// The template the unit is rendered from, or `None` for the built-in one.
    template: Option<PathBuf>,
    name: Option<String>,
    enable: Option<bool>,
    scope: String,
    #[serde(flatten)]
    unit: UnitReport,
    socket: Option<SocketReport>,
}

#[derive(Debug, Serialize)]
struct SocketReport {
    template: Option<PathBuf>,
    name: Option<String>,
    #[serde(flatten)]
    unit: UnitReport,
}

/// What a unit is rendered with.
#[derive(Debug, Serialize)]
struct UnitReport {
    /// The variables the template is rendered with, in the order they are
    /// layered.
    variables: Vec<VariableReport>,
    /// The pass-through directives.
    directives: Vec<DirectiveReport>,
    /// Configuration which doesn't end up in the unit.
    warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
struct VariableReport {
    name: String,
    /// The value, or `None` when it is only known when deploying.
    value: Option<toml::Value>,
    /// `global`, `configured` or `built-in`.
    origin: &'static str,
    /// Where a value which is only known when deploying comes from.
    note: Option<String>,
}

#[derive(Debug, Serialize)]
struct DirectiveReport {
    /// The table the directive is configured in, such as `service`.
    section: &'static str,
    directive: String,
    values: Vec<String>,
}

/// The name of the built-in defaults layer.
const DEFAULTS: &str = "built-in defaults";

fn build(cx: &Inspect<'_>, opts: &Opts) -> Report {
    let paths = cx.paths;
    let root = absolute(paths.root);

    let worktree = cx.worktree.map(|(repo, checkout)| WorktreeReport {
        repo: display_repo(repo),
        path: absolute(&checkout.to_path(paths.root)),
    });

    let mut config = Vec::new();

    config.push(SourceReport {
        kind: "defaults",
        name: DEFAULTS.to_owned(),
        path: None,
        applies_to: None,
        state: "built-in",
        keys: defaults_keys(cx.config.defaults),
        repo_sections: Vec::new(),
        errors: Vec::new(),
    });

    for source in &cx.collected.sources {
        config.push(SourceReport {
            kind: "file",
            name: source.config_path.to_string(),
            path: Some(absolute(&source.path)),
            applies_to: (!source.dir.as_str().is_empty()).then(|| source.dir.to_string()),
            state: state_name(source.state),
            keys: source.keys.clone(),
            repo_sections: source
                .repos
                .iter()
                .map(|(path, keys)| RepoSectionReport {
                    path: path.to_string(),
                    keys: keys.clone(),
                })
                .collect(),
            errors: source.errors.clone(),
        });
    }

    let user_config = UserConfigReport {
        dir: paths.config.map(absolute),
        files: paths
            .config
            .into_iter()
            .map(|p| p.join(GITHUB_TOKEN))
            .chain([paths.root.join(GITHUB_TOKEN)])
            .map(|p| FileReport {
                exists: p.is_file(),
                path: absolute(&p),
            })
            .collect(),
    };

    let errors = cx
        .collected
        .errors
        .iter()
        .map(|e| format!("{e:#}"))
        .collect();

    let current_dir = (!cx.repo_opts.all && cx.in_repo_path)
        .then_some(paths.current)
        .flatten()
        .map(|c| c.to_string());

    let selection = SelectionReport {
        all: cx.repo_opts.all,
        paths: cx.repo_opts.repos.clone(),
        sets: cx
            .repo_opts
            .set
            .iter()
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect(),
        supported_os: cx.repo_opts.supported_os,
        os: cx.os.to_string(),
        current_dir,
        error: cx.collected.selection_error.clone(),
    };

    let mut repos = Vec::new();

    for (index, repo) in cx.repos.iter().enumerate() {
        let exclusion = cx.collected.exclusions.get(index).cloned().flatten();
        let selected = exclusion.is_none() && cx.collected.selection_error.is_none();

        let sources = repo_sources(repo);
        let mut urls = vec![repo.url().to_string()];

        if let Some(c) = cx.config.repos.get(repo.path()) {
            for url in &c.urls {
                let url = url.to_string();

                if !urls.contains(&url) {
                    urls.push(url);
                }
            }
        }

        let layers = layers(&config, &cx.collected.sources, repo.path());

        let effective = selected.then(|| effective(cx, opts, repo, &layers));

        repos.push(RepoReport {
            path: display_repo(repo.path()),
            sources,
            urls,
            selected,
            excluded: exclusion.map(|e| e.to_string()),
            layers,
            effective,
        });
    }

    Report {
        root,
        current: paths.current.map(|c| c.to_string()),
        worktree,
        config,
        user_config,
        errors,
        selection,
        repos_from: if cx.from_group { "config" } else { "git" },
        repos,
    }
}

fn state_name(state: SourceState) -> &'static str {
    match state {
        SourceState::Loaded => "loaded",
        SourceState::Missing => "missing",
        SourceState::Invalid => "invalid",
    }
}

fn defaults_keys(defaults: &toml::Table) -> Vec<String> {
    defaults
        .keys()
        .map(|key| format!("variables.{key}"))
        .collect()
}

fn repo_sources(repo: &Repo) -> Vec<String> {
    repo.source_list()
        .map(|source| match source {
            RepoSource::Git => String::from("the git checkout at the root"),
            RepoSource::Config(path) => format!("{KICK_TOML} [repo.\"{path}\"]"),
        })
        .collect()
}

/// The configuration layers which apply to the repo at `path`, from least to
/// most specific.
fn layers(
    config: &[SourceReport],
    sources: &[ConfigSource],
    path: &RelativePath,
) -> Vec<LayerReport> {
    let path = path.normalize();
    let mut layers: Vec<(String, &'static str, Vec<String>)> = Vec::new();

    if let Some(defaults) = config.first() {
        layers.push((defaults.name.clone(), defaults.state, defaults.keys.clone()));
    }

    for source in sources {
        let dir = source.dir.normalize();

        if dir.as_str().is_empty() {
            layers.push((
                source.config_path.to_string(),
                state_name(source.state),
                source.keys.clone(),
            ));

            for (repo, keys) in &source.repos {
                if repo.normalize() == path {
                    layers.push((
                        format!("{} [repo.\"{repo}\"]", source.config_path),
                        "loaded",
                        keys.iter().filter(|k| *k != "url").cloned().collect(),
                    ));
                }
            }
        } else if dir == path {
            layers.push((
                source.config_path.to_string(),
                state_name(source.state),
                source.keys.clone(),
            ));
        }
    }

    let mut out = Vec::<LayerReport>::new();

    for (source, state, keys) in layers {
        let keys = keys
            .into_iter()
            .map(|key| {
                let merges_over = out
                    .iter()
                    .filter(|layer| layer.keys.iter().any(|k| overlaps(&k.key, &key)))
                    .map(|layer| layer.source.clone())
                    .collect();

                KeyReport { key, merges_over }
            })
            .collect();

        out.push(LayerReport {
            source,
            state,
            keys,
        });
    }

    out
}

/// Test if two dotted keys refer to overlapping configuration.
fn overlaps(a: &str, b: &str) -> bool {
    fn prefix(a: &str, b: &str) -> bool {
        a.strip_prefix(b).is_some_and(|rest| rest.starts_with('.'))
    }

    a == b || prefix(a, b) || prefix(b, a)
}

fn effective(
    cx: &Inspect<'_>,
    opts: &Opts,
    repo: &Repo,
    layers: &[LayerReport],
) -> EffectiveReport {
    let config = cx.config;
    let base_build = config.build(repo);
    let repo_dir = absolute(&cx.paths.to_path(repo.path()));

    let configured = |section: &str| {
        layers
            .iter()
            .any(|layer| layer.keys.iter().any(|k| overlaps(&k.key, section)))
    };

    let globals = config.variables(repo);

    let install = section(
        &config.install(repo),
        Section::Install,
        configured("install"),
        &base_build,
        &globals,
        &repo_dir,
        opts,
    );

    let deploy = section(
        &config.deploy(repo),
        Section::Deploy,
        configured("deploy"),
        &base_build,
        &globals,
        &repo_dir,
        opts,
    );

    EffectiveReport {
        name: config.name(repo).map(str::to_owned),
        cargo_toml: config.cargo_toml(repo).map(|p| p.to_string()),
        branch: config.branch(repo).map(str::to_owned),
        os: config.os(repo).iter().map(|os| os.to_string()).collect(),
        build: build_report(&base_build),
        install,
        deploy,
    }
}

fn section(
    base: &Deploy,
    section: Section,
    configured: bool,
    base_build: &Build,
    globals: &toml::Table,
    repo_dir: &Path,
    opts: &Opts,
) -> SectionReport {
    let (profile, profile_error) = match choose_profile(base, section, opts.to.as_deref(), false) {
        Ok(Choice::Base) => (None, None),
        Ok(Choice::Profile(name)) => (Some(name.to_owned()), None),
        Ok(Choice::Ask(..)) => (None, None),
        Err(error) => (None, Some(error.to_string())),
    };

    let config = profile
        .as_deref()
        .and_then(|name| base.with_profile(name))
        .unwrap_or_else(|| base.clone());

    let kind = match section {
        Section::Install => DeployKind::Local,
        Section::Deploy => config.kind.unwrap_or_default(),
    };

    let mut build = base_build.clone();
    build.merge_with(config.build.clone());

    let systemd_default = match section {
        Section::Install => config.systemd.is_some(),
        Section::Deploy => true,
    };

    let systemd_config = config.systemd.clone().unwrap_or_default();

    let scope = systemd_config.scope.unwrap_or(match kind {
        DeployKind::Ssh => SystemdScope::System,
        DeployKind::Local => SystemdScope::User,
    });

    let systemd = systemd_config.enabled.unwrap_or(systemd_default).then(|| {
        let binary = build.binary.clone().or_else(|| build.package.clone());

        let bin_dir =
            trim_dir(config.bin_dir.as_deref().unwrap_or(&default_bin_dir(kind))).to_owned();
        let unit_dir = trim_dir(
            config
                .unit_dir
                .as_deref()
                .unwrap_or(default_unit_dir(scope)),
        )
        .to_owned();

        let name = systemd_config.name.clone().or_else(|| binary.clone());

        let host = match kind {
            DeployKind::Local => Provided::Value(string(LOCAL_HOST)),
            DeployKind::Ssh => match &config.host[..] {
                [] => Provided::Note("from --host"),
                [host] => Provided::Value(string(bare_host(host))),
                hosts => Provided::Value(toml::Value::Array(
                    hosts.iter().map(|host| string(bare_host(host))).collect(),
                )),
            },
        };

        let known = |value: Option<&String>, note: &'static str| match value {
            Some(value) => Provided::Value(string(value)),
            None => Provided::Note(note),
        };

        let exec = binary.as_ref().map(|binary| format!("{bin_dir}/{binary}"));

        let common = |name: Option<&String>, note: &'static str| {
            vec![
                ("name", known(name, note)),
                (
                    "binary",
                    known(binary.as_ref(), "from the package in Cargo.toml"),
                ),
                ("exec", known(exec.as_ref(), "<bin_dir>/<binary>")),
                ("bin_dir", Provided::Value(string(&bin_dir))),
                ("unit_dir", Provided::Value(string(&unit_dir))),
                ("host", host.clone()),
                ("scope", Provided::Value(string(scope.as_str()))),
            ]
        };

        let label = format!("[{}.systemd]", section.as_str());

        let socket = systemd_config
            .socket
            .as_ref()
            .filter(|s| s.enabled.unwrap_or(true));

        let socket_name = socket.and_then(|s| s.name.clone().or_else(|| name.clone()));

        let mut provided = common(name.as_ref(), "same as binary");

        if socket.is_some() {
            provided.push((
                "socket",
                known(
                    socket_name.as_ref().map(|n| format!("{n}.socket")).as_ref(),
                    "<name>.socket",
                ),
            ));
        }

        let unit = unit_report(
            UnitKind::Service,
            &label,
            systemd_config.template.as_ref(),
            globals,
            &systemd_config.variables,
            &systemd_config.directives,
            provided,
        );

        let socket = socket.map(|s| {
            let mut provided = common(socket_name.as_ref(), "same as the service");

            provided.push((
                "service",
                known(
                    name.as_ref().map(|n| format!("{n}.service")).as_ref(),
                    "<name>.service",
                ),
            ));

            SocketReport {
                template: s.template.as_ref().map(|t| absolute(&t.path)),
                name: s.name.clone(),
                unit: unit_report(
                    UnitKind::Socket,
                    &format!("[{}.systemd.socket]", section.as_str()),
                    s.template.as_ref(),
                    globals,
                    &s.variables,
                    &s.directives,
                    provided,
                ),
            }
        });

        SystemdReport {
            template: systemd_config.template.as_ref().map(|t| absolute(&t.path)),
            name: systemd_config.name.clone(),
            enable: systemd_config.enable,
            scope: scope.to_string(),
            unit,
            socket,
        }
    });

    SectionReport {
        configured,
        profiles: base.profiles.keys().cloned().collect(),
        profile,
        profile_error,
        kind: kind.to_string(),
        hosts: config.host.clone(),
        user: config.user.clone(),
        port: config.port,
        sudo: config.sudo,
        bin_dir: config.bin_dir.clone(),
        unit_dir: config.unit_dir.clone(),
        staging_dir: config.staging_dir.clone(),
        build: build_report(&build),
        commands: commands(&config.commands),
        files: config
            .files
            .iter()
            .map(|file| DeployFileReport {
                source: file.source.to_string(),
                resolved: normalize(&file.source.to_path(repo_dir)),
                dest: file.dest.clone(),
                mode: file.mode.as_ref().map(|m| format!("{m:?}")),
            })
            .collect(),
        post_install: commands(&config.post_install),
        post_start: commands(&config.post_start),
        systemd,
    }
}

/// A variable kick provides to a unit template.
#[derive(Clone)]
enum Provided {
    /// The value is known up front.
    Value(toml::Value),
    /// The value is only known when deploying, and comes from what is noted.
    Note(&'static str),
}

fn string(value: &str) -> toml::Value {
    toml::Value::String(value.to_owned())
}

/// The host without any login user, which is what a unit template sees.
fn bare_host(host: &str) -> &str {
    host.rsplit_once('@').map_or(host, |(_, host)| host)
}

/// Report what a unit is rendered with, the same way `kick deploy` layers it.
fn unit_report(
    kind: UnitKind,
    label: &str,
    template: Option<&UnitTemplate>,
    globals: &toml::Table,
    configured: &toml::Table,
    directives: &Directives,
    provided: Vec<(&'static str, Provided)>,
) -> UnitReport {
    let source = template.map(|t| &*t.source);
    let mut variables = systemd::configured(source, globals, configured);
    let mut notes = Vec::new();

    for (name, value) in provided {
        match value {
            Provided::Value(value) => variables.provide(name, value),
            // NB: A configured `exec` overrides what kick would provide.
            Provided::Note(..) if variables.origin(name) == Some(systemd::Origin::Configured) => {}
            Provided::Note(note) => notes.push(VariableReport {
                name: name.to_owned(),
                value: None,
                origin: systemd::Origin::BuiltIn.as_str(),
                note: Some(note.to_owned()),
            }),
        }
    }

    let mut report = variables
        .iter()
        .map(|(name, value, origin)| VariableReport {
            name: name.to_owned(),
            value: Some(value.clone()),
            origin: origin.as_str(),
            note: None,
        })
        .collect::<Vec<_>>();

    report.extend(notes);

    UnitReport {
        variables: report,
        directives: directives
            .iter(kind)
            .map(|(section, directive, values)| DirectiveReport {
                section,
                directive: directive.to_owned(),
                values: values.into_iter().map(str::to_owned).collect(),
            })
            .collect(),
        warnings: systemd::lint(kind, label, source, configured, directives),
    }
}

fn build_report(build: &Build) -> BuildReport {
    BuildReport {
        binary: build.binary.clone(),
        package: build.package.clone(),
        profile: build.profile.clone(),
        features: build.features.clone(),
        pre_build: commands(&build.pre_build),
        commands: commands(&build.commands),
    }
}

fn commands(commands: &[ConfigCommand]) -> Vec<String> {
    commands
        .iter()
        .map(|c| {
            let line = c.argv().join(" ");

            if c.sudo { format!("sudo {line}") } else { line }
        })
        .collect()
}

fn display_repo(path: &RelativePath) -> String {
    if path.as_str().is_empty() {
        String::from(".")
    } else {
        path.to_string()
    }
}

/// Make a path absolute, resolving symlinks if it exists.
fn absolute(path: &Path) -> PathBuf {
    if let Ok(path) = path.canonicalize() {
        return path;
    }

    normalize(&std::path::absolute(path).unwrap_or_else(|_| path.to_owned()))
}

/// Lexically normalize a path, removing `.` and resolving `..` components.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();

    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push(c);
                }
            }
            c => out.push(c),
        }
    }

    out
}

fn write_text(o: &mut impl Write, r: &Report) -> io::Result<()> {
    writeln!(o, "Root: {}", r.root.display())?;

    if let Some(current) = &r.current {
        let current = if current.is_empty() { "." } else { current };
        writeln!(o, "Current: {current} (relative to the root)")?;
    }

    match &r.worktree {
        Some(w) => writeln!(o, "Worktree: repo {} uses {}", w.repo, w.path.display())?,
        None => writeln!(o, "Worktree: none")?,
    }

    writeln!(o)?;
    writeln!(o, "Configuration, in load order:")?;

    for (index, source) in r.config.iter().enumerate() {
        let n = index + 1;

        match &source.path {
            Some(path) => writeln!(o, "  {n}. {} ({})", path.display(), source.state)?,
            None => writeln!(o, "  {n}. {} ({})", source.name, source.state)?,
        }

        match &source.applies_to {
            Some(repo) => writeln!(o, "       applies to repo {repo}")?,
            None if source.kind == "file" => writeln!(o, "       applies to every repo")?,
            None => {}
        }

        if !source.keys.is_empty() {
            writeln!(o, "       sets: {}", source.keys.join(", "))?;
        }

        for section in &source.repo_sections {
            writeln!(
                o,
                "       [repo.\"{}\"]: {}",
                section.path,
                section.keys.join(", ")
            )?;
        }

        for error in &source.errors {
            writeln!(o, "       error: {error}")?;
        }
    }

    writeln!(o)?;

    match &r.user_config.dir {
        Some(dir) => writeln!(
            o,
            "User configuration: {} (only `{GITHUB_TOKEN}` is read, no {KICK_TOML})",
            dir.display()
        )?,
        None => writeln!(o, "User configuration: no directory")?,
    }

    for file in &r.user_config.files {
        let state = if file.exists { "present" } else { "absent" };
        writeln!(o, "  {} ({state})", file.path.display())?;
    }

    if !r.errors.is_empty() {
        writeln!(o)?;
        writeln!(
            o,
            "Errors ({}), other commands refuse to run until they are fixed:",
            r.errors.len()
        )?;

        for error in &r.errors {
            let mut lines = error.lines();

            if let Some(first) = lines.next() {
                writeln!(o, "  - {first}")?;
            }

            for line in lines {
                writeln!(o, "    {line}")?;
            }
        }
    }

    writeln!(o)?;
    writeln!(o, "Selection:")?;

    let s = &r.selection;
    writeln!(o, "  --all: {}", if s.all { "yes" } else { "no" })?;

    if !s.paths.is_empty() {
        writeln!(o, "  -p: {}", s.paths.join(", "))?;
    }

    if !s.sets.is_empty() {
        writeln!(o, "  --set: {}", s.sets.join(", "))?;
    }

    if s.supported_os {
        writeln!(o, "  --supported-os: {}", s.os)?;
    }

    if let Some(current) = &s.current_dir {
        writeln!(
            o,
            "  limited to the repo containing the current directory `{current}`"
        )?;
    }

    if let Some(error) = &s.error {
        writeln!(o, "  error, other commands refuse to run:")?;

        for line in error.lines() {
            writeln!(o, "    {line}")?;
        }
    }

    writeln!(o)?;

    let from = match r.repos_from {
        "config" => format!("from [repo] sections in {KICK_TOML}"),
        _ => String::from("no [repo] sections, so the git checkout at the root"),
    };

    let selected = r.repos.iter().filter(|repo| repo.selected).count();

    writeln!(
        o,
        "Repos ({} total, {selected} selected, {from}):",
        r.repos.len()
    )?;

    if r.repos.is_empty() {
        writeln!(o, "  none")?;
    }

    for repo in &r.repos {
        let mark = if repo.selected { "+" } else { "-" };
        writeln!(o, "  {mark} {}", repo.path)?;

        match &repo.excluded {
            Some(reason) => writeln!(o, "      excluded: {reason}")?,
            None if repo.selected => writeln!(o, "      selected")?,
            None => writeln!(o, "      not selected, see the selection error")?,
        }

        writeln!(o, "      from: {}", repo.sources.join(", "))?;
        writeln!(o, "      url: {}", repo.urls.join(", "))?;
        // NB: Layers are only shown for selected repos to keep the output
        // readable with many repos, the JSON output has all of them.
        if !repo.selected {
            continue;
        }

        writeln!(o, "      layers, least to most specific:")?;

        for layer in &repo.layers {
            if layer.keys.is_empty() {
                let state = match layer.state {
                    "missing" | "invalid" => layer.state,
                    _ => "sets nothing",
                };

                writeln!(o, "        {} ({state})", layer.source)?;
                continue;
            }

            let keys = layer
                .keys
                .iter()
                .map(|k| {
                    if k.merges_over.is_empty() {
                        k.key.clone()
                    } else {
                        format!("{} (over {})", k.key, k.merges_over.join(", "))
                    }
                })
                .collect::<Vec<_>>();

            writeln!(o, "        {}: {}", layer.source, keys.join(", "))?;
        }
    }

    for repo in &r.repos {
        let Some(e) = &repo.effective else {
            continue;
        };

        writeln!(o)?;
        writeln!(o, "Effective configuration of {}:", repo.path)?;

        if let Some(name) = &e.name {
            writeln!(o, "  name: {name}")?;
        }

        if let Some(cargo_toml) = &e.cargo_toml {
            writeln!(o, "  cargo_toml: {cargo_toml}")?;
        }

        if let Some(branch) = &e.branch {
            writeln!(o, "  branch: {branch}")?;
        }

        if !e.os.is_empty() {
            writeln!(o, "  os: {}", e.os.join(", "))?;
        }

        writeln!(o, "  [build]")?;
        write_build(o, &e.build, "    ")?;
        write_section(o, "install", &e.install)?;
        write_section(o, "deploy", &e.deploy)?;
    }

    Ok(())
}

fn write_build(o: &mut impl Write, b: &BuildReport, indent: &str) -> io::Result<()> {
    let binary = b
        .binary
        .as_deref()
        .unwrap_or("(the package name in Cargo.toml)");
    writeln!(o, "{indent}binary: {binary}")?;

    if let Some(package) = &b.package {
        writeln!(o, "{indent}package: {package}")?;
    }

    writeln!(
        o,
        "{indent}profile: {}",
        b.profile.as_deref().unwrap_or("release")
    )?;

    if !b.features.is_empty() {
        writeln!(o, "{indent}features: {}", b.features.join(", "))?;
    }

    for command in &b.pre_build {
        writeln!(o, "{indent}pre_build: {command}")?;
    }

    for command in &b.commands {
        writeln!(o, "{indent}command: {command}")?;
    }

    Ok(())
}

fn write_section(o: &mut impl Write, name: &str, s: &SectionReport) -> io::Result<()> {
    if !s.configured {
        writeln!(o, "  [{name}] not configured")?;
        return Ok(());
    }

    writeln!(o, "  [{name}]")?;

    if !s.profiles.is_empty() {
        writeln!(o, "    profiles: {}", s.profiles.join(", "))?;
    }

    match (&s.profile, &s.profile_error) {
        (Some(profile), _) => writeln!(o, "    profile: {profile}")?,
        (None, Some(error)) => writeln!(o, "    profile: none, {error}")?,
        (None, None) => {}
    }

    writeln!(o, "    kind: {}", s.kind)?;

    if !s.hosts.is_empty() {
        writeln!(o, "    host: {}", s.hosts.join(", "))?;
    } else if s.kind == DeployKind::Ssh.to_string() {
        writeln!(o, "    host: from --host")?;
    }

    if let Some(user) = &s.user {
        writeln!(o, "    user: {user}")?;
    }

    if let Some(port) = s.port {
        writeln!(o, "    port: {port}")?;
    }

    if let Some(sudo) = s.sudo {
        writeln!(o, "    sudo: {sudo}")?;
    }

    for (key, value) in [
        ("bin_dir", &s.bin_dir),
        ("unit_dir", &s.unit_dir),
        ("staging_dir", &s.staging_dir),
    ] {
        if let Some(value) = value {
            writeln!(o, "    {key}: {value}")?;
        }
    }

    writeln!(o, "    build:")?;
    write_build(o, &s.build, "      ")?;

    for command in &s.commands {
        writeln!(o, "    command: {command}")?;
    }

    for file in &s.files {
        write!(
            o,
            "    file: {} -> {} ({})",
            file.source,
            file.dest,
            file.resolved.display()
        )?;

        if let Some(mode) = &file.mode {
            write!(o, " mode {mode}")?;
        }

        writeln!(o)?;
    }

    for command in &s.post_install {
        writeln!(o, "    post_install: {command}")?;
    }

    for command in &s.post_start {
        writeln!(o, "    post_start: {command}")?;
    }

    match &s.systemd {
        Some(systemd) => {
            let template = match &systemd.template {
                Some(path) => path.display().to_string(),
                None => String::from("built-in template"),
            };

            writeln!(o, "    systemd: {template}, {} scope", systemd.scope)?;

            if let Some(name) = &systemd.name {
                writeln!(o, "      name: {name}")?;
            }

            if let Some(enable) = systemd.enable {
                writeln!(o, "      enable: {enable}")?;
            }

            write_unit(o, &systemd.unit, "      ")?;

            if let Some(socket) = &systemd.socket {
                let template = match &socket.template {
                    Some(path) => path.display().to_string(),
                    None => String::from("built-in template"),
                };

                writeln!(o, "      socket: {template}")?;

                if let Some(name) = &socket.name {
                    writeln!(o, "        name: {name}")?;
                }

                write_unit(o, &socket.unit, "        ")?;
            }
        }
        None => writeln!(o, "    systemd: no unit")?,
    }

    Ok(())
}

fn write_unit(o: &mut impl Write, unit: &UnitReport, indent: &str) -> io::Result<()> {
    if !unit.variables.is_empty() {
        writeln!(o, "{indent}variables:")?;

        for variable in &unit.variables {
            match (&variable.value, &variable.note) {
                (Some(value), _) => writeln!(
                    o,
                    "{indent}  {} = {value} ({})",
                    variable.name, variable.origin
                )?,
                (None, Some(note)) => writeln!(
                    o,
                    "{indent}  {}: {note} ({})",
                    variable.name, variable.origin
                )?,
                (None, None) => writeln!(o, "{indent}  {} ({})", variable.name, variable.origin)?,
            }
        }
    }

    if !unit.directives.is_empty() {
        writeln!(o, "{indent}directives:")?;

        for directive in &unit.directives {
            for value in &directive.values {
                writeln!(
                    o,
                    "{indent}  [{}] {}={value}",
                    systemd::section_title(directive.section),
                    directive.directive
                )?;
            }
        }
    }

    for warning in &unit.warnings {
        writeln!(o, "{indent}warning: {warning}")?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::fs;
    use std::path::Path;

    use relative_path::{RelativePath, RelativePathBuf};

    use super::{Collected, Inspect, Opts, Report, build};
    use crate::config::{self, Os, SourceState};
    use crate::ctxt::Paths;
    use crate::glob::Fragment;
    use crate::model::{Repo, RepoSource};
    use crate::templates::Templating;
    use crate::{Exclusion, RepoOptions, filter_repos};

    fn paths(root: &Path) -> Paths<'_> {
        Paths {
            root,
            current: None,
            config: None,
            cache: None,
            redirect: None,
        }
    }

    fn repos(config: &config::Config<'_>) -> Vec<Repo> {
        config
            .repos
            .iter()
            .map(|(path, c)| {
                Repo::new(
                    [RepoSource::Config(path.clone())],
                    path.clone(),
                    c.urls.iter().next().unwrap().clone(),
                )
            })
            .collect()
    }

    /// Build the report for every repo in `root` without any selection.
    fn report(root: &Path, opts: &Opts) -> Report {
        let templating = Templating::new().unwrap();
        let defaults = config::defaults();
        let loaded = config::load_all(paths(root), &templating, &defaults);
        let repos = repos(&loaded.config);
        let repo_opts = RepoOptions::default();

        let cx = Inspect {
            paths: paths(root),
            worktree: None,
            config: &loaded.config,
            repos: &repos,
            from_group: true,
            in_repo_path: false,
            repo_opts: &repo_opts,
            os: &Os::Linux,
            collected: Collected {
                exclusions: vec![None; repos.len()],
                sources: loaded.sources,
                errors: loaded.errors,
                selection_error: None,
            },
        };

        build(&cx, opts)
    }

    #[test]
    fn nested_hierarchy() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        fs::write(
            root.join("Kick.toml"),
            "[deploy]\n\
             user = \"base\"\n\
             [variables]\n\
             ci_name = \"Build\"\n\
             [repo.\"repos/app\"]\n\
             url = \"https://example.com/app\"\n\
             deploy = { user = \"section\" }\n\
             [repo.\"repos/lib\"]\n\
             url = \"https://example.com/lib\"\n",
        )
        .unwrap();

        let app = root.join("repos/app");
        fs::create_dir_all(app.join("systemd")).unwrap();
        fs::write(app.join("systemd/app.service"), "[Service]\n").unwrap();
        fs::write(
            app.join("Kick.toml"),
            "[deploy]\n\
             default_profile = \"remote\"\n\
             [deploy.profiles.remote]\n\
             host = [\"remote.example.com\"]\n\
             systemd = \"systemd/app.service\"\n\
             [[deploy.files]]\n\
             source = \"config/*.toml\"\n\
             dest = \"/etc/app/\"\n",
        )
        .unwrap();

        let report = report(root, &Opts::default());
        assert!(report.errors.is_empty(), "{:?}", report.errors);

        let sources = report
            .config
            .iter()
            .map(|s| (s.name.as_str(), s.state))
            .collect::<Vec<_>>();

        assert_eq!(
            sources,
            [
                ("built-in defaults", "built-in"),
                ("Kick.toml", "loaded"),
                ("repos/app/Kick.toml", "loaded"),
                ("repos/lib/Kick.toml", "missing"),
            ]
        );

        assert_eq!(report.config[1].keys, ["deploy", "variables"]);
        assert_eq!(report.config[1].repo_sections.len(), 2);
        assert_eq!(report.config[2].applies_to.as_deref(), Some("repos/app"));
        assert_eq!(
            report.config[1].path.as_deref(),
            Some(root.canonicalize().unwrap().join("Kick.toml").as_path())
        );

        let app = report.repos.iter().find(|r| r.path == "repos/app").unwrap();

        let layers = app
            .layers
            .iter()
            .map(|l| {
                let keys = l
                    .keys
                    .iter()
                    .map(|k| format!("{} < {}", k.key, k.merges_over.join(" < ")))
                    .collect::<Vec<_>>();
                (l.source.as_str(), keys)
            })
            .collect::<Vec<_>>();

        assert_eq!(
            layers,
            [
                (
                    "built-in defaults",
                    vec![
                        String::from("variables.ci_name < "),
                        String::from("variables.weekly_name < ")
                    ]
                ),
                (
                    "Kick.toml",
                    vec![
                        String::from("deploy < "),
                        String::from("variables < built-in defaults")
                    ]
                ),
                (
                    "Kick.toml [repo.\"repos/app\"]",
                    vec![String::from("deploy < Kick.toml")]
                ),
                (
                    "repos/app/Kick.toml",
                    vec![String::from(
                        "deploy < Kick.toml < Kick.toml [repo.\"repos/app\"]"
                    )]
                ),
            ]
        );

        let effective = app.effective.as_ref().unwrap();
        let deploy = &effective.deploy;
        assert!(deploy.configured);
        assert!(!effective.install.configured);
        assert_eq!(deploy.profile.as_deref(), Some("remote"));
        assert_eq!(deploy.hosts, ["remote.example.com"]);
        assert_eq!(deploy.user.as_deref(), Some("section"));

        let app_dir = root.canonicalize().unwrap().join("repos/app");
        let systemd = deploy.systemd.as_ref().unwrap();
        assert_eq!(
            systemd.template.as_deref(),
            Some(app_dir.join("systemd/app.service").as_path())
        );
        assert_eq!(deploy.files[0].resolved, app_dir.join("config/*.toml"));

        // `--to` picks another profile, which does not exist.
        let report = super::tests::report(
            root,
            &Opts {
                to: Some(String::from("missing")),
                ..Opts::default()
            },
        );
        let app = report.repos.iter().find(|r| r.path == "repos/app").unwrap();
        let deploy = &app.effective.as_ref().unwrap().deploy;
        assert!(deploy.profile.is_none());
        assert!(
            deploy
                .profile_error
                .as_deref()
                .unwrap()
                .contains("No profile named `missing`")
        );
    }

    /// The variables a unit is rendered with are reported along with where
    /// they come from, as are its directives and warnings.
    #[test]
    fn systemd_variables() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        fs::write(
            root.join("Kick.toml"),
            "[variables]\n\
             motd = \"hello\"\n\
             [repo.\"repos/app\"]\n\
             url = \"https://example.com/app\"\n",
        )
        .unwrap();

        let app = root.join("repos/app");
        fs::create_dir_all(app.join("systemd")).unwrap();
        fs::write(
            app.join("systemd/app.service"),
            "[Service]\nExecStart={{ exec }} {{ motd }}\n",
        )
        .unwrap();
        fs::write(
            app.join("Kick.toml"),
            "[build]\n\
             binary = \"app\"\n\
             [deploy.systemd]\n\
             user = \"app\"\n\
             exec = \"/usr/bin/wrapper /usr/local/bin/app\"\n\
             enabel = true\n\
             [deploy.systemd.service]\n\
             User = \"other\"\n\
             LimitNOFILE = 65536\n\
             [deploy.profiles.custom]\n\
             host = [\"login@moore\"]\n\
             [deploy.profiles.custom.systemd]\n\
             template = \"systemd/app.service\"\n\
             [deploy.profiles.plain]\n",
        )
        .unwrap();

        let opts = Opts {
            to: Some(String::from("plain")),
            ..Opts::default()
        };

        let report = report(root, &opts);
        assert!(report.errors.is_empty(), "{:?}", report.errors);

        let app = report.repos.iter().find(|r| r.path == "repos/app").unwrap();
        let deploy = &app.effective.as_ref().unwrap().deploy;
        let systemd = deploy.systemd.as_ref().unwrap();

        let variables = systemd
            .unit
            .variables
            .iter()
            .map(|v| {
                let value = match (&v.value, &v.note) {
                    (Some(value), _) => value.to_string(),
                    (None, Some(note)) => note.clone(),
                    (None, None) => String::new(),
                };

                format!("{} = {value} ({})", v.name, v.origin)
            })
            .collect::<Vec<_>>();

        // NB: The built-in template doesn't see global variables.
        assert_eq!(
            variables,
            [
                "user = \"app\" (configured)",
                "exec = \"/usr/bin/wrapper /usr/local/bin/app\" (configured)",
                "enabel = true (configured)",
                "name = \"app\" (built-in)",
                "binary = \"app\" (built-in)",
                "bin_dir = \"/usr/local/bin\" (built-in)",
                "unit_dir = \"/etc/systemd/system\" (built-in)",
                "scope = \"system\" (built-in)",
                "host = from --host (built-in)",
            ]
        );

        let directives = systemd
            .unit
            .directives
            .iter()
            .map(|d| format!("{}.{}={}", d.section, d.directive, d.values.join(",")))
            .collect::<Vec<_>>();

        assert_eq!(
            directives,
            ["service.LimitNOFILE=65536", "service.User=other"]
        );

        assert_eq!(
            systemd.unit.warnings,
            [
                "`enabel` in `[deploy.systemd]` is not used by the built-in template",
                "`user` in `[deploy.systemd]` is ignored since `User` is set in `[deploy.systemd.service]`",
            ]
        );

        let mut text = Vec::new();
        super::write_text(&mut text, &report).unwrap();
        let text = String::from_utf8(text).unwrap();
        assert!(text.contains("\n    host: from --host\n"), "{text}");
        assert!(
            text.contains("\n        host: from --host (built-in)\n"),
            "{text}"
        );
        assert!(
            text.contains("\n        [Service] LimitNOFILE=65536\n"),
            "{text}"
        );

        // A custom template sees the global variables, and is deployed to a
        // configured host.
        let opts = Opts {
            to: Some(String::from("custom")),
            ..Opts::default()
        };

        let report = super::tests::report(root, &opts);
        let app = report.repos.iter().find(|r| r.path == "repos/app").unwrap();
        let deploy = &app.effective.as_ref().unwrap().deploy;
        let unit = &deploy.systemd.as_ref().unwrap().unit;

        let origin = |name: &str| {
            unit.variables
                .iter()
                .find(|v| v.name == name)
                .map(|v| (v.value.as_ref().map(|v| v.to_string()), v.origin))
        };

        assert_eq!(
            origin("motd"),
            Some((Some(String::from("\"hello\"")), "global"))
        );
        assert_eq!(
            origin("host"),
            Some((Some(String::from("\"moore\"")), "built-in"))
        );

        assert_eq!(
            unit.warnings,
            [
                "`user` in `[deploy.systemd]` is not used by the unit template",
                "`enabel` in `[deploy.systemd]` is not used by the unit template",
                "Directives are configured for `[deploy.systemd]`, but the unit template never refers to `directives`",
            ]
        );
    }

    #[test]
    fn broken_config_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        fs::write(
            root.join("Kick.toml"),
            "[repo.\"repos/app\"]\n\
             url = \"https://example.com/app\"\n",
        )
        .unwrap();
        fs::create_dir_all(root.join("repos/app")).unwrap();
        fs::write(root.join("repos/app/Kick.toml"), "[deploy]\nhost = 5\n").unwrap();

        let templating = Templating::new().unwrap();
        let defaults = config::defaults();

        // Other commands fail to load the configuration.
        assert!(config::load(paths(root), &templating, &defaults).is_err());

        let loaded = config::load_all(paths(root), &templating, &defaults);
        assert_eq!(loaded.errors.len(), 1);
        assert!(
            loaded
                .config
                .repos
                .contains_key(RelativePath::new("repos/app"))
        );

        let source = &loaded.sources[1];
        assert_eq!(source.config_path, "repos/app/Kick.toml");
        assert_eq!(source.state, SourceState::Loaded);
        assert_eq!(source.errors.len(), 1);
        assert!(source.errors[0].contains("expected string, got integer"));

        let report = report(root, &Opts::default());
        assert_eq!(report.errors.len(), 1);
        assert!(report.errors[0].contains("repos/app/Kick.toml"));

        // Invalid TOML in the root.
        fs::write(root.join("Kick.toml"), "repo = [\n").unwrap();

        let loaded = config::load_all(paths(root), &templating, &defaults);
        assert_eq!(loaded.errors.len(), 1);
        assert_eq!(loaded.sources.len(), 1);
        assert_eq!(loaded.sources[0].state, SourceState::Invalid);
        assert!(loaded.sources[0].errors[0].contains("TOML parse error"));
        assert!(loaded.config.repos.is_empty());
    }

    #[test]
    fn selection_filters() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        fs::write(
            root.join("Kick.toml"),
            "[repo.\"repos/a\"]\n\
             url = \"https://example.com/a\"\n\
             os = [\"windows\"]\n\
             [repo.\"repos/b\"]\n\
             url = \"https://example.com/b\"\n\
             [repo.\"other/c\"]\n\
             url = \"https://example.com/c\"\n",
        )
        .unwrap();

        let templating = Templating::new().unwrap();
        let defaults = config::defaults();
        let config = config::load(paths(root), &templating, &defaults).unwrap();

        let filter = |opts: &RepoOptions,
                      current: Option<&str>,
                      set: Option<&[&str]>|
         -> Vec<(String, Option<Exclusion>)> {
            let repos = repos(&config);
            let filters = opts
                .repos
                .iter()
                .map(|r| Fragment::parse(r))
                .collect::<Vec<_>>();
            let set = set.map(|s| {
                s.iter()
                    .map(RelativePathBuf::from)
                    .collect::<HashSet<RelativePathBuf>>()
            });

            let exclusions = filter_repos(
                &config,
                current.map(RelativePath::new),
                opts,
                &repos,
                &filters,
                set.as_ref(),
                &Os::Linux,
            )
            .unwrap();

            for (repo, exclusion) in repos.iter().zip(&exclusions) {
                assert_eq!(repo.is_disabled(), exclusion.is_some());
            }

            repos
                .iter()
                .map(|r| r.path().to_string())
                .zip(exclusions)
                .collect()
        };

        let none = RepoOptions::default();

        assert_eq!(
            filter(&none, None, None),
            [
                (String::from("other/c"), None),
                (String::from("repos/a"), None),
                (String::from("repos/b"), None),
            ]
        );

        let paths = RepoOptions {
            repos: vec![String::from("repos/*")],
            ..RepoOptions::default()
        };

        assert_eq!(
            filter(&paths, None, None),
            [
                (String::from("other/c"), Some(Exclusion::NoPathMatch)),
                (String::from("repos/a"), None),
                (String::from("repos/b"), None),
            ]
        );

        assert_eq!(
            filter(&none, Some("repos/b/src"), None),
            [
                (
                    String::from("other/c"),
                    Some(Exclusion::OutsideCurrent("repos/b/src".into()))
                ),
                (
                    String::from("repos/a"),
                    Some(Exclusion::OutsideCurrent("repos/b/src".into()))
                ),
                (String::from("repos/b"), None),
            ]
        );

        assert_eq!(
            filter(&none, None, Some(&["other/c"])),
            [
                (String::from("other/c"), None),
                (String::from("repos/a"), Some(Exclusion::NotInSet)),
                (String::from("repos/b"), Some(Exclusion::NotInSet)),
            ]
        );

        let supported = RepoOptions {
            supported_os: true,
            ..RepoOptions::default()
        };

        assert_eq!(
            filter(&supported, None, None),
            [
                (String::from("other/c"), None),
                (
                    String::from("repos/a"),
                    Some(Exclusion::UnsupportedOs(vec![String::from("Windows")]))
                ),
                (String::from("repos/b"), None),
            ]
        );
    }
}
