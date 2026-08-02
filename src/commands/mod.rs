pub mod clear;
pub mod copy;
pub mod create;
pub mod delete;
pub mod diff;
pub mod edit;
pub mod info;
pub mod kernels;
pub mod r#move;
pub mod read;
pub mod replace;
pub mod run;
pub mod search;

use crate::cli::{Cli, Command};
use anyhow::Result;

pub fn dispatch(cli: Cli) -> Result<()> {
    match cli.command {
        Command::Read {
            notebook,
            selection,
            r#type,
            show_outputs,
            json,
            lines,
        } => read::run(
            &notebook,
            &selection,
            r#type.as_ref().map(|t| t.as_str()),
            show_outputs,
            json,
            lines.as_deref(),
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
    }
}
