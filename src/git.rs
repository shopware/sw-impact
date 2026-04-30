use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use anyhow::{Context, Result, anyhow, bail};

pub fn ensure_worktree(path: &Path) -> Result<()> {
    let output = git_output(path, ["rev-parse", "--is-inside-work-tree"])?;
    let value = parse_stdout(output)?;

    if value.trim() == "true" {
        Ok(())
    } else {
        bail!("{} is not inside a Git worktree", path.display())
    }
}

pub fn worktree_root(path: &Path) -> Result<PathBuf> {
    ensure_worktree(path)?;

    let output = git_output(path, ["rev-parse", "--show-toplevel"])?;
    let root = parse_stdout(output)?;
    Ok(PathBuf::from(root.trim()))
}

pub fn resolve_merge_base(path: &Path, base: &str) -> Result<String> {
    let output = git_output(path, ["merge-base", "HEAD", base])
        .with_context(|| format!("failed to resolve merge base against {base}"))?;
    Ok(parse_stdout(output)?.trim().to_owned())
}

pub fn changed_committed_files(path: &Path, merge_base: &str) -> Result<Vec<PathBuf>> {
    git_path_list(
        path,
        [
            "diff",
            "--name-only",
            "-z",
            "--no-renames",
            "--diff-filter=ACMRTD",
            merge_base,
            "HEAD",
        ],
    )
}

pub fn unstaged_tracked_files(path: &Path) -> Result<Vec<PathBuf>> {
    git_path_list(
        path,
        [
            "diff",
            "--name-only",
            "-z",
            "--no-renames",
            "--diff-filter=ACMRTD",
        ],
    )
}

pub fn staged_tracked_files(path: &Path) -> Result<Vec<PathBuf>> {
    git_path_list(
        path,
        [
            "diff",
            "--cached",
            "--name-only",
            "-z",
            "--no-renames",
            "--diff-filter=ACMRTD",
        ],
    )
}

pub fn untracked_files(path: &Path) -> Result<Vec<PathBuf>> {
    git_path_list(path, ["ls-files", "--others", "--exclude-standard", "-z"])
}

pub fn changed_files(path: &Path, merge_base: &str) -> Result<Vec<PathBuf>> {
    let mut files = BTreeSet::new();

    for file in changed_committed_files(path, merge_base)? {
        files.insert(file);
    }

    for file in staged_tracked_files(path)? {
        files.insert(file);
    }

    for file in unstaged_tracked_files(path)? {
        files.insert(file);
    }

    for file in untracked_files(path)? {
        files.insert(file);
    }

    Ok(files.into_iter().collect())
}

pub fn base_file_content(
    path: &Path,
    merge_base: &str,
    relative_path: &Path,
) -> Result<Option<String>> {
    let spec = format!("{merge_base}:{}", git_path(relative_path));
    let output = Command::new("git")
        .arg("-C")
        .arg(path)
        .arg("show")
        .arg(&spec)
        .output()
        .with_context(|| format!("failed to run git show for {}", relative_path.display()))?;

    if output.status.success() {
        return String::from_utf8(output.stdout)
            .map(Some)
            .with_context(|| format!("base content is not UTF-8 for {}", relative_path.display()));
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    if is_missing_revision_path(&stderr) {
        return Ok(None);
    }

    Err(git_error(output, &["show", &spec])).with_context(|| {
        format!(
            "failed to read base content for {}",
            relative_path.display()
        )
    })
}

fn git_output<const N: usize>(path: &Path, args: [&str; N]) -> Result<Output> {
    let output = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(args)
        .output()
        .with_context(|| format!("failed to run git in {}", path.display()))?;

    if output.status.success() {
        Ok(output)
    } else {
        Err(git_error(output, &args))
    }
}

fn git_path_list<const N: usize>(path: &Path, args: [&str; N]) -> Result<Vec<PathBuf>> {
    let output = git_output(path, args)?;
    let stdout = String::from_utf8(output.stdout).context("git path output was not UTF-8")?;

    Ok(stdout
        .split_terminator('\0')
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .collect())
}

fn parse_stdout(output: Output) -> Result<String> {
    String::from_utf8(output.stdout).context("git output was not UTF-8")
}

fn git_error(output: Output, args: &[&str]) -> anyhow::Error {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let detail = if stderr.trim().is_empty() {
        stdout.trim()
    } else {
        stderr.trim()
    };

    anyhow!("git {} failed: {detail}", args.join(" "))
}

fn git_path(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn is_missing_revision_path(stderr: &str) -> bool {
    stderr.contains("exists on disk, but not in")
        || stderr.contains("does not exist in")
        || (stderr.contains("Path '") && stderr.contains("' does not exist"))
        || (stderr.contains("path '") && stderr.contains("' does not exist"))
}
