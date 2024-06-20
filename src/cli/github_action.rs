use anyhow::Result;
use clap::Parser;
use termcolor::{ColorChoice, StandardStream};

use crate::command_system::{ActionConfig, Actions, Batch, BatchConfig, BatchOptions};
use crate::ctxt::Ctxt;
use crate::model::Repo;
use crate::rstr::RStr;
use crate::shell::Shell;

#[derive(Default, Debug, Parser)]
pub(crate) struct Opts {
    #[command(flatten)]
    batch_opts: BatchOptions,
    /// The default shell to use when printing command invocations.
    ///
    /// By default this is `bash` for unix-like environments and `powershell`
    /// for windows.
    #[arg(long, value_name = "shell")]
    shell: Option<Shell>,
    /// The workflow to run.
    #[arg(value_name = "id")]
    id: String,
}

pub(crate) fn entry(cx: &mut Ctxt<'_>, opts: &Opts) -> Result<()> {
    let mut o = StandardStream::stdout(ColorChoice::Auto);

    with_repos!(
        cx,
        "run action",
        format_args!("for: {opts:?}"),
        |cx, repo| { action(&mut o, cx, repo, opts) }
    );

    Ok(())
}

#[tracing::instrument(skip_all)]
fn action(o: &mut StandardStream, cx: &Ctxt<'_>, repo: &Repo, opts: &Opts) -> Result<()> {
    let repo_path = cx.to_path(repo.path());

    let mut actions = Actions::default();
    actions.add_action(&opts.id)?;

    let runners = actions.synchronize(cx)?;
    let default_shell = opts.shell.unwrap_or_else(|| cx.os.shell());

    let c = ActionConfig::default();

    let (mut main, post) = runners.build(RStr::new(opts.id.as_str()), &c)?;
    main.extend(post);

    let batch = Batch::with_commands(main);

    let mut c = BatchConfig::new(cx, &repo_path, default_shell);
    c.add_opts(&opts.batch_opts)?;

    batch.commit(o, &c)?;
    Ok(())
}
