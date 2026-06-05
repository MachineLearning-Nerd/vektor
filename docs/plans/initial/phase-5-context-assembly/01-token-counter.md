# Task 5.1 — `TokenCounter::estimate` (language-specific heuristic + two-pass verify)

**Phase**: 5 — Context Assembly
**Task ID**: 5.1
**PRD reference**: Section 5.3 (TokenCounter) + Section 5.4 (Token Budget Allocation Strategy)
**Roadmap stage**: Stage 2 / `v0.4.0`
**Effort estimate**: S
**Depends on**: 4.5
**Blocks**: 5.5

## Objective

Provide the token-estimation primitive the `ContextAssembler` (5.5) uses to fill a
token budget. Two layers: a fast **language-specific `bytes/N` heuristic** for the
greedy fill pass, and a precise **`tiktoken-rs`** count used in the two-pass
verification once the package is assembled. The heuristic is cheap enough to call
per chunk during greedy allocation; the precise pass runs once on the assembled
package and corrects over/under-fill.

## Inputs (must exist before starting)

- `HybridResult` from task 4.5 (`src/search/hybrid.rs`) — carries `content: String`
  and `language: String`. The assembler feeds those into the counter; the counter
  takes the language as a `&str` (the same lowercase strings the chunker emits:
  `"python"`, `"typescript"`, `"tsx"`, `"javascript"`, `"jsx"`, `"rust"`, `"go"` —
  see `chunker::Language::as_str`).
- `src/error.rs` `VektorError` taxonomy (for any tiktoken init failure — prefer the
  existing variants; do not invent a new one unless a tiktoken load failure has no
  reasonable home).
- `tiktoken-rs = "0.11"` — **already a dependency** (`Cargo.toml`); do not add or
  bump it. Pick a `cl100k_base` encoder constructor (e.g. `tiktoken_rs::cl100k_base()`);
  cache the encoder in a `OnceLock`/`OnceCell` so it is built once, not per call.

## Outputs (must exist after completion)

- A `TokenCounter` type (suggested module `src/context/budget.rs` or
  `src/context/token_counter.rs`) exposing:
  - `TokenCounter::estimate(text: &str, language: &str) -> usize` — the fast
    heuristic. Uses `text.len()` (UTF-8 byte length) divided by the per-language
    ratio, rounded up (`div_ceil`) so a non-empty chunk never estimates to 0.
  - A precise path, e.g. `TokenCounter::count_exact(text: &str) -> usize`, backed by
    the cached `tiktoken-rs` encoder (`encode_with_special_tokens(text).len()` or the
    ordinary-token variant — pick one and document it). This is what the two-pass
    verification calls on the assembled package text.
- The per-language ratio table (PRD §5.3), as a private `fn ratio_for(language) -> f32`:
  - Python / JavaScript / TypeScript (incl. `jsx`/`tsx`): `bytes / 3.5`
  - Rust / Go / Java: `bytes / 4.2`
  - Documentation (Markdown / RST — `"markdown"`, `"md"`, `"rst"`): `bytes / 4.8`
  - Default (unknown / unmatched language): `bytes / 3.8`

## Approach

- Heuristic: `((text.len() as f64) / ratio_for(language) as f64).ceil() as usize`.
  Match the language string case-insensitively against the table; anything
  unrecognized falls to the `3.8` default. These ratios are the v2.3 fix replacing
  the old flat `bytes/3.5` (which had 29–43% error on Rust/Java) — do not collapse
  them back to one constant.
- Precise pass (the verify half of the two-pass strategy, PRD §5.3/§5.4): the
  assembler greedily fills to **90% of the budget** using `estimate`, then calls
  `count_exact` on the concatenated package text. If the exact count is **under**
  budget, the assembler may add more chunks; if **over**, it truncates the last
  chunk to fit. This task owns the two counting fns; the add/truncate loop lives in
  5.5. Document the 90% threshold here so 5.5 wires it consistently.
- Build the tiktoken encoder lazily and reuse it. The PRD budgets the whole
  two-pass step at **<1ms**, so do not rebuild the encoder per chunk or per call.
- No re-reading files: both fns operate on in-memory chunk content already carried
  by `HybridResult`.

## Acceptance criteria

- [x] `estimate` returns language-differentiated counts: the same byte string
      estimates fewer tokens for `"rust"` (÷4.2) than for `"python"` (÷3.5), and docs
      (`"markdown"`, ÷4.8) lower still.
- [x] Unknown / empty language string falls back to the `3.8` default ratio.
- [x] `estimate` never returns 0 for non-empty input (uses `div_ceil`/`ceil`); empty
      input returns 0.
- [x] `count_exact` agrees with a known `tiktoken-rs` (`cl100k_base`) token count for
      a fixed sample string (golden test against the encoder, not against the heuristic).
- [x] The tiktoken encoder is constructed once (cached), verified by it being a
      `OnceLock`/`OnceCell` (or equivalent) and not re-instantiated per call.
- [x] `count_exact` uses the existing `tiktoken-rs` dependency (`Cargo.toml`) — no new dependency added.
- [x] No `unwrap()` outside `#[cfg(test)]`.

## Verification

```bash
cargo build
cargo test context::budget::tests::estimate_is_language_specific
cargo test context::budget::tests::estimate_unknown_language_uses_default
cargo test context::budget::tests::estimate_nonempty_never_zero
cargo test context::budget::tests::count_exact_matches_tiktoken_golden
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
```

(Adjust the module path in the `cargo test` lines to wherever `TokenCounter` lands,
e.g. `context::token_counter::tests::...`.)

## Notes / open questions

- **Heuristic vs. precise division of labor:** the heuristic is the hot path (called
  per candidate chunk during greedy fill); `count_exact` runs once per assembled
  package. Keeping them as two methods on one type makes the 90% / verify wiring in
  5.5 obvious. The heuristic's ~5% error on Python and the per-language ratios are
  exactly why the second pass exists — heuristic for speed, tiktoken for the final
  ±5% budget guarantee (Phase 5 exit criterion: `token_budget=8000` within ±5%).
- **Encoder choice:** `cl100k_base` is the GPT-3.5/4 family encoder and a reasonable
  default for a code-context tool; the budget number agents pass is an abstract token
  count, not a specific model's. Document the chosen encoder in a code comment so the
  ±5% exit-criterion test is interpreted against the right tokenizer. If a different
  encoder is selected, update the golden test and this note.
- **Language strings, not the enum:** `HybridResult.language` is a `String`, so the
  counter takes `&str`. Markdown/RST chunks currently come from the doc-chunking path
  (PRD v2.2 doc-window chunking); their language tag should be `"markdown"`/`"rst"` —
  confirm the exact string the doc path emits when 5.5 wires this, and extend
  `ratio_for` if it differs.
