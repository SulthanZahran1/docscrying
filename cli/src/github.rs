//! GitHub source: `docscrying serve github:owner/repo[@ref]` downloads the
//! repo tarball from codeload and indexes it exactly like a local directory.
//!
//! v1 pragmatics: shells out to `curl` and `tar` (present on every Linux box
//! this tool targets; keeps the build hermetic with zero new crates). v2 (lazy
//! GitHub REST source with per-doc fetch) will replace this behind a Source
//! trait with a proper HTTP client.

use std::path::PathBuf;
use std::process::Command;

pub struct GithubSource {
    /// "owner/repo" — what the reader header shows
    pub display: String,
    /// Resolved commit SHA (full 40 hex)
    pub sha: String,
    /// The ref the user asked for (branch/tag/sha), if any
    pub reference: Option<String>,
    /// Local dir holding the extracted tree
    pub dir: PathBuf,
}

/// Parse `owner/repo[@ref]` (the part after `github:`).
///
/// Refs may contain slashes (`feature/foo`); `@` is split from the right so a
/// ref like `a@b` survives (GitHub branch names with `@` are vanishingly rare).
pub fn parse(spec: &str) -> Result<(String, String, Option<String>), String> {
    let spec = spec.trim();
    if spec.is_empty() {
        return Err("github source: expected github:owner/repo[@ref]".into());
    }
    let (owner, rest) = spec
        .split_once('/')
        .ok_or_else(|| format!("github source: expected github:owner/repo, got github:{spec}"))?;
    if owner.is_empty() || rest.is_empty() {
        return Err(format!(
            "github source: expected github:owner/repo, got github:{spec}"
        ));
    }
    let (repo, reference) = match rest.rsplit_once('@') {
        Some((_r, "")) => return Err(format!("github source: empty ref in github:{spec}")),
        Some((r, rf)) => (r, Some(rf.to_string())),
        None => (rest, None),
    };
    if repo.is_empty() {
        return Err(format!(
            "github source: expected github:owner/repo, got github:{spec}"
        ));
    }
    for (what, s) in [("owner", owner), ("repo", repo)] {
        if !s
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
        {
            return Err(format!("github source: invalid {what} name {s:?}"));
        }
    }
    Ok((owner.to_string(), repo.to_string(), reference))
}

/// Resolve the ref to a commit SHA via the GitHub REST API.
/// No ref given -> the repo's default branch.
fn resolve_sha(
    owner: &str,
    repo: &str,
    reference: Option<&str>,
    token: Option<&str>,
) -> Result<(String, Option<String>), String> {
    let api = |path: &str| -> Result<serde_json::Value, String> {
        let url = format!("https://api.github.com{path}");
        let mut cmd = Command::new("curl");
        cmd.args([
            "-fsSL",
            "--max-time",
            "30",
            "-H",
            "Accept: application/vnd.github+json",
            "-H",
            "User-Agent: docscrying",
            "-H",
            "X-GitHub-Api-Version: 2022-11-28",
        ]);
        if let Some(t) = token {
            cmd.args(["-H", &format!("Authorization: Bearer {t}")]);
        }
        cmd.arg(&url);
        let out = cmd
            .output()
            .map_err(|e| format!("github source: cannot run curl: {e}"))?;
        if !out.status.success() {
            let detail = String::from_utf8_lossy(&out.stderr).trim().to_string();
            return Err(format!(
                "github source: {owner}/{repo}: API request failed ({detail})"
            ));
        }
        serde_json::from_slice(&out.stdout)
            .map_err(|e| format!("github source: bad API response: {e}"))
    };
    match reference {
        Some(rf) => {
            let v = api(&format!("/repos/{owner}/{repo}/commits/{rf}"))?;
            let sha = v["sha"]
                .as_str()
                .ok_or_else(|| "github source: bad commit response".to_string())?
                .to_string();
            Ok((sha, Some(rf.to_string())))
        }
        None => {
            let v = api(&format!("/repos/{owner}/{repo}"))?;
            let db = v["default_branch"]
                .as_str()
                .ok_or_else(|| "github source: bad repo response".to_string())?;
            let v2 = api(&format!("/repos/{owner}/{repo}/commits/{db}"))?;
            let sha = v2["sha"]
                .as_str()
                .ok_or_else(|| "github source: bad commit response".to_string())?
                .to_string();
            Ok((sha, Some(db.to_string())))
        }
    }
}

