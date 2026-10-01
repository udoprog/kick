//! The systemd units installed as part of a deployment.
//!
//! Units are rendered from a [minijinja] template, either a built-in one or one
//! provided by the project being deployed.
//!
//! [minijinja]: https://docs.rs/minijinja

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

    use super::{DEFAULT_SOCKET_TEMPLATE, DEFAULT_TEMPLATE, render, validate};

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
}
