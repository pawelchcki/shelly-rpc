// Stamps SHELLYCTL_VERSION into the binary at compile time.
//
// On a clean release tag it equals CARGO_PKG_VERSION so installer receipts
// (axoupdater) and crate metadata stay aligned. Otherwise a dev suffix with
// the short SHA (and `-dirty` if the worktree has uncommitted changes) is
// appended, yielding e.g. `0.1.0-dev.abc1234-dirty`.
//
// Outside a git checkout (cargo-dist source tarball, crates.io install) we
// fall back to the plain package version.

use std::process::Command;

fn main() {
    let pkg = std::env::var("CARGO_PKG_VERSION").expect("CARGO_PKG_VERSION set by cargo");
    let version = resolve_version(&pkg);
    println!("cargo:rustc-env=SHELLYCTL_VERSION={version}");

    // Rebuild when HEAD moves or the index changes so the stamp tracks reality.
    // Missing refs are fine — rerun-if-changed on a nonexistent path is a no-op.
    println!("cargo:rerun-if-changed=../.git/HEAD");
    println!("cargo:rerun-if-changed=../.git/index");
    println!("cargo:rerun-if-env-changed=SHELLYCTL_RELEASE_OVERRIDE");
}

fn resolve_version(pkg: &str) -> String {
    if let Ok(forced) = std::env::var("SHELLYCTL_RELEASE_OVERRIDE") {
        if !forced.is_empty() {
            return forced;
        }
    }

    let Some(sha) = git(&["rev-parse", "--short=7", "HEAD"]) else {
        return pkg.to_string();
    };

    if git(&["describe", "--tags", "--exact-match", "HEAD"]).is_some() {
        return pkg.to_string();
    }

    let dirty = match git_raw(&["status", "--porcelain"]) {
        Some(out) if !out.trim().is_empty() => "-dirty",
        _ => "",
    };

    format!("{pkg}-dev.{sha}{dirty}")
}

fn git(args: &[&str]) -> Option<String> {
    git_raw(args)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn git_raw(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout).ok()
}
