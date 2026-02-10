use std::fs;
use std::path::PathBuf;

use clap::Args;
use clap_complete::{Shell, generate_to};

#[derive(Args, Debug)]
pub struct CompletionsArgs {
    /// Output directory (default: dist/share/completions)
    #[arg(long = "out-dir", default_value = "dist/share/completions")]
    pub out_dir: PathBuf,

    /// Generate only for specific shell (default: all)
    #[arg(long, value_enum)]
    pub shell: Option<Shell>,
}

pub fn cmd_completions(args: CompletionsArgs) -> Result<(), String> {
    let out_dir = crate::workspace_root().join(args.out_dir);
    fs::create_dir_all(&out_dir).map_err(|e| format!("{}: {e}", out_dir.display()))?;

    let mut cmd = scrat::command();
    let bin_name = "scrat";

    let shells: Vec<Shell> = match args.shell {
        Some(shell) => vec![shell],
        None => vec![Shell::Bash, Shell::Zsh, Shell::Fish, Shell::PowerShell],
    };

    for shell in shells {
        let path = generate_to(shell, &mut cmd, bin_name, &out_dir)
            .map_err(|e| format!("generate {shell} completions: {e}"))?;
        println!("wrote {}", path.display());
    }

    Ok(())
}
