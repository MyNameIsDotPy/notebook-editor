pub mod bookmark;
pub mod clear;
pub mod copy;
pub mod create;
pub mod delete;
pub mod diff;
pub mod duplicate;
pub mod edit;
pub mod export;
pub mod ids;
pub mod info;
pub mod kernels;
pub mod merge;
pub mod r#move;
pub mod query;
pub mod read;
pub mod refs;
pub mod rename_id;
pub mod render;
pub mod replace;
pub mod run;
pub mod search;
pub mod session;
pub mod session_client;
pub mod split;
pub mod strip;
pub mod validate;

use crate::cli::{BookmarkAction, Cli, Command, SessionAction};
use anyhow::Result;

pub fn dispatch(cli: Cli) -> Result<()> {
    match cli.command {
        Command::Read {
            notebook,
            selection,
            r#type,
            show_outputs,
            only_outputs,
            max_output_chars,
            json,
            lines,
            full_output,
            output_lines,
        } => read::run(
            &notebook,
            &selection,
            r#type.as_ref().map(|t| t.as_str()),
            show_outputs,
            only_outputs,
            max_output_chars,
            json,
            lines.as_deref(),
            if full_output {
                None
            } else {
                Some(output_lines.unwrap_or(crate::output_limit::DEFAULT_MAX_LINES))
            },
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

        Command::Export {
            notebook,
            index,
            file,
            force,
        } => export::run(&notebook, index, &file, force, cli.quiet),

        Command::Duplicate {
            notebook,
            selection,
            at,
        } => duplicate::run(&notebook, &selection, at, cli.backup, cli.quiet),

        Command::Strip {
            notebook,
            selection,
            outputs,
            cell_metadata,
            notebook_metadata,
            dry_run,
        } => strip::run(
            &notebook,
            &selection,
            outputs,
            cell_metadata,
            notebook_metadata,
            dry_run,
            cli.backup,
            cli.quiet,
        ),

        Command::Validate { notebook, json } => validate::run(&notebook, json),
        Command::Repair { notebook } => validate::repair(&notebook, cli.backup, cli.quiet),
        Command::Bookmark { notebook, action } => match action {
            BookmarkAction::Set { name, index } => {
                bookmark::set(&notebook, &name, index, cli.backup, cli.quiet)
            }
            BookmarkAction::List { json } => bookmark::list(&notebook, json),
            BookmarkAction::Remove { name } => {
                bookmark::remove(&notebook, &name, cli.backup, cli.quiet)
            }
        },

        Command::Ids { notebook, json } => ids::run(&notebook, json),

        Command::Merge {
            notebook,
            selection,
        } => merge::run(&notebook, &selection, cli.backup, cli.quiet),
        Command::Split {
            notebook,
            index,
            at_line,
        } => split::run(&notebook, index, at_line, cli.backup, cli.quiet),
        Command::RenameId { notebook, old, new } => {
            rename_id::run(&notebook, &old, &new, cli.backup, cli.quiet)
        }
        Command::Refs { notebook, to, json } => refs::run(&notebook, &to, json),
        Command::Render {
            notebook,
            output,
            force,
            driver_python,
        } => render::run(&notebook, &output, force, driver_python.as_deref()),
        Command::Query {
            notebook,
            pattern,
            scope,
            ignore_case,
            json,
        } => query::run(&notebook, &pattern, &scope, ignore_case, json),

        Command::Edit {
            notebook,
            index,
            source,
            file,
            editor,
            r#type,
            lines,
            insert_after,
            insert_before,
            delete_lines,
        } => edit::run(
            &notebook,
            index,
            source,
            file,
            editor,
            r#type.as_ref().map(|t| t.as_str()),
            lines.as_deref(),
            insert_after,
            insert_before,
            delete_lines.as_deref(),
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

        Command::Clear {
            notebook,
            selection,
            dry_run,
        } => clear::run(&notebook, &selection, dry_run, cli.backup, cli.quiet),

        Command::Diff { a, b, detailed } => diff::run(&a, &b, detailed),

        Command::Copy {
            src,
            selection,
            dst,
            at,
        } => copy::run(&src, &selection, &dst, at, cli.backup, cli.quiet),

        Command::Info { notebook } => info::run(&notebook),

        Command::Run {
            notebook,
            selection,
            timeout,
            kernel,
            interpreter,
            driver_python,
            allow_errors,
            include_prior,
            startup_timeout,
            iopub_timeout,
            no_record_timing,
            overall_timeout,
            cwd,
            env,
            dry_run,
            json,
            session,
            create_session,
        } => run::run(
            &notebook,
            &selection,
            timeout,
            kernel.as_deref(),
            interpreter.as_deref(),
            driver_python.as_deref(),
            allow_errors,
            include_prior,
            startup_timeout,
            iopub_timeout,
            !no_record_timing,
            overall_timeout,
            cwd.as_deref(),
            &env,
            dry_run,
            json,
            cli.backup,
            cli.quiet,
            session.as_deref(),
            create_session,
        ),

        Command::Replace {
            notebook,
            selection,
            pattern,
            replacement,
            r#type,
            ignore_case,
            dry_run,
        } => replace::run(
            &notebook,
            &selection,
            &pattern,
            &replacement,
            r#type.as_ref().map(|t| t.as_str()),
            ignore_case,
            dry_run,
            cli.backup,
            cli.quiet,
        ),

        Command::Search {
            notebook,
            pattern,
            r#type,
            show_source,
            ignore_case,
        } => search::run(
            &notebook,
            &pattern,
            r#type.as_ref().map(|t| t.as_str()),
            show_source,
            ignore_case,
        ),

        Command::Kernels {
            json,
            details,
            check,
            notebook,
            driver_python,
        } => kernels::run(
            json,
            details,
            check,
            notebook.as_deref(),
            driver_python.as_deref(),
        ),

        Command::Session { action } => match action {
            SessionAction::Start {
                name,
                kernel,
                interpreter,
                driver_python,
                notebook,
                cwd,
                env,
                startup_timeout,
                json,
            } => session::start(
                name.as_deref(),
                kernel.as_deref(),
                interpreter.as_deref(),
                driver_python.as_deref(),
                notebook.as_deref(),
                cwd.as_deref(),
                &env,
                startup_timeout,
                json,
            ),
            SessionAction::List { json } => session::list(json),
            SessionAction::Stop { name, force } => session::stop(&name, force),
        },
    }
}
