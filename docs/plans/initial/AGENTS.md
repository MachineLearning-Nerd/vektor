# AGENTS.md — How to execute this implementation plan

> **Audience**: any agent (Claude Code, Codex CLI, Cursor, a human) that will execute tasks from `docs/plans/initial/`.
> This file is the **contract** for how work happens. Read it once at the start of any session.

---

## What this directory is

`docs/plans/initial/` is the Stage 2 implementation plan for Vektor's first usable release (`v0.1.0` → `v0.4.x`). It's organized into 7 phases (Phase 0 through Phase 6); each phase contains 4–12 tasks; each task is one self-contained markdown file.

This plan is the **operational layer** beneath the strategic docs:

```
VEKTOR_PRD.md         — what the product is and why          (strategic, ~2,700 lines)
VEKTOR_ROADMAP.md     — when stages ship and exit criteria    (strategic, ~500 lines)
docs/plans/initial/   — how Stage 2 gets done, task by task  (operational, this dir)
```

If a task file disagrees with the PRD or roadmap, **the PRD/roadmap wins**; update the task file and flag the drift.

---

## The execution loop

Every agent session follows this loop:

```
1. Read AGENTS.md (this file)
2. Read docs/plans/initial/README.md (phase index + current state)
3. Identify which phase is active (the lowest-numbered phase not marked ✅ Done)
4. Read docs/plans/initial/phase-N/README.md (phase overview)
5. Read docs/plans/initial/DEPENDENCIES.md to know what tasks are blocked
6. Pick an unblocked task in the active phase
7. Create a TODO entry for the chosen task (TaskCreate or local equivalent)
8. Execute the task per its acceptance criteria
9. Verify (run the commands in the Verification section of the task file)
10. Commit (one task = one commit, see Commit Discipline below)
11. Mark task ✅ Done in the phase README
12. If more unblocked tasks exist in this phase, GOTO 6; else if next phase has unblocked tasks, GOTO 4
```

**The loop is intentionally boring.** Each step is independently checkable. Future-you (or a peer agent) can audit any session by walking the loop in reverse.

---

## Task file anatomy

Every task file (`docs/plans/initial/phase-N/NN-task-name.md`) has this structure:

```markdown
# Task N.M — <short imperative title>

**Phase**: N — <phase name>
**Task ID**: N.M
**PRD reference**: Section X / Function Y.Z
**Roadmap stage**: Stage 2 / v0.X.Y
**Effort estimate**: <S | M | L | XL>  (S=≤1h, M=1–4h, L=4–8h, XL=>8h split me)
**Depends on**: <task IDs separated by commas, or "none">
**Blocks**: <task IDs that cannot start until this is done>

## Objective
One paragraph. What this task produces and why it matters.

## Inputs (must exist before starting)
- File or condition 1
- File or condition 2

## Outputs (must exist after completion)
- File or condition 1
- File or condition 2

## Approach
High-level steps. Not pseudocode. The agent fills in the details.

## Acceptance criteria
- [ ] Concrete, verifiable checkbox 1
- [ ] Concrete, verifiable checkbox 2

## Verification
Exact commands to run that prove the acceptance criteria pass.

```bash
cargo check
cargo test path::to::test_name
```

## Notes / open questions
Anything ambiguous the executing agent should flag instead of guessing.
```

If you write a task file that's missing any of these sections, **you're not done** — fill them in before commit.

---

## How to create a TODO list for a phase

When starting Phase N:

1. Open `docs/plans/initial/phase-N/README.md` and read the task list
2. For **each task** in the phase, create a TODO using your local task system:
   - Claude Code: `TaskCreate(subject: "Phase N.M — <title>", description: "Execute task N.M per docs/plans/initial/phase-N/NN-task-name.md")`
   - Codex CLI: equivalent
   - Cursor: append to the project todo list
3. Read `DEPENDENCIES.md` and set up `blockedBy` relationships between TODOs
4. Don't start work until **all phase TODOs exist in your local tracker** — half-tracked work invites missed dependencies

The TODO list is **your local execution memory**. The task files are the **shared specification**. Don't conflate them.

---

## How to dispatch tasks to background agents

