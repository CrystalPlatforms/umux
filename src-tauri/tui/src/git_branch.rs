// git_branch — read-only repo label resolver (v1.0 Phase 14 / #41).
//
// Deep module: a tiny resolve_branch(dir) -> Option<String> surface hiding the
// direct `.git` parsing that answers "which git line is this panel's STARTING
// working directory on?". No `git` binary is ever spawned (plan decision,
// 2026-08-26) — everything reads two small text files. Never mutates anything;
// used only by the sidebar/tab-bar branch labels.
//
// Assumptions encoded by these tests:
//  - Input: a local filesystem path to a directory (the persisted
//    panels[].workingDirectory). May be empty, deleted, relative, or hostile —
//    every failure mode below resolves to None, never an error, because the
//    label must be able to just be ABSENT on non-repo directories.
//  - `.git` present as a DIRECTORY (normal clone): `<dir>/.git/HEAD` holds
//    either `ref: refs/heads/<name>\n` (branch) or a bare object id
//    (detached). The branch name is the tail after `refs/heads/`; any OTHER
//    ref namespace yields None rather than a guessed name.
//  - `.git` present as a FILE (worktree/submodule-style redirect): its whole
//    content is `gitdir: <path>\n`, where <path> may be absolute OR relative
//    to the directory containing the `.git` file; the pointed-to dir's HEAD
//    answers with the same rules.
//  - Detached: a bare id of 40..=64 hex chars is shortened to its FIRST 7
//    characters (git's classic short length); anything shorter is refused.
//  - Output: Some(label) — a branch short name or short sha — or None.
//  - NOT tested here: permission races mid-read (same None path), symlinks
//    (resolved by the OS), frontend mapping (Vitest lives in src/tabBranch.ts).

use std::path::Path;

/// The resolved display label for a directory: a branch name (`main`) or a
/// short detached-HEAD sha (`9a4c1f2`). None = no repository / unparseable —
/// the caller renders nothing.
pub fn resolve_branch(dir: &Path) -> Option<String> {
    let git = dir.join(".git");
    if git.is_dir() {
        return head_label(&git);
    }
    if git.is_file() {
        let content = std::fs::read_to_string(&git).ok()?;
        let target = redirect_target(content, dir)?;
        return head_label(&target);
    }
    None
}

/// The `gitdir:` target of a worktree-style `.git` FILE, resolved to an
/// absolute path (a relative target anchors at the directory holding the
/// `.git` file, i.e. the worktree root).
fn redirect_target(content: String, anchor: &Path) -> Option<std::path::PathBuf> {
    let path = content.strip_prefix("gitdir:")?.trim();
    if path.is_empty() {
        return None;
    }
    let candidate = Path::new(path);
    Some(if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        anchor.join(candidate)
    })
}

