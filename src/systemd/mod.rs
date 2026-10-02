//! The systemd units installed as part of a deployment.
//!
//! Units are rendered from a [minijinja] template, either a built-in one or one
//! provided by the project being deployed.
//!
//! [minijinja]: https://docs.rs/minijinja

use std::collections::{HashMap, HashSet};
use std::fmt;

use anyhow::{Result, anyhow};
use serde::Serialize;

/// The template used when a deployment asks for a unit without providing one.
pub(crate) const DEFAULT_TEMPLATE: &str = include_str!("default.service");

/// The template used when a deployment asks for a socket unit without
/// providing one.
pub(crate) const DEFAULT_SOCKET_TEMPLATE: &str = include_str!("default.socket");

/// The name a template is registered under, which is what shows up in errors.
const NAME: &str = "unit";

/// Check that the given template can be compiled.
///
/// This is done when the configuration is loaded so that a broken template is
/// reported up front instead of in the middle of a deployment.
pub(crate) fn validate(source: &str) -> Result<()> {
    let mut env = env();
    env.add_template(NAME, source)?;
    Ok(())
}

/// Render the given template.
pub(crate) fn render<T>(source: &str, data: &T) -> Result<String>
where
    T: Serialize,
{
    let mut env = env();
    env.add_template(NAME, source)?;

    let template = env
        .get_template(NAME)
        .map_err(|error| anyhow!("Missing template: {error}"))?;

    Ok(template.render(data)?)
}

/// Which kind of unit is being rendered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UnitKind {
    /// The service unit.
    Service,
    /// The socket unit which activates the service.
    Socket,
}

impl UnitKind {
    /// The pass-through directive tables of this kind of unit, which are named
    /// after the section of the unit file they are written into.
    pub(crate) fn sections(self) -> &'static [&'static str] {
        match self {
            UnitKind::Service => &["unit", "service", "install"],
            UnitKind::Socket => &["unit", "socket", "install"],
        }
    }

    /// The built-in template for this kind of unit.
    pub(crate) fn default_template(self) -> &'static str {
        match self {
            UnitKind::Service => DEFAULT_TEMPLATE,
            UnitKind::Socket => DEFAULT_SOCKET_TEMPLATE,
        }
    }

    /// Variables of the built-in template which fill in a directive, as
    /// `(variable, section, directive)`. A pass-through directive replaces
    /// what the variable would have produced.
    fn conveniences(self) -> &'static [(&'static str, &'static str, &'static str)] {
        match self {
            UnitKind::Service => &[
                ("description", "unit", "Description"),
                ("after", "unit", "After"),
                ("wants", "unit", "Wants"),
                ("requires", "unit", "Requires"),
                ("start_limit_interval_sec", "unit", "StartLimitIntervalSec"),
                ("start_limit_burst", "unit", "StartLimitBurst"),
                ("type", "service", "Type"),
                ("user", "service", "User"),
                ("group", "service", "Group"),
                ("working_directory", "service", "WorkingDirectory"),
                ("kill_signal", "service", "KillSignal"),
                ("environment", "service", "Environment"),
                ("environment_file", "service", "EnvironmentFile"),
                ("exec", "service", "ExecStart"),
                ("args", "service", "ExecStart"),
                ("restart", "service", "Restart"),
                ("restart_sec", "service", "RestartSec"),
                ("timeout_stop_sec", "service", "TimeoutStopSec"),
                ("wanted_by", "install", "WantedBy"),
            ],
            UnitKind::Socket => &[
                ("description", "unit", "Description"),
                ("listen_stream", "socket", "ListenStream"),
                ("socket_user", "socket", "SocketUser"),
                ("socket_group", "socket", "SocketGroup"),
                ("socket_mode", "socket", "SocketMode"),
                ("directory_mode", "socket", "DirectoryMode"),
                ("remove_on_stop", "socket", "RemoveOnStop"),
                ("wanted_by", "install", "WantedBy"),
            ],
        }
    }
}

/// Directives which are written verbatim into a section of a unit, keyed by
/// the name of the table they are configured in such as `service`.
///
/// Each directive is stored as a list of the values it is written with, one
/// line per value.
#[derive(Default, Debug, Clone)]
pub(crate) struct Directives {
    sections: toml::Table,
}

