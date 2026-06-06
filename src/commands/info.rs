use anyhow::Result;
use crate::notebook::Notebook;

pub fn run(notebook: &str) -> Result<()> {
    let nb = Notebook::from_file(notebook)?;

    let total = nb.len();
    let code_count = nb.cells.iter().filter(|c| c.cell_type == "code").count();
    let md_count = nb.cells.iter().filter(|c| c.cell_type == "markdown").count();
    let raw_count = nb.cells.iter().filter(|c| c.cell_type == "raw").count();

    let kernel = nb.kernel_name().unwrap_or("unknown");
    let lang = nb.language().unwrap_or("unknown");

    println!("Notebook:  {notebook}");
    println!("Kernel:    {kernel} ({lang})");
    println!("Format:    nbformat {}.{}", nb.nbformat, nb.nbformat_minor);
    println!(
        "Cells:     {total}  ({code_count} code, {md_count} markdown, {raw_count} raw)"
    );
    println!();
    println!(" {:<4} {:<10} {:<7} {}", "#", "type", "lines", "outputs");
    println!(" {}", "-".repeat(38));

    for (i, cell) in nb.cells.iter().enumerate() {
        let num = i + 1;
        let lines = cell.source_str().lines().count();
        let outputs = if cell.cell_type == "code" {
            cell.outputs.len().to_string()
        } else {
            "-".to_string()
        };
        println!(" {:<4} {:<10} {:<7} {}", num, cell.cell_type, lines, outputs);
    }

    Ok(())
}