/// The label carried by `<git_dir>/HEAD`: the `refs/heads/` tail of a symbolic
/// ref, or the short form of a bare detached id. Any other content is refused.
fn head_label(git_dir: &Path) -> Option<String> {
    let head = std::fs::read_to_string(git_dir.join("HEAD")).ok()?;
    let trimmed = head.trim();
    if let Some(name) = trimmed.strip_prefix("ref: ").and_then(|r| r.strip_prefix("refs/heads/")) {
        return (!name.is_empty()).then(|| name.to_string());
    }
    // Detached HEAD: a bare object id (40 chars SHA-1, 64 SHA-256) shortens
    // to git's classic 7-character form; anything else is refused.
    if trimmed.len() >= 40
        && trimmed.len() <= 64
        && trimmed.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Some(trimmed[..7].to_string());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fixture builder: create `root/dir/.git/HEAD` with `head_content`.
    fn make_repo(root: &Path, dir: &str, head_content: &str) -> std::path::PathBuf {
        let git_dir = root.join(dir).join(".git");
        std::fs::create_dir_all(&git_dir).unwrap();
        let head = git_dir.join("HEAD");
        std::fs::write(&head, head_content).unwrap();
        root.join(dir)
    }

    // T1 (AC — branch name for a normal repo):
    //   Input:  a directory whose `.git/HEAD` says `ref: refs/heads/main\n`
    //   Output: Some("main") — the symbolic ref's branch tail.
    #[test]
    fn normal_repo_returns_branch_name_from_head_ref() {
        let tmp = tempfile::tempdir().unwrap();
        let proj = make_repo(tmp.path(), "proj", "ref: refs/heads/main\n");
        assert_eq!(resolve_branch(&proj), Some("main".to_string()));
    }

    // T2 (AC — worktree: follow the `.git` FILE redirect; ABSOLUTE gitdir):
    //   Input:  `wt/.git` containing `gitdir: /abs/…/.git/worktrees/wt\n`,
    //           whose HEAD says `ref: refs/heads/feature\n`
    //   Output: Some("feature") — the redirect TARGET's branch.
    #[test]
    fn worktree_git_file_redirect_is_followed_absolute() {
        let tmp = tempfile::tempdir().unwrap();
        let wt_root = make_repo(tmp.path(), "main", "ref: refs/heads/main\n");
        let worktrees = wt_root.join(".git").join("worktrees").join("wt");
        std::fs::create_dir_all(&worktrees).unwrap();
        std::fs::write(worktrees.join("HEAD"), "ref: refs/heads/feature\n").unwrap();

        let wt = tmp.path().join("wt");
        std::fs::create_dir(&wt).unwrap();
        let git_file = wt.join(".git");
        std::fs::write(
            &git_file,
            format!("gitdir: {}\n", worktrees.display()),
        )
        .unwrap();

        assert_eq!(resolve_branch(&wt), Some("feature".to_string()));
    }

    // T3 (AC — detached HEAD: a bare object id becomes a SHORT sha):
    //   Input:  `.git/HEAD` holding a 40-char object id
    //   Output: Some("<first 7 chars>") — git's classic short length.
    #[test]
    fn detached_head_returns_short_sha() {
        let tmp = tempfile::tempdir().unwrap();
        let sha = "9a4c1f2e5b7d3a8c6f0e1d2b4a9c8e7f5a3b2c1d";
        let proj = make_repo(tmp.path(), "proj", &format!("{sha}\n"));
        assert_eq!(resolve_branch(&proj), Some("9a4c1f2".to_string()));
    }

    // T4 (AC — no repository at all → clean None, never an error):
    //   Input:  a plain directory without `.git` (and, incidentally, a path
    //           that does not exist and an empty one)
    //   Output: None in all three cases — the row simply shows no label.
    #[test]
    fn non_repo_directories_resolve_to_none() {
        let tmp = tempfile::tempdir().unwrap();
        let plain = tmp.path().join("plain");
        std::fs::create_dir(&plain).unwrap();
        assert_eq!(resolve_branch(&plain), None);
        assert_eq!(resolve_branch(&tmp.path().join("nope")), None);
    }

    // T6 (guard — refuse to guess: content the parser cannot NAME with
    //  confidence resolves to None rather than some invented label):
    //   Input:  `.git/HEAD` holding a non-refs/heads symbolic ref, an EMPTY
    //           branch tail, plain prose, and a shorter-than-40 bare id.
    //   Output: None every time — no label beats a wrong label.
    #[test]
    fn unparseable_head_yields_none() {
        let tmp = tempfile::tempdir().unwrap();
        for content in [
            "ref: refs/stash\n",
            "ref: refs/heads/\n",
            "hello world\n",
            "abc123",
        ] {
            let proj = make_repo(tmp.path(), &format!("p-{}", content.len()), content);
            assert_eq!(resolve_branch(&proj), None, "content: {content:?}");
        }
    }
    //   Input:  `wt/.git` containing `gitdir: ../main/.git/worktrees/wt\n`
    //   Output: Some("feature") — resolved against the worktree's own root.
    #[test]
    fn worktree_git_file_redirect_is_followed_relative() {
        let tmp = tempfile::tempdir().unwrap();
        let main_root = make_repo(tmp.path(), "main", "ref: refs/heads/main\n");
        let worktrees = main_root.join(".git").join("worktrees").join("wt");
        std::fs::create_dir_all(&worktrees).unwrap();
        std::fs::write(worktrees.join("HEAD"), "ref: refs/heads/feature\n").unwrap();

        let wt = tmp.path().join("wt");
        std::fs::create_dir(&wt).unwrap();
        std::fs::write(wt.join(".git"), "gitdir: ../main/.git/worktrees/wt\n").unwrap();

        assert_eq!(resolve_branch(&wt), Some("feature".to_string()));
    }
}
