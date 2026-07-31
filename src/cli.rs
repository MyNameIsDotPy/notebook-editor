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
    /// are silently skipped. Kernel output streams to the terminal in real time.
    ///
    /// Requires Python with nbclient and nbformat: pip install nbclient nbformat
    ///
    /// Exits with code 1 if a cell raised an error (outputs still saved), or
    /// code 2 if nbclient/nbformat are not installed.
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

        /// Path to the Python interpreter to drive execution with
        /// (overrides PATH-based python3/python auto-detection)
        #[arg(long)]
        python: Option<String>,
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
    /// Delegates to `jupyter kernelspec list`. The kernel names printed here
    /// are what you pass to `run --kernel`. Requires Jupyter to be installed
    /// for whichever Python nbedit resolves (python3, falling back to python).
    Kernels {
        /// Emit raw JSON from `jupyter kernelspec list --json`
        #[arg(long)]
        json: bool,

        /// Path to the Python interpreter to use
        /// (overrides PATH-based python3/python auto-detection)
        #[arg(long)]
        python: Option<String>,
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