/// Download and extract the repo at the resolved commit into a fresh temp dir.
/// Always a fresh snapshot per serve (no caching); /tmp is ephemeral by design.
/// `token` (optional) enables private repos.
pub fn fetch(spec: &str, token: Option<&str>) -> Result<GithubSource, String> {
    let (owner, repo, reference) = parse(spec)?;
    let (sha, branch) = resolve_sha(&owner, &repo, reference.as_deref(), token)?;
    let dir = std::env::temp_dir().join(format!("docscrying-github-{owner}-{repo}-{sha}"));
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("github source: cannot create temp dir: {e}"))?;
    let tarball = dir.join("repo.tar.gz");
    let url = format!("https://codeload.github.com/{owner}/{repo}/tar.gz/{sha}");
    let mut cmd = Command::new("curl");
    cmd.args(["-fsSL", "--max-time", "120", "-o"]).arg(&tarball);
    if let Some(t) = token {
        cmd.args(["-H", &format!("Authorization: Bearer {t}")]);
    }
    cmd.arg(&url);
    let out = cmd
        .output()
        .map_err(|e| format!("github source: cannot run curl: {e}"))?;
    if !out.status.success() {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(format!(
            "github source: download failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let out = Command::new("tar")
        .args(["-xzf"])
        .arg(&tarball)
        .args(["-C", &dir.to_string_lossy(), "--strip-components=1"])
        .output()
        .map_err(|e| format!("github source: cannot run tar: {e}"))?;
    let _ = std::fs::remove_file(&tarball);
    if !out.status.success() {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(format!(
            "github source: extract failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let display = format!("{owner}/{repo}");
    Ok(GithubSource {
        display,
        sha,
        reference: branch,
        dir,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_spec() {
        assert_eq!(
            parse("SulthanZahran1/docscrying").unwrap(),
            ("SulthanZahran1".into(), "docscrying".into(), None)
        );
    }

    #[test]
    fn parses_ref() {
        assert_eq!(
            parse("owner/repo@main").unwrap(),
            ("owner".into(), "repo".into(), Some("main".into()))
        );
    }

    #[test]
    fn parses_ref_with_slash() {
        assert_eq!(
            parse("owner/repo@feature/foo").unwrap(),
            ("owner".into(), "repo".into(), Some("feature/foo".into()))
        );
    }

    #[test]
    fn parses_sha_ref() {
        assert_eq!(
            parse("owner/repo@0123456789abcdef0123456789abcdef01234567").unwrap(),
            (
                "owner".into(),
                "repo".into(),
                Some("0123456789abcdef0123456789abcdef01234567".into())
            )
        );
    }

    #[test]
    fn trims_whitespace() {
        assert_eq!(
            parse("  owner/repo  ").unwrap(),
            ("owner".into(), "repo".into(), None)
        );
    }

    #[test]
    fn rejects_missing_slash() {
        assert!(parse("owner").is_err());
        assert!(parse("owner/").is_err());
        assert!(parse("/repo").is_err());
        assert!(parse("").is_err());
    }

    #[test]
    fn rejects_empty_ref() {
        assert!(parse("owner/repo@").is_err());
    }

    #[test]
    fn rejects_invalid_chars() {
        assert!(parse("own er/repo").is_err());
        assert!(parse("owner/repo/extra").is_err());
        assert!(parse("owner/repo!").is_err());
    }
}