impl Directives {
    /// Add the directives of a section, replacing any directive which is
    /// already defined.
    pub(crate) fn insert(&mut self, section: &str, directives: toml::Table) {
        let toml::Value::Table(existing) = self
            .sections
            .entry(section)
            .or_insert_with(|| toml::Value::Table(toml::Table::new()))
        else {
            unreachable!("sections only contain tables");
        };

        // NB: A more specific layer replaces a directive rather than adding to
        // it, since there would otherwise be no way to get rid of a line. An
        // empty string is written as `Directive=`, which is how systemd resets
        // a list.
        for (key, value) in directives {
            existing.insert(key, value);
        }
    }

    pub(crate) fn merge_with(&mut self, other: Self) {
        for (section, value) in other.sections {
            if let toml::Value::Table(table) = value {
                self.insert(&section, table);
            }
        }
    }

    /// The directives configured for the given section.
    pub(crate) fn section(&self, section: &str) -> Option<&toml::Table> {
        self.sections.get(section).and_then(toml::Value::as_table)
    }

    /// Test if the given directive is set in the given section.
    pub(crate) fn contains(&self, section: &str, directive: &str) -> bool {
        self.section(section)
            .is_some_and(|table| table.contains_key(directive))
    }

    /// Remove a directive.
    pub(crate) fn remove(&mut self, section: &str, directive: &str) {
        if let Some(toml::Value::Table(table)) = self.sections.get_mut(section) {
            table.remove(directive);
        }
    }

    /// Test if no directives are set.
    pub(crate) fn is_empty(&self) -> bool {
        self.sections
            .values()
            .all(|value| value.as_table().is_none_or(toml::Table::is_empty))
    }

    /// Iterate over every directive as `(section, directive, values)`, in the
    /// order of the sections of the given kind of unit and the order they are
    /// written in, which is sorted by name within a section.
    pub(crate) fn iter(
        &self,
        kind: UnitKind,
    ) -> impl Iterator<Item = (&'static str, &str, Vec<&str>)> + '_ {
        kind.sections().iter().flat_map(move |section| {
            let mut directives = self
                .section(section)
                .into_iter()
                .flatten()
                .map(move |(key, value)| (*section, key.as_str(), values(value)))
                .collect::<Vec<_>>();

            // NB: Templates see the directives as a map, which minijinja
            // iterates over in sorted order.
            directives.sort_by(|a, b| a.1.cmp(b.1));
            directives
        })
    }

    /// The directives as they are handed to a template, with every section of
    /// the given kind of unit present so that a template can refer to it
    /// without checking whether it is defined.
    pub(crate) fn to_value(&self, kind: UnitKind) -> toml::Value {
        let mut out = toml::Table::new();

        for section in kind.sections() {
            let table = self.section(section).cloned().unwrap_or_default();
            out.insert((*section).to_owned(), toml::Value::Table(table));
        }

        toml::Value::Table(out)
    }
}

fn values(value: &toml::Value) -> Vec<&str> {
    match value {
        toml::Value::Array(values) => values.iter().filter_map(toml::Value::as_str).collect(),
        value => value.as_str().into_iter().collect(),
    }
}

/// Validate a pass-through directive and convert its value into the lines it
/// is written as.
pub(crate) fn directive(key: &str, value: toml::Value) -> Result<toml::Value, String> {
    let valid = key.starts_with(|c: char| c.is_ascii_uppercase())
        && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');

    if !valid {
        return Err(format!(
            "`{key}` is not a systemd directive, which is spelled the way it is in a unit file such as `LimitNOFILE`"
        ));
    }

    fn scalar(value: toml::Value) -> Result<String, String> {
        let value = match value {
            toml::Value::String(value) => value,
            toml::Value::Integer(value) => value.to_string(),
            toml::Value::Float(value) => value.to_string(),
            toml::Value::Boolean(value) => String::from(if value { "yes" } else { "no" }),
            other => {
                return Err(format!(
                    "expected a string, number, boolean, or a list of them, got {}",
                    other.type_str()
                ));
            }
        };

        if value.contains(['\n', '\r']) {
            return Err(String::from("a directive value cannot span multiple lines"));
        }

        Ok(value)
    }

    let values = match value {
        toml::Value::Array(values) => values
            .into_iter()
            .map(scalar)
            .collect::<Result<Vec<_>, _>>()?,
        value => vec![scalar(value)?],
    };

    Ok(toml::Value::Array(
        values.into_iter().map(toml::Value::String).collect(),
    ))
}

/// Why the given variable cannot be configured for the given kind of unit,
/// since it is one kick provides to the template.
///
/// `exec` is the exception, which can be configured to start the binary
/// through a wrapper.
pub(crate) fn reserved(kind: UnitKind, key: &str) -> Option<String> {
    let reason = match key {
        "binary" => {
            "`binary` is provided to the template as the name of the binary being deployed, set `binary` in the `[build]` section instead"
        }
        "bin_dir" => {
            "`bin_dir` is provided to the template as the directory the binary is installed into, set the `bin_dir` option of the deployment instead"
        }
        "unit_dir" => {
            "`unit_dir` is provided to the template as the directory the unit is installed into, set the `unit_dir` option of the deployment instead"
        }
        "host" => {
            "`host` is provided to the template as the host the unit is installed on, set the `host` option of the deployment or use `--host` instead"
        }
        "directives" => {
            "`directives` is provided to the template as the pass-through directives, which are set in the tables named after the sections of the unit such as `[deploy.systemd.service]`"
        }
        "service" if kind == UnitKind::Socket => {
            "`service` is provided to the socket template as the file name of the service it activates, which is named by `name` in the systemd section"
        }
        "scope" if kind == UnitKind::Socket => {
            "`scope` is set in the systemd section, and applies to both the service and the socket"
        }
        _ => return None,
    };

    Some(String::from(reason))
}

/// Variables which are provided by kick, but which can be configured to
/// override what kick would have provided.
const OVERRIDABLE: &[&str] = &["exec"];

/// Where a template variable comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Origin {
    /// The global `[variables]` section.
    Global,
    /// The systemd section of the deployment.
    Configured,
    /// Provided by kick.
    BuiltIn,
    /// A command line option such as `--service-user`.
    Option,
}

