use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(
    name = "nbedit",
    about = "Command-line editor for Jupyter notebooks",
    version
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,

    /// Write a .bak copy of the notebook before modifying it
    #[arg(long, global = true)]
    pub backup: bool,

    /// Suppress confirmation messages
    #[arg(short, long, global = true)]
    pub quiet: bool,

    /// Print debug information
    #[arg(short, long, global = true)]
    pub verbose: bool,
}

#[derive(Subcommand)]
pub enum Command {
    /// Print the source of one or more cells
    Read {
        /// Path to the notebook file
        notebook: String,

        /// Cell selection: index, list (1,3,5), range (2-6), or keywords 'all'/'last'
        selection: String,

        /// Filter by cell type
        #[arg(long, value_enum)]
        r#type: Option<CellTypeFilter>,

        /// Also print cell outputs (code cells only)
        #[arg(long)]
        show_outputs: bool,

        /// Emit the full cell JSON instead of plain source
        #[arg(long)]
        json: bool,

        /// Only print specific lines within each cell (same syntax as cell selection)
        #[arg(long)]
        lines: Option<String>,
    },

    /// Create a new cell
    Create {
        /// Path to the notebook file
        notebook: String,

        /// Cell type
        #[arg(long, value_enum, default_value = "code")]
        r#type: CellTypeArg,

        /// Insert before this index (1-based); omit to append
        #[arg(long)]
        at: Option<usize>,

        /// Inline source string
        #[arg(long, conflicts_with = "file")]
        source: Option<String>,

        /// Read source from a file
        #[arg(long, conflicts_with = "source")]
        file: Option<String>,
    },

    /// Replace the source of an existing cell
    ///
    /// Without line flags, replaces the entire cell source. Use --lines for a
    /// block replace (removes the span, inserts replacement in its place),
    /// --insert-after/--insert-before to add lines without removing any, or
    /// --delete-lines to remove specific lines. --source and --file are mutually
    /// exclusive; omit both with --editor to open the cell in $EDITOR.
    Edit {
        /// Path to the notebook file
        notebook: String,

        /// Index of the cell to edit (1-based)
        index: usize,

        /// Replace source with inline text
        #[arg(long, conflicts_with_all = ["file", "editor"])]
        source: Option<String>,

        /// Replace source with file contents
        #[arg(long, conflicts_with_all = ["source", "editor"])]
        file: Option<String>,

        /// Open the cell in $EDITOR
        #[arg(long, conflicts_with_all = ["source", "file"])]
        editor: bool,

        /// Change the cell type
        #[arg(long, value_enum)]
        r#type: Option<CellTypeArg>,

        /// Only replace specific lines within the cell (same syntax as cell selection)
        #[arg(long, conflicts_with_all = ["insert_after", "insert_before", "delete_lines"])]
        lines: Option<String>,

        /// Insert new content after this line number (1-based)
        #[arg(long, conflicts_with_all = ["lines", "insert_before", "delete_lines", "editor"])]
        insert_after: Option<usize>,

        /// Insert new content before this line number (1-based)
        #[arg(long, conflicts_with_all = ["lines", "insert_after", "delete_lines", "editor"])]
        insert_before: Option<usize>,

        /// Delete specific lines within the cell (same syntax as cell selection)
        #[arg(long, conflicts_with_all = ["lines", "insert_after", "insert_before", "source", "file", "editor"])]
        delete_lines: Option<String>,
    },

    /// Remove one or more cells
    Delete {
        /// Path to the notebook file
        notebook: String,

        /// Cell selection: index, list (1,3,5), range (2-6), or keywords 'all'/'last'
        selection: String,

        /// Print which cells would be deleted without modifying the file
        #[arg(long)]
        dry_run: bool,
    },

    /// Reorder cells by moving a selection to a new position
    Move {
        /// Path to the notebook file
        notebook: String,

        /// Cell selection to move
        selection: String,

        /// Destination index (1-based) or 'last'
        #[arg(long)]
        to: String,
    },

    /// Print notebook metadata and cell summary
    Info {
        /// Path to the notebook file
        notebook: String,
    },

