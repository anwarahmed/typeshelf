//! Stamps the build with the git commit it came from, which the update check compares
//! against the latest commit on GitHub.

use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn main() {
    let commit = git(&["rev-parse", "HEAD"]).unwrap_or_default();
    // A build with local changes is not any published version; it must never be
    // "updated" over, so it is marked and the update check leaves it alone.
    let dirty = git(&["status", "--porcelain"]).is_some_and(|s| !s.is_empty());
    println!("cargo:rustc-env=TYPESHELF_COMMIT={commit}{}", if dirty && !commit.is_empty() { "-dirty" } else { "" });
    // Committing changes no source file, so rerun on every build to keep the stamp
    // honest. The crate itself is only recompiled when the stamp actually changes.
    println!("cargo:rerun-if-changed=.force-rerun");
}