impl Origin {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Origin::Global => "global",
            Origin::Configured => "configured",
            Origin::BuiltIn => "built-in",
            Origin::Option => "option",
        }
    }
}

impl fmt::Display for Origin {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_str().fmt(f)
    }
}

/// The variables a unit template is rendered with, along with where each of
/// them came from.
#[derive(Default, Debug, Clone)]
pub(crate) struct Variables {
    values: toml::Table,
    origins: HashMap<String, Origin>,
}

impl Variables {
    /// Add every variable in the given table, replacing what is already there.
    pub(crate) fn extend(&mut self, table: &toml::Table, origin: Origin) {
        for (key, value) in table {
            self.insert(key, value.clone(), origin);
        }
    }

    /// Insert a variable, replacing what is already there.
    pub(crate) fn insert(&mut self, key: &str, value: toml::Value, origin: Origin) {
        self.values.insert(key.to_owned(), value);
        self.origins.insert(key.to_owned(), origin);
    }

    /// Insert a variable provided by kick, which replaces anything but an
    /// [overridable] variable which has been configured.
    ///
    /// [overridable]: OVERRIDABLE
    pub(crate) fn provide(&mut self, key: &str, value: toml::Value) {
        if OVERRIDABLE.contains(&key) && self.origin(key) == Some(Origin::Configured) {
            return;
        }

        self.insert(key, value, Origin::BuiltIn);
    }

    /// Get where a variable came from.
    pub(crate) fn origin(&self, key: &str) -> Option<Origin> {
        self.origins.get(key).copied()
    }

    /// Iterate over every variable as `(name, value, origin)`.
    pub(crate) fn iter(&self) -> impl Iterator<Item = (&str, &toml::Value, Origin)> + '_ {
        self.values.iter().map(|(key, value)| {
            let origin = self.origins.get(key).copied().unwrap_or(Origin::BuiltIn);
            (key.as_str(), value, origin)
        })
    }

    /// Render the given template with these variables and the given
    /// pass-through directives.
    pub(crate) fn render(
        &self,
        source: &str,
        kind: UnitKind,
        directives: &Directives,
    ) -> Result<String> {
        let mut data = self.values.clone();
        data.insert(String::from("directives"), directives.to_value(kind));
        render(source, &data)
    }
}

/// The variables a unit template is rendered with before kick adds the ones it
/// provides.
///
/// Global variables are only visible to a custom template, since the built-in
/// templates have no use for them and names like `description` or `user` tend
/// to mean something else in the global `[variables]` section.
pub(crate) fn configured(
    template: Option<&str>,
    globals: &toml::Table,
    configured: &toml::Table,
) -> Variables {
    let mut variables = Variables::default();

    if template.is_some() {
        variables.extend(globals, Origin::Global);
    }

    variables.extend(configured, Origin::Configured);
    variables
}

