# Phase 4 Agent Execution TODO

This file tracks the implementation-agent and review-agent loop for Phase 4.
The task files and `DEPENDENCIES.md` remain the source of truth for scope and
dependency ordering.

## Guardrails

- Do not run blocked tasks. Start only tasks whose dependencies are complete.
- Run each implementation task in a background worker agent.
- After each implementation worker reports completion, run a separate background
  review agent for that task.
- If the review agent finds valid issues, fix them and run review again until
  there are no valid issues.
- Do not commit automatically. Official task rows stay unchecked until the
  implementation, review loop, verification, and user-authorized commit are done.
- Keep worker write scopes as narrow as possible. Parallel workers may run only
  when their dependencies are satisfied and their likely write scopes are not
  materially overlapping.

## Task Tracker

| Task | Implementation agent | Review agent | Status | Next action |
|---|---|---|---|---|
| 4.1 `TextIndex::new` | Completed: Pasteur (`019e82ac-19b6-7150-8e0a-75e3fdbebec0`) | Clean: Franklin (`019e82c3-2824-7440-b095-97b221deafed`) | Clean; no valid review issues | Dependency satisfied for 4.2. |
| 4.2 `TextIndex::add_chunks` | Completed: Dewey (`019e82c6-3a6c-7421-a281-471f501e214a`) | Clean: Wegener (`019e82cf-bf2c-7df1-b15b-b6fa7a4c5fd0`) | Clean; no valid review issues | Dependencies satisfied for 4.3 and 4.6. |
| 4.3 `TextIndex::search` | Completed: James (`019e82d3-3312-7ee1-8870-575d65caa4e1`) | Clean: Lorentz (`019e82dd-fd19-7602-ab7a-6d1925b3b54d`) | Clean; no valid review issues | Dependency satisfied for 4.5. |
| 4.4 `rrf_fuse` + weights + synonyms | Completed: Hubble (`019e82ac-61f0-7603-ac8d-5a150a0666b8`) | Clean: Banach (`019e82c3-577e-7910-84e4-cebe046422ea`) | Clean; no valid review issues | Wait for dependent tasks 4.3 before starting 4.5. |
| 4.5 `search_hybrid` | Completed: Arendt (`019e82e2-f7d8-7f20-bf32-58959da6f47c`) + local fix | Clean: Descartes (`019e82f8-232c-7390-ad77-20302c2a5450`) | Clean; no valid review issues | Dependencies satisfied for 4.7 and 4.8. |
| 4.6 extend `index_codebase` for Tantivy | Completed: Kierkegaard (`019e82d3-6ef9-7090-88af-0b74c01c42d3`) | Clean: Leibniz (`019e82de-26be-7ca0-bac4-96e8851a9d60`) | Clean; no valid review issues | Dependency satisfied for 4.7 after 4.5. |
| 4.7 MCP real dispatch | Completed: Meitner (`019e82fd-46a7-7ee3-ac15-a6fcca4f114b`) | Clean: Beauvoir (`019e830a-abb0-7711-98d5-821dfa081290`) | Clean; no valid review issues | Dependency satisfied for 4.8. |
| 4.8 tool handlers | Completed: Gibbs (`019e8310-bfcb-7021-b1b5-f05d3d6e6075`) + local remediation | Clean: Noether (`019e832a-6c1c-7131-88dc-215b2c0a972e`) | Clean; no valid review issues | Phase 4 implementation/review loop complete. |

## Planned Waves

1. Wave 1: run 4.1 and 4.4 implementation workers in background.
2. Wave 1 review: run one review agent for 4.1 and one review agent for 4.4.
3. Wave 2: run 4.2 after 4.1 is clean.
4. Wave 2 review: run a review agent for 4.2.
5. Wave 3: run 4.3 and 4.6 after 4.2 is clean if their write scopes remain safe
   to parallelize; otherwise run them sequentially.
6. Wave 3 review: run review agents for 4.3 and 4.6.
7. Wave 4: run 4.5 after 4.3 and 4.4 are clean.
8. Wave 4 review: run a review agent for 4.5.
9. Wave 5: run 4.7 after 4.5 and 4.6 are clean.
10. Wave 5 review: run a review agent for 4.7.
11. Wave 6: run 4.8 after 4.5 and 4.7 are clean.
12. Wave 6 review: run a review agent for 4.8, then run the Phase 4 exit checks.

## Final Verification

- `cargo build` passed during the 4.8 remediation review.
- `cargo test --workspace` passed after 4.8 remediation.
- `cargo fmt --check` passed after 4.8 remediation.
- `cargo clippy --workspace --all-targets -- -D warnings` passed after 4.8 remediation.
- `git diff --check` passed after 4.8 remediation and after the final tracker update.