    /// Search for a pattern (regex) across cell sources
    ///
    /// Prints matching cells with the line number and content of each match,
    /// prefixed with '>'. Exits with code 1 if no matches are found.
    Search {
        /// Path to the notebook file
        notebook: String,

        /// Pattern to search for (regular expression)
        pattern: String,

        /// Filter by cell type
        #[arg(long, value_enum)]
        r#type: Option<CellTypeFilter>,

        /// Print the full source of each matching cell (non-matching lines indented)
        #[arg(long)]
        show_source: bool,

        /// Case-insensitive matching
        #[arg(short = 'i', long)]
        ignore_case: bool,
    },

    /// Clear outputs and reset execution counts on selected cells
    ///
    /// Only code cells are affected; markdown and raw cells are skipped.
    /// Useful before committing notebooks to avoid storing large output blobs
    /// in version control.
    Clear {
        /// Path to the notebook file
        notebook: String,

        /// Cell selection: index, list (1,3,5), range (2-6), or keywords 'all'/'last'
        selection: String,

        /// Print what would be cleared without modifying the file
        #[arg(long)]
        dry_run: bool,
    },

    /// Show a cell-level diff between two notebooks (ignores outputs and metadata)
    ///
    /// Prints added (+), removed (-), and changed (~) cells. --detailed also
    /// shows a line-level diff of the source within changed cells. Exits with
    /// code 1 if any differences are found.
    Diff {
        /// First notebook file
        a: String,

        /// Second notebook file
        b: String,

        /// Also diff cell source line by line for changed cells
        #[arg(long)]
        detailed: bool,
    },

    /// Copy cells from one notebook into another
    Copy {
        /// Source notebook file
        src: String,

        /// Cell selection from the source notebook
        selection: String,

        /// Destination notebook file
        dst: String,

        /// Insert before this index in the destination (1-based); omit to append
        #[arg(long)]
        at: Option<usize>,
    },

    /// Execute cells via a Jupyter kernel and write outputs back to the notebook
    ///
    /// Runs the kernel from the notebook's directory so relative paths in cell
    /// code (e.g. open("data.csv")) resolve correctly. Markdown and raw cells
    /// are silently skipped. Cell streams are captured into notebook outputs.
    ///
    /// Requires Python with nbclient and nbformat: pip install nbclient nbformat
    ///
    /// Exits with code 1 for execution failure, code 2 for missing driver
    /// dependencies, code 124 for overall timeout, or code 130 for Ctrl-C.
    Run {
        /// Path to the notebook file
        notebook: String,

        /// Cell selection: index, list (1,3,5), range (2-6), or keywords 'all'/'last'
        selection: String,

        /// Per-cell execution timeout in seconds (-1 for no limit)
        #[arg(long, default_value = "-1")]
        timeout: i64,

        /// Kernel name override (default: from notebook metadata)
        #[arg(long)]
        kernel: Option<String>,

        /// Python interpreter that runs notebook cells (need not be registered)
        #[arg(long)]
        interpreter: Option<String>,

        /// Python interpreter containing nbclient (legacy alias: --python)
        #[arg(long = "driver-python", alias = "python")]
        driver_python: Option<String>,

        /// Continue executing later cells after a cell raises an error
        #[arg(long)]
        allow_errors: bool,

        /// Execute preceding code cells as context, but update only selected cells
        #[arg(long, conflicts_with = "session")]
        include_prior: bool,

        /// Kernel startup timeout in seconds
        #[arg(long, default_value = "60")]
        startup_timeout: u64,

        /// IOPub-channel timeout in seconds
        #[arg(long, default_value = "4")]
        iopub_timeout: u64,

        /// Disable nbclient execution timing metadata
        #[arg(long)]
        no_record_timing: bool,

        /// Maximum wall-clock time for the entire execution
        #[arg(long)]
        overall_timeout: Option<u64>,

        /// Working directory for the kernel (default: notebook directory)
        #[arg(long)]
        cwd: Option<String>,

        /// Environment variable passed to the kernel, in KEY=VALUE form
        #[arg(long = "env", value_name = "KEY=VALUE")]
        env: Vec<String>,

        /// Resolve and validate the kernel without executing cells
        #[arg(long)]
        dry_run: bool,

        /// Emit a machine-readable execution report
        #[arg(long)]
        json: bool,

        /// Execute against a persistent kernel session instead of a one-shot kernel
        #[arg(long)]
        session: Option<String>,

        /// Create the session if it doesn't exist yet (requires --session)
        #[arg(long, requires = "session")]
        create_session: bool,
    },