/// The top-level variables the given template refers to.
fn referenced(source: &str) -> Result<HashSet<String>> {
    let mut env = env();
    env.add_template(NAME, source)?;

    let template = env
        .get_template(NAME)
        .map_err(|error| anyhow!("Missing template: {error}"))?;

    Ok(template.undeclared_variables(false))
}

/// Find configuration which would not end up in the rendered unit, such as a
/// misspelled variable which the template never uses.
///
/// `label` is how the section the unit is configured in is referred to, such as
/// `[deploy.systemd]`. `template` is the source of a custom template, or `None`
/// for the built-in template.
pub(crate) fn lint(
    kind: UnitKind,
    label: &str,
    template: Option<&str>,
    configured: &toml::Table,
    directives: &Directives,
) -> Vec<String> {
    let mut warnings = Vec::new();

    let source = template.unwrap_or(kind.default_template());

    let what = match template {
        Some(..) => "the unit template",
        None => "the built-in template",
    };

    let Ok(used) = referenced(source) else {
        return warnings;
    };

    for key in configured.keys() {
        if used.contains(key) {
            continue;
        }

        let mut warning = format!("`{key}` in `{label}` is not used by {what}");

        if key.starts_with(|c: char| c.is_ascii_uppercase()) {
            let sections = kind
                .sections()
                .iter()
                .map(|section| format!("`{}`", table(label, section)))
                .collect::<Vec<_>>();

            warning.push_str(&format!(
                ", systemd directives belong in one of {}",
                sections.join(", ")
            ));
        }

        warnings.push(warning);
    }

    match template {
        None => {
            for (variable, section, directive) in kind.conveniences() {
                if configured.contains_key(*variable) && directives.contains(section, directive) {
                    warnings.push(format!(
                        "`{variable}` in `{label}` is ignored since `{directive}` is set in `{}`",
                        table(label, section)
                    ));
                }
            }
        }
        Some(..) => {
            if !directives.is_empty() && !used.contains("directives") {
                warnings.push(format!(
                    "Directives are configured for `{label}`, but the unit template never refers to `directives`"
                ));
            }
        }
    }

    warnings
}

/// The title of the section of a unit file which a directive table is written
/// into, such as `Service` for `service`.
pub(crate) fn section_title(section: &str) -> &str {
    match section {
        "unit" => "Unit",
        "service" => "Service",
        "socket" => "Socket",
        "install" => "Install",
        other => other,
    }
}

/// The name of a directive table, such as `[deploy.systemd.service]` for the
/// `service` section of `[deploy.systemd]`.
pub(crate) fn table(label: &str, section: &str) -> String {
    let base = label.trim_start_matches('[').trim_end_matches(']');
    format!("[{base}.{section}]")
}

