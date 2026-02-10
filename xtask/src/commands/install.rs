use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use clap::Args;

#[derive(Args, Debug)]
pub struct InstallArgs {
    /// Destination directory for the installed binary (default: ~/.bin)
    #[arg(long = "bin-dir", default_value = "~/.bin")]
    pub bin_dir: String,

    /// Cargo profile to build (default: dev)
    #[arg(long = "profile", default_value = "dev")]
    pub profile: String,
}

pub fn cmd_install(args: InstallArgs) -> Result<(), String> {
    let bin_dir = expand_tilde(&args.bin_dir)?;
    fs::create_dir_all(&bin_dir).map_err(|e| format!("{}: {e}", bin_dir.display()))?;

    let root = crate::workspace_root();
    let status = build_cli(&root, &args.profile)?;
    if !status.success() {
        return Err(format!(
            "cargo build failed with status {}",
            status.code().unwrap_or(1)
        ));
    }

    let bin_path = built_binary(&root, &args.profile);
    if !bin_path.exists() {
        return Err(format!("built binary not found at {}", bin_path.display()));
    }

    let dest = bin_dir.join("scrat");
    fs::copy(&bin_path, &dest).map_err(|e| format!("{}: {e}", dest.display()))?;
    set_executable(&dest)?;
    println!("installed {}", dest.display());
    Ok(())
}

fn build_cli(root: &Path, profile: &str) -> Result<std::process::ExitStatus, String> {
    let mut cmd = Command::new("cargo");
    cmd.arg("build").arg("-p").arg("scrat").current_dir(root);
    if profile == "release" {
        cmd.arg("--release");
    } else if profile == "dev" {
        // Default dev profile - no extra args needed
    } else {
        cmd.arg("--profile").arg(profile);
    }
    cmd.status()
        .map_err(|e| format!("failed to run cargo: {e}"))
}

fn built_binary(root: &Path, profile: &str) -> PathBuf {
    let dir = match profile {
        "release" => "release",
        "dev" => "debug",
        _ => profile,
    };
    root.join("target").join(dir).join("scrat")
}

fn expand_tilde(path: &str) -> Result<PathBuf, String> {
    if let Some(rest) = path.strip_prefix("~/") {
        let home = std::env::var("HOME").map_err(|_| "HOME is not set".to_string())?;
        return Ok(PathBuf::from(home).join(rest));
    }
    if path == "~" {
        let home = std::env::var("HOME").map_err(|_| "HOME is not set".to_string())?;
        return Ok(PathBuf::from(home));
    }
    Ok(PathBuf::from(path))
}

#[cfg_attr(not(unix), allow(unused_variables))]
fn set_executable(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(path)
            .map_err(|e| format!("{}: {e}", path.display()))?
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(())
}