    /// Find and replace text (regex) within cell sources
    ///
    /// Replacement supports capture groups ($1, $2, …). Use --dry-run to
    /// preview changes before writing. Exits with code 1 if no matches are
    /// found.
    Replace {
        /// Path to the notebook file
        notebook: String,

        /// Cell selection: index, list (1,3,5), range (2-6), or keywords 'all'/'last'
        selection: String,

        /// Regex pattern to search for
        pattern: String,

        /// Replacement string (supports capture groups: $1, $2, …)
        replacement: String,

        /// Filter by cell type
        #[arg(long, value_enum)]
        r#type: Option<CellTypeFilter>,

        /// Case-insensitive matching
        #[arg(short = 'i', long)]
        ignore_case: bool,

        /// Show what would change without modifying the file
        #[arg(long)]
        dry_run: bool,
    },

    /// List Jupyter kernels installed on this machine
    ///
    /// Discovers standard kernelspecs, workspace and active Python environments,
    /// and environments exposed by common Python environment managers.
    Kernels {
        /// Emit normalized discovery results as JSON
        #[arg(long)]
        json: bool,

        /// Show interpreter, source, and kernelspec details
        #[arg(long)]
        details: bool,

        /// Probe Python candidates for ipykernel availability
        #[arg(long)]
        check: bool,

        /// Rank candidates for this notebook
        #[arg(long)]
        notebook: Option<String>,

        /// Python interpreter used to locate prefix-scoped kernelspecs
        #[arg(long = "driver-python", alias = "python")]
        driver_python: Option<String>,
    },

    /// Manage persistent kernel sessions used by `run --session`
    ///
    /// A session keeps a kernel process running across separate `nbedit run`
    /// invocations, so variables and imports persist between them. Sessions run
    /// until explicitly stopped.
    Session {
        #[command(subcommand)]
        action: SessionAction,
    },
}

#[derive(Subcommand)]
pub enum SessionAction {
    /// Start a persistent kernel and register it as a session
    Start {
        /// Session name used to reference it later; a random id if omitted
        #[arg(long)]
        name: Option<String>,

        /// Kernel name override (default: automatic)
        #[arg(long)]
        kernel: Option<String>,

        /// Python interpreter that runs notebook cells (need not be registered)
        #[arg(long)]
        interpreter: Option<String>,

        /// Python interpreter containing nbclient (legacy alias: --python)
        #[arg(long = "driver-python", alias = "python")]
        driver_python: Option<String>,

        /// Notebook used only to rank kernel candidates when resolving automatically
        #[arg(long)]
        notebook: Option<String>,

        /// Working directory for the kernel (default: current directory)
        #[arg(long)]
        cwd: Option<String>,

        /// Environment variable passed to the kernel, in KEY=VALUE form
        #[arg(long = "env", value_name = "KEY=VALUE")]
        env: Vec<String>,

        /// Kernel startup timeout in seconds
        #[arg(long, default_value = "60")]
        startup_timeout: u64,

        /// Emit a machine-readable result
        #[arg(long)]
        json: bool,
    },

    /// List known sessions and whether their kernel is still alive
    List {
        /// Emit normalized session records as JSON
        #[arg(long)]
        json: bool,
    },

    /// Stop a session and shut down its kernel
    Stop {
        /// Session name or id
        name: String,

        /// Skip the graceful shutdown and kill the kernel process directly by PID
        #[arg(long)]
        force: bool,
    },
}

#[derive(Clone, ValueEnum)]
pub enum CellTypeFilter {
    Code,
    Markdown,
    Raw,
}

#[derive(Clone, ValueEnum)]
pub enum CellTypeArg {
    Code,
    Markdown,
    Raw,
}

impl CellTypeArg {
    pub fn as_str(&self) -> &'static str {
        match self {
            CellTypeArg::Code => "code",
            CellTypeArg::Markdown => "markdown",
            CellTypeArg::Raw => "raw",
        }
    }
}

impl CellTypeFilter {
    pub fn as_str(&self) -> &'static str {
        match self {
            CellTypeFilter::Code => "code",
            CellTypeFilter::Markdown => "markdown",
            CellTypeFilter::Raw => "raw",
        }
    }
}