fn env() -> minijinja::Environment<'static> {
    let mut env = minijinja::Environment::new();
    // NB: Unit files are not markup, so values are used exactly as given.
    env.set_auto_escape_callback(|_| minijinja::AutoEscape::None);
    // NB: Quietly substituting an empty string for a variable which hasn't been
    // defined would produce a broken unit and install it on a server, so
    // templates have to be explicit about what is optional through `default()`
    // or `is defined`.
    env.set_undefined_behavior(minijinja::UndefinedBehavior::Strict);
    // NB: Unit files are expected to end with a newline like any other text
    // file, and minijinja would otherwise eat the one the template ends with.
    env.set_keep_trailing_newline(true);
    env
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{
        DEFAULT_SOCKET_TEMPLATE, DEFAULT_TEMPLATE, Directives, Origin, UnitKind, Variables,
        configured, directive, lint, render, reserved, validate,
    };

    fn table(source: &str) -> toml::Table {
        toml::from_str(source).unwrap()
    }

    /// Parse directive tables the way the configuration does.
    fn directives(source: &str) -> Directives {
        let mut directives = Directives::default();

        for (section, value) in table(source) {
            let toml::Value::Table(value) = value else {
                panic!("expected table");
            };

            let mut out = toml::Table::new();

            for (key, value) in value {
                out.insert(key.clone(), directive(&key, value).unwrap());
            }

            directives.insert(&section, out);
        }

        directives
    }

    fn service(configured: &str, directives: &Directives) -> String {
        let mut variables = Variables::default();
        variables.extend(&table(configured), Origin::Configured);
        variables.provide("name", toml::Value::from("track"));
        variables.provide("exec", toml::Value::from("/usr/local/bin/track"));
        variables.provide("scope", toml::Value::from("system"));
        variables
            .render(DEFAULT_TEMPLATE, UnitKind::Service, directives)
            .unwrap()
    }

    /// The built-in template has to render with nothing but the variables which
    /// are always defined, since everything else is optional.
    #[test]
    fn default_template_is_self_sufficient() {
        validate(DEFAULT_TEMPLATE).unwrap();

        let ctx = BTreeMap::from([("name", "track"), ("exec", "/usr/local/bin/track")]);

        let unit = render(DEFAULT_TEMPLATE, &ctx).unwrap();

        assert!(unit.contains("Description=track service"));
        assert!(unit.contains("Type=simple"));
        assert!(unit.contains("ExecStart=/usr/local/bin/track\n"));
        assert!(unit.contains("WantedBy=multi-user.target"));
        assert!(!unit.contains("User="));
        assert!(unit.ends_with("\n"));
    }

    /// Optional variables fill in the directives they belong to.
    #[test]
    fn default_template_takes_variables() {
        let ctx = BTreeMap::from([
            ("name", minijinja::Value::from("track")),
            ("exec", minijinja::Value::from("/usr/local/bin/track")),
            ("user", minijinja::Value::from("track")),
            (
                "args",
                minijinja::Value::from(vec!["--bind", "0.0.0.0:3004"]),
            ),
            (
                "environment",
                minijinja::Value::from(BTreeMap::from([("RUST_LOG", "info")])),
            ),
        ]);

        let unit = render(DEFAULT_TEMPLATE, &ctx).unwrap();

        assert!(unit.contains("User=track"));
        assert!(unit.contains("Environment=RUST_LOG=info"));
        assert!(unit.contains("ExecStart=/usr/local/bin/track --bind 0.0.0.0:3004\n"));
    }

    /// A user unit has no use for the network targets of the system instance,
    /// and is wanted by the default target of the user instance.
    #[test]
    fn default_template_user_scope() {
        let ctx = BTreeMap::from([
            ("name", "kanban"),
            ("exec", "/home/me/.cargo/bin/kanban"),
            ("scope", "user"),
        ]);

        let unit = render(DEFAULT_TEMPLATE, &ctx).unwrap();

        assert!(!unit.contains("After="), "{unit}");
        assert!(!unit.contains("Wants="), "{unit}");
        assert!(!unit.contains("User="), "{unit}");
        assert!(unit.contains("WantedBy=default.target\n"), "{unit}");

        let ctx = BTreeMap::from([
            ("name", "kanban"),
            ("exec", "/home/me/.cargo/bin/kanban"),
            ("scope", "user"),
            ("after", "network.target"),
            ("wanted_by", "graphical-session.target"),
        ]);

        let unit = render(DEFAULT_TEMPLATE, &ctx).unwrap();

        assert!(unit.contains("After=network.target\n"), "{unit}");
        assert!(!unit.contains("Wants="), "{unit}");
        assert!(
            unit.contains("WantedBy=graphical-session.target\n"),
            "{unit}"
        );

        let ctx = BTreeMap::from([
            ("name", "track"),
            ("exec", "/usr/local/bin/track"),
            ("scope", "system"),
        ]);

        let unit = render(DEFAULT_TEMPLATE, &ctx).unwrap();

        assert!(unit.contains("After=network-online.target\n"), "{unit}");
        assert!(unit.contains("Wants=network-online.target\n"), "{unit}");
        assert!(unit.contains("WantedBy=multi-user.target\n"), "{unit}");
    }

    /// A service activated by a socket requires it and is ordered after it.
    #[test]
    fn default_template_with_socket() {
        let ctx = BTreeMap::from([
            ("name", "kanban"),
            ("exec", "/home/me/.cargo/bin/kanban"),
            ("scope", "user"),
            ("socket", "kanban.socket"),
        ]);

        let unit = render(DEFAULT_TEMPLATE, &ctx).unwrap();

        assert!(unit.contains("Requires=kanban.socket\n"), "{unit}");
        assert!(unit.contains("After=kanban.socket\n"), "{unit}");
    }

    /// The built-in socket template only needs what to listen on.
    #[test]
    fn default_socket_template() {
        validate(DEFAULT_SOCKET_TEMPLATE).unwrap();

        let ctx = BTreeMap::from([
            ("name", minijinja::Value::from("kanban")),
            ("service", minijinja::Value::from("kanban.service")),
            (
                "listen_stream",
                minijinja::Value::from("%t/kanban/kanban.sock"),
            ),
        ]);

        let unit = render(DEFAULT_SOCKET_TEMPLATE, &ctx).unwrap();

        let expected = "\
[Unit]
Description=kanban socket

[Socket]
ListenStream=%t/kanban/kanban.sock

[Install]
WantedBy=sockets.target
";

        assert_eq!(unit, expected);

        let ctx = BTreeMap::from([
            ("name", minijinja::Value::from("kanban-api")),
            ("service", minijinja::Value::from("kanban.service")),
            (
                "listen_stream",
                minijinja::Value::from(vec!["/run/kanban/kanban.sock", "127.0.0.1:3000"]),
            ),
            ("socket_mode", minijinja::Value::from("0660")),
            ("socket_group", minijinja::Value::from("kanban")),
            ("directory_mode", minijinja::Value::from("0750")),
            ("remove_on_stop", minijinja::Value::from(true)),
        ]);

        let unit = render(DEFAULT_SOCKET_TEMPLATE, &ctx).unwrap();

        let expected = "\
[Unit]
Description=kanban-api socket

[Socket]
ListenStream=/run/kanban/kanban.sock
ListenStream=127.0.0.1:3000
SocketGroup=kanban
SocketMode=0660
DirectoryMode=0750
RemoveOnStop=yes
Service=kanban.service

[Install]
WantedBy=sockets.target
";

        assert_eq!(unit, expected);

        // Without anything to listen on the socket is broken.
        let ctx = BTreeMap::from([("name", "kanban"), ("service", "kanban.service")]);
        assert!(render(DEFAULT_SOCKET_TEMPLATE, &ctx).is_err());
    }

    /// Referring to something undefined is an error rather than an empty
    /// substitution which would produce a broken unit.
    #[test]
    fn undefined_variables_are_an_error() {
        let ctx = BTreeMap::<&str, &str>::new();
        assert!(render("ExecStart={{ exec }}", &ctx).is_err());
    }

    /// Pass-through directives are written at the end of their section, one
    /// line per value, with booleans as `yes` or `no`.
    #[test]
    fn default_template_directives() {
        let directives = directives(
            r#"
[unit]
After = ["network-online.target", "postgresql.service"]
StartLimitBurst = 3

[service]
LimitNOFILE = 65536
ProtectSystem = "strict"
ExecStartPre = ["/usr/local/bin/track migrate", "/usr/local/bin/track check"]
NoNewPrivileges = true
PrivateTmp = false

[install]
Alias = "tracker.service"
"#,
        );

        let unit = service(r#"user = "track""#, &directives);

        let expected = "\
[Unit]
Description=track service
Wants=network-online.target
After=network-online.target
After=postgresql.service
StartLimitBurst=3

[Service]
Type=simple
User=track
ExecStart=/usr/local/bin/track
Restart=always
RestartSec=5
ExecStartPre=/usr/local/bin/track migrate
ExecStartPre=/usr/local/bin/track check
LimitNOFILE=65536
NoNewPrivileges=yes
PrivateTmp=no
ProtectSystem=strict

[Install]
WantedBy=multi-user.target
Alias=tracker.service
";

        assert_eq!(unit, expected);
    }

    /// A pass-through directive replaces what a variable or a default would
    /// have produced, except for the ordering on a socket which is always
    /// added.
    #[test]
    fn default_template_directives_win() {
        let directives = directives(
            r#"
[unit]
After = "postgresql.service"
Description = "Overridden"

[service]
User = "other"
Environment = ["A=1", "B=2"]
ExecStart = ["", "/usr/bin/wrapper /usr/local/bin/track"]
Restart = "on-failure"

[install]
WantedBy = ["multi-user.target", "graphical.target"]
"#,
        );

        let unit = service(
            r#"
description = "Track"
user = "track"
environment = { RUST_LOG = "info" }
args = ["--bind", "0.0.0.0:3004"]
socket = "track.socket"
"#,
            &directives,
        );

        let expected = "\
[Unit]
Wants=network-online.target
Requires=track.socket
After=track.socket
After=postgresql.service
Description=Overridden

[Service]
Type=simple
RestartSec=5
Environment=A=1
Environment=B=2
ExecStart=
ExecStart=/usr/bin/wrapper /usr/local/bin/track
Restart=on-failure
User=other

[Install]
WantedBy=multi-user.target
WantedBy=graphical.target
";

        assert_eq!(unit, expected);
    }

    /// The socket template takes pass-through directives too, and doesn't
    /// need `listen_stream` when `ListenStream` is given.
    #[test]
    fn default_socket_template_directives() {
        let directives = directives(
            r#"
[unit]
PartOf = "kanban.service"

[socket]
ListenStream = ["/run/kanban/kanban.sock", "127.0.0.1:3000"]
SocketMode = "0660"
Accept = false

[install]
WantedBy = "sockets.target"
"#,
        );

        let mut variables = Variables::default();
        variables.extend(&table(r#"socket_mode = "0600""#), Origin::Configured);
        variables.provide("name", toml::Value::from("kanban"));
        variables.provide("service", toml::Value::from("kanban.service"));

        let unit = variables
            .render(DEFAULT_SOCKET_TEMPLATE, UnitKind::Socket, &directives)
            .unwrap();

        let expected = "\
[Unit]
Description=kanban socket
PartOf=kanban.service

[Socket]
Accept=no
ListenStream=/run/kanban/kanban.sock
ListenStream=127.0.0.1:3000
SocketMode=0660

[Install]
WantedBy=sockets.target
";

        assert_eq!(unit, expected);
    }

    /// Without any directives the templates render exactly what they did
    /// before directives existed.
    #[test]
    fn default_template_without_directives() {
        let unit = service(
            r#"
description = "Track Service"
user = "track"
group = "track"
working_directory = "~"
kill_signal = "SIGINT"
start_limit_interval_sec = "60s"
start_limit_burst = 3
timeout_stop_sec = "5min"
args = ["--bind", "0.0.0.0:3004"]
"#,
            &Directives::default(),
        );

        let expected = "\
[Unit]
Description=Track Service
After=network-online.target
Wants=network-online.target
StartLimitIntervalSec=60s
StartLimitBurst=3

[Service]
Type=simple
User=track
Group=track
WorkingDirectory=~
KillSignal=SIGINT
ExecStart=/usr/local/bin/track --bind 0.0.0.0:3004
Restart=always
RestartSec=5
TimeoutStopSec=5min

[Install]
WantedBy=multi-user.target
";

        assert_eq!(unit, expected);
    }

    /// A custom template sees the directives of every section, even those
    /// which aren't configured.
    #[test]
    fn custom_template_directives() {
        let template = "\
[Service]
{%- for key, values in directives.service | items %}
{%- for value in values %}
{{ key }}={{ value }}
{%- endfor %}
{%- endfor %}
{{ directives.unit | length }}
";

        let directives = directives("service = { LimitNOFILE = 1024 }");

        let unit = Variables::default()
            .render(template, UnitKind::Service, &directives)
            .unwrap();

        assert_eq!(unit, "[Service]\nLimitNOFILE=1024\n0\n");
    }

    #[test]
    fn directive_values() {
        assert_eq!(
            directive("LimitNOFILE", toml::Value::from(1024)).unwrap(),
            toml::Value::from(vec!["1024"])
        );
        assert_eq!(
            directive("X-Custom", toml::Value::from(true)).unwrap(),
            toml::Value::from(vec!["yes"])
        );
        assert_eq!(
            directive("ExecStartPre", toml::Value::from(vec!["a", "b"])).unwrap(),
            toml::Value::from(vec!["a", "b"])
        );

        // Not spelled the way systemd spells it.
        assert!(directive("limit_nofile", toml::Value::from(1)).is_err());
        assert!(directive("Limit NOFILE", toml::Value::from(1)).is_err());
        assert!(directive("", toml::Value::from(1)).is_err());
        // Something which cannot be written as a line.
        assert!(directive("Environment", toml::Value::Table(table("A = 1"))).is_err());
        assert!(
            directive(
                "ExecStartPre",
                toml::Value::Array(vec![toml::Value::from(vec!["nested"])])
            )
            .is_err()
        );
        assert!(directive("ExecStart", toml::Value::from("a\nExecStart=b")).is_err());
    }

    /// Profiles replace a directive rather than adding to it.
    #[test]
    fn directives_merge() {
        let mut base = directives(
            r#"
[service]
ExecStartPre = ["a", "b"]
LimitNOFILE = 1024
"#,
        );

        base.merge_with(directives(
            r#"
[service]
ExecStartPre = "c"
"#,
        ));

        let merged = base.iter(UnitKind::Service).collect::<Vec<_>>();

        assert_eq!(
            merged,
            [
                ("service", "ExecStartPre", vec!["c"]),
                ("service", "LimitNOFILE", vec!["1024"]),
            ]
        );
    }

    /// Only `exec` among the variables kick provides can be overridden.
    #[test]
    fn provided_variables() {
        let mut variables = Variables::default();
        variables.extend(
            &table(r#"exec = "/usr/bin/wrapper /usr/local/bin/track""#),
            Origin::Configured,
        );
        variables.provide("exec", toml::Value::from("/usr/local/bin/track"));
        assert_eq!(variables.origin("exec"), Some(Origin::Configured));

        let mut variables = Variables::default();
        variables.extend(&table(r#"exec = "global""#), Origin::Global);
        variables.provide("exec", toml::Value::from("/usr/local/bin/track"));
        assert_eq!(variables.origin("exec"), Some(Origin::BuiltIn));

        for key in ["binary", "bin_dir", "unit_dir", "host", "directives"] {
            assert!(reserved(UnitKind::Service, key).is_some(), "{key}");
            assert!(reserved(UnitKind::Socket, key).is_some(), "{key}");
        }

        assert!(reserved(UnitKind::Service, "exec").is_none());
        assert!(reserved(UnitKind::Socket, "service").is_some());
        assert!(reserved(UnitKind::Socket, "scope").is_some());
        assert!(reserved(UnitKind::Service, "user").is_none());
    }

    /// Global variables are visible to a custom template underneath the
    /// configured ones, but not to the built-in template.
    #[test]
    fn global_variables() {
        let globals = table(
            r#"
description = "Global"
motd = "hello"
"#,
        );

        let local = table(r#"description = "Local""#);

        let custom = "{{ description }} {{ motd }}";
        let variables = configured(Some(custom), &globals, &local);
        assert_eq!(variables.origin("motd"), Some(Origin::Global));
        assert_eq!(variables.origin("description"), Some(Origin::Configured));

        let unit = variables
            .render(custom, UnitKind::Service, &Directives::default())
            .unwrap();
        assert_eq!(unit, "Local hello");

        let variables = configured(None, &globals, &table(""));
        assert_eq!(variables.origin("motd"), None);
        assert_eq!(variables.origin("description"), None);
    }

    #[test]
    fn lint_warnings() {
        let configured = table(
            r#"
user = "track"
enabel = true
LimitNOFILE = 1024
"#,
        );

        let directives = directives("service = { User = \"other\" }");

        let warnings = lint(
            UnitKind::Service,
            "[deploy.systemd]",
            None,
            &configured,
            &directives,
        );

        assert_eq!(
            warnings,
            [
                "`enabel` in `[deploy.systemd]` is not used by the built-in template",
                "`LimitNOFILE` in `[deploy.systemd]` is not used by the built-in template, systemd directives belong in one of `[deploy.systemd.unit]`, `[deploy.systemd.service]`, `[deploy.systemd.install]`",
                "`user` in `[deploy.systemd]` is ignored since `User` is set in `[deploy.systemd.service]`",
            ]
        );

        // A custom template only uses what it refers to, and has to refer to
        // `directives` for them to be written.
        let warnings = lint(
            UnitKind::Socket,
            "[deploy.systemd.socket]",
            Some("ListenStream={{ listen_stream }}"),
            &table(
                r#"
listen_stream = "/run/x.sock"
socket_mode = "0600"
"#,
            ),
            &directives,
        );

        assert_eq!(
            warnings,
            [
                "`socket_mode` in `[deploy.systemd.socket]` is not used by the unit template",
                "Directives are configured for `[deploy.systemd.socket]`, but the unit template never refers to `directives`",
            ]
        );

        // Everything the built-in templates document is used.
        let all = table(
            r#"
description = ""
after = ""
wants = ""
requires = ""
start_limit_interval_sec = ""
start_limit_burst = ""
type = ""
user = ""
group = ""
working_directory = ""
kill_signal = ""
environment = {}
environment_file = ""
exec = ""
args = []
restart = ""
restart_sec = ""
timeout_stop_sec = ""
wanted_by = ""
"#,
        );

        assert!(
            lint(
                UnitKind::Service,
                "[deploy.systemd]",
                None,
                &all,
                &Directives::default()
            )
            .is_empty()
        );

        let all = table(
            r#"
description = ""
listen_stream = ""
socket_user = ""
socket_group = ""
socket_mode = ""
directory_mode = ""
remove_on_stop = ""
wanted_by = ""
"#,
        );

        assert!(
            lint(
                UnitKind::Socket,
                "[deploy.systemd.socket]",
                None,
                &all,
                &Directives::default()
            )
            .is_empty()
        );
    }
}