Background agents work best when:
1. The task is **completely self-contained** (the task file has everything needed)
2. The task has **no unresolved dependencies** (DEPENDENCIES.md says it's unblocked)
3. The task is **medium-effort** (M or L; XL tasks should be split before dispatching)
4. The task **doesn't touch the same files** as another in-flight background task

Dispatching pattern (Claude Code's `Agent` tool):

```
Agent({
  description: "Execute task 1.3",
  subagent_type: "claude",
  prompt: """
    Execute task 1.3 (Config module) from docs/plans/initial/phase-1-skeleton/03-config-module.md.

    Working directory: /Users/dineshjinjala/Documents/AllCode/Vektor

    Read the task file. Execute the Approach steps. Verify against Acceptance Criteria.
    Commit the result per AGENTS.md Commit Discipline.
    Report back: (a) the commit hash, (b) any deviations from the task spec,
    (c) any blockers you hit that should be flagged in the task file's Notes section.
  """,
  run_in_background: false   // foreground if you need the result to continue; background if parallel
})
```

**Parallel dispatching**: only dispatch multiple background tasks in parallel if their `Blocks` and `Depends on` sets are disjoint AND they touch different file paths. Otherwise you'll hit merge conflicts.

**Don't dispatch XL tasks.** Split them first. An XL task that takes 12 hours in one shot will produce a 600-line commit with mixed concerns. Split into 3 M tasks of 4 hours each, dispatch independently, integrate.

---

## Commit Discipline

One task = one commit. No bundling.

Commit message format (per global CLAUDE.md, no Co-Author):

```
<type>(<scope>): <task ID> — <imperative summary>

<task objective restated in 1-2 sentences>

- Bullet of what concretely changed
- Bullet of what concretely changed
- ...

Closes docs/plans/initial/phase-N/NN-task-name.md
```

`<type>` is conventional commit (`feat` / `fix` / `refactor` / `test` / `docs` / `chore`).
`<scope>` is the Cargo workspace member or top-level module (`chunker`, `embedder`, `mcp`, `ci`, `docs`).

Examples:
```
feat(chunker): 2.3 — implement AST chunker for Python and Rust
test(chunker): 2.5 — add integration tests for chunk dispatcher
ci(release): 6.2 — add GitHub Actions release pipeline for v0.4 binaries
```

**Pre-commit checks** (must pass before commit):
- `cargo fmt --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace` (unless task explicitly defers tests)

If any check fails, **do not bypass with `--no-verify`**. Fix the issue.

---

## Updating task status

When a task completes:
1. Edit `docs/plans/initial/phase-N/README.md` and change the task's status from `⬜` to `✅`
2. Add the commit hash next to the entry: `✅ Done — \`abc1234\``
3. If the task's `Notes / open questions` section gained new content during execution, leave the new content in place — it's institutional memory for the next maintainer

When a phase completes (all tasks ✅ Done):
1. Update `docs/plans/initial/README.md`'s Current State table
2. Update `VEKTOR_ROADMAP.md`'s Current State table
3. Tag the release per Stage 2 sub-release schedule (`v0.1.0`, `v0.2.0`, etc.)

---

## When to deviate from the plan

Sometimes a task file is wrong. Reality intrudes; an assumption was incorrect.

**Allowed deviations:**
- Tightening acceptance criteria (more strict ≠ scope creep)
- Adding a new task that another task discovered was needed
- Splitting an L/XL task into smaller tasks

**NOT allowed without flagging:**
- Skipping acceptance criteria
- Changing the task's `Outputs` (this is a contract; downstream tasks depend on it)
- Bundling two tasks into one commit
- Marking a task ✅ Done while any acceptance criterion is unchecked

If you find yourself wanting to do a "NOT allowed" deviation, **stop, write a 3-line note in the task file's Notes section explaining the situation, and ask the human reviewer**.

---

## When to abandon the plan

If 3+ task files in a row turn out to have wrong assumptions, **stop executing and re-plan**. This is the symptom of a stale plan, not the symptom of an unlucky day. Open a new commit modifying the relevant task files and the phase README before resuming execution.

The plan is a tool, not a master.

---

## Reading order for a new agent

If you've never seen this repo before, read in this order:

1. **`VEKTOR_PRD.md`** Sections 1–3 (the pitch + problem + goals) — ~15 min
2. **`VEKTOR_PRD.md`** Sections 4 + 6 + 8 (architecture + embedder + workflow tools) — ~30 min
3. **`VEKTOR_ROADMAP.md`** in full — ~20 min
4. **This file (AGENTS.md)** — ~10 min
5. **`docs/plans/initial/README.md`** — ~5 min
6. **`docs/plans/initial/DEPENDENCIES.md`** — ~5 min
7. The phase README and task files for whichever phase is active

Total ramp-up: ~90 minutes. Don't start executing tasks before this is done.

---

## What this plan does NOT cover

By design:
- **Stages 3–6** of the roadmap (workflow tools, hardening, trust layer, platform). Those phases are not pre-planned at the task level because requirements will drift over the months between now and then. When Stage 2 completes, write `docs/plans/stage-3-workflow-tools/` with the same structure as this directory.
- **Daily operational concerns**: branch naming, PR review etiquette, etc. Those go in `CONTRIBUTING.md` once it exists (Stage 6 deliverable).
- **Decision logs** for irreversible architectural choices. Use `docs/decisions/NNNN-title.md` (ADRs) for those.

If you find yourself wanting to plan beyond Stage 2 in this directory, **stop**. Future-you needs the freedom to react to what Stage 2 teaches; pre-planning Stages 3–6 today is shelf-ware.

---

*This file is itself versioned. If you change the execution workflow, edit this file and commit. Future agents will read the updated version.*
