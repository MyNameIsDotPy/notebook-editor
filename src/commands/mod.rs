pub mod create;
pub mod delete;
pub mod edit;
pub mod info;
pub mod read;
pub mod r#move;

use anyhow::Result;
use crate::cli::{Cli, Command};

pub fn dispatch(cli: Cli) -> Result<()> {
    match cli.command {
        Command::Read {
            notebook,
            selection,
            r#type,
            show_outputs,
            json,
        } => read::run(
            &notebook,
            &selection,
            r#type.as_ref().map(|t| t.as_str()),
            show_outputs,
            json,
        ),

        Command::Create {
            notebook,
            r#type,
            at,
            source,
            file,
        } => create::run(
            &notebook,
            r#type.as_str(),
            at,
            source,
            file,
            cli.backup,
            cli.quiet,
        ),

        Command::Edit {
            notebook,
            index,
            source,
            file,
            editor,
            r#type,
        } => edit::run(
            &notebook,
            index,
            source,
            file,
            editor,
            r#type.as_ref().map(|t| t.as_str()),
            cli.backup,
            cli.quiet,
        ),

        Command::Delete {
            notebook,
            selection,
            dry_run,
        } => delete::run(&notebook, &selection, dry_run, cli.backup, cli.quiet),

        Command::Move {
            notebook,
            selection,
            to,
        } => r#move::run(&notebook, &selection, &to, cli.backup, cli.quiet),

        Command::Info { notebook } => info::run(&notebook),
    }
}
