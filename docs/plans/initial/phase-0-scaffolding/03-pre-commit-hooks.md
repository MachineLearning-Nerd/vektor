# Task 0.3 — Pre-commit hooks (rustfmt + clippy)

**Phase**: 0 — Scaffolding
**Task ID**: 0.3
**PRD reference**: PRD Section 15 (Implementation Rules — "cargo check must pass before every commit")
**Roadmap stage**: Stage 2 / pre-`v0.1.0`
**Effort estimate**: S (≤1h)
**Depends on**: 0.1
**Blocks**: 1.1

## Objective

Install a local git pre-commit hook that runs `cargo fmt --check` and `cargo clippy` before every commit, blocking commits that would fail CI. This catches "I forgot to format" mistakes locally instead of after a CI round-trip.

Hooks are **opt-in per developer machine** — they don't ship in the repo. What ships is the *config* that tells the hook tool what to run.

## Inputs (must exist before starting)

- `Cargo.toml` and `src/main.rs` from task 0.1

> **Note on alignment with task 0.2**: this task's hooks invoke the same commands
> CI runs (`cargo fmt --check`, `cargo clippy -- -D warnings`). They align *by intent*,
> but task 0.3 does NOT depend on task 0.2 existing first — hooks call `cargo` directly
> and don't need a GitHub Actions workflow to function. If 0.2 ships later and adds new
> checks, update this file's hooks at that time to stay aligned. See `DEPENDENCIES.md`
> for the authoritative DAG.

## Outputs (must exist after completion)

- `.pre-commit-config.yaml` — config for the [pre-commit framework](https://pre-commit.com/)
- Developer instructions in `README.md` for `pre-commit install`
- Hook actually installed on the executing-agent's machine and verified to fire on a test commit

## Approach

1. Choose hook framework: **`pre-commit`** (Python tool, language-agnostic, ubiquitous). Alternatives are `husky` (Node-tied) or raw `.git/hooks/pre-commit` (no sharing). `pre-commit` wins because it's a single config file in the repo.
2. Create `.pre-commit-config.yaml`:
   ```yaml
   repos:
     - repo: https://github.com/doublify/pre-commit-rust
       rev: v1.0
       hooks:
         - id: fmt
           args: ["--all", "--", "--check"]
         - id: clippy
           args: ["--workspace", "--all-targets", "--", "-D", "warnings"]
     - repo: https://github.com/pre-commit/pre-commit-hooks
       rev: v5.0.0
       hooks:
         - id: trailing-whitespace
         - id: end-of-file-fixer
         - id: check-merge-conflict
         - id: check-yaml
         - id: check-toml
   ```
3. Add a section to `README.md` (or a dedicated `CONTRIBUTING.md` stub) explaining:
   ```bash
   pip install pre-commit
   pre-commit install
   ```
4. Test by making a deliberate format violation: add `let     x=1;` to `src/main.rs` and `git commit`. The commit should be blocked.
5. Document in the task Notes any platform-specific issues (e.g., Apple Silicon Python install path).

## Acceptance criteria

- [ ] `.pre-commit-config.yaml` exists and is syntactically valid (`pre-commit validate-config` passes)
- [ ] On the executing-agent's machine, `pre-commit install` succeeds and creates `.git/hooks/pre-commit`
- [ ] A deliberately-mis-formatted change is **blocked** by the hook: `git commit` exits non-zero with a fmt error message
- [ ] A correctly-formatted change passes the hook and commits normally
- [ ] `pre-commit run --all-files` runs all hooks across the existing repo without errors
- [ ] README.md (or CONTRIBUTING.md if it exists) tells developers how to install pre-commit

## Verification

> **Pre-condition**: working tree MUST be clean before running this verification block.
> The block makes a deliberate edit to `src/main.rs`, attempts a commit, then restores
> the file. It does **not** use `git reset --hard` or `git checkout --` against the
> repo broadly. If the pre-condition fails, abort — never bypass it.

```bash
# Pre-condition: clean working tree
[ -n "$(git status --porcelain)" ] && {
    echo "ABORT: working tree must be clean before running this verification."
    echo "       Run 'git status' and commit or stash existing changes first."
    exit 1
}

# Config validation + install
pre-commit validate-config
pre-commit install

# Test: a deliberate fmt violation must be BLOCKED by the hook
echo "fn  bad(){}" >> src/main.rs
git add src/main.rs
git commit -m "task-0.3-test-should-block"  # hook expected to refuse this
HOOK_EXIT=$?

# Restore: single-file restore (scope limited to src/main.rs, safe because
# pre-condition guaranteed there were no other uncommitted edits to that file)
git restore --staged --worktree src/main.rs

# Post-condition: working tree should be clean again
if [ -n "$(git status --porcelain)" ]; then
    echo "WARN: working tree not clean after test. Run 'git status' to inspect."
fi

# Assertion: the commit must have been blocked
if [ $HOOK_EXIT -eq 0 ]; then
    echo "FAIL: hook did not block bad commit"
    exit 1
fi
echo "OK: hook blocked bad commit"

# Confirm hook script is installed
test -x .git/hooks/pre-commit && echo "OK: hook script present" || { echo "FAIL: no pre-commit hook"; exit 1; }

# Optional: run every hook against every tracked file
pre-commit run --all-files
```

The "happy path" case (a well-formatted commit succeeds) is **not** tested here on
purpose. Testing it would require creating and then unwinding a real commit, which
either needs `git reset --hard HEAD~1` (destructive — listed as banned in global
CLAUDE.md) or a temporary branch. Either adds complexity for marginal value: if the
bad-commit test passes and the hook script is installed, the happy path is implied.

## Notes / open questions

- **Hooks are opt-in**: Vektor cannot force every contributor to install pre-commit. CI (task 0.2) is the source of truth. Hooks are convenience.
- **`pre-commit-rust` repo activity**: the `doublify/pre-commit-rust` repo has been unmaintained at times. Pin to `v1.0` (current stable). If it stales further, fall back to raw shell hooks calling `cargo fmt --check` and `cargo clippy` directly.
- **clippy speed**: clippy on the whole workspace takes 10–30 seconds. That's bearable for a pre-commit hook. If it grows past 60 seconds during Phase 5, consider scoping clippy to changed files only via `clippy --workspace --tests --benches --no-deps -- -D warnings` or only running it on `git push` via a separate `pre-push` hook.
- **Don't bypass with `--no-verify`**: this defeats the purpose. If a hook is firing wrongly, fix the config; don't skip.
- **macOS-specific**: pre-commit's `language: rust` install can hit weird paths on Apple Silicon. If it fails, install `pre-commit` via Homebrew (`brew install pre-commit`) instead of pip.

## Commit

```
chore(hooks): 0.3 — add pre-commit config for rustfmt + clippy + basics

Local pre-commit hook config using the pre-commit framework. Blocks
commits that fail cargo fmt --check or cargo clippy -D warnings,
matching the CI checks added in task 0.2. Also runs trailing-whitespace,
end-of-file-fixer, check-yaml, check-toml from pre-commit-hooks.

Developers must run `pre-commit install` once per checkout to enable.
CI remains the source of truth for pass/fail; hooks are local convenience.

Closes docs/plans/initial/phase-0-scaffolding/03-pre-commit-hooks.md
```
