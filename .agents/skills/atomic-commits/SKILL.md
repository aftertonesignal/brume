---
name: atomic-commits
description: >
  Brume's commit-hygiene rule: one logical change per commit, never
  bundle unrelated work, write the message body to explain why
  rather than what. Triggers on "commit", "commits", "commit message",
  "submit a PR", "open a PR", "push", "stage changes", "git commit",
  "git add", "splitting commits", "should i commit this together",
  "atomic commit", "commit hygiene".
---

# Atomic commits

The maintainer reviews five focused commits more easily than one
sprawling one. This is the single most-cited convention in
`CONTRIBUTING.md` and the most-violated by AI-authored PRs. Treat it
as load-bearing.

## The rule

**One logical change per commit.** A logical change is a self-
contained behavioral shift — a feature, a bug fix, a refactor, a
test, a doc update. Each one is independently revertable, reviewable,
and bisectable.

If you find yourself fixing something tangential while implementing
a feature, that fix is a separate commit. If you're refactoring two
unrelated subsystems in service of a third, the refactors are
separate commits. If you ran `cargo fmt` and it made fmt-only
changes to files you didn't otherwise touch, those go in a `chore:
rustfmt` commit, not in your feature commit.

## When to split

Split when any of these are true:

- The diff touches multiple subsystems that don't depend on each
  other for the change to compile or pass tests.
- You can describe the change in one sentence using the word "and"
  joining two clauses that aren't obviously the same thing.
- A reviewer would reasonably accept one half and reject the other.
- The combined diff is over ~300 lines and contains conceptually
  distinct parts.

Three concrete patterns from the codebase:

1. **Feature + tangential fix.** Adding a new modulation source
   surfaces a stale doc comment in an unrelated file. Two commits:
   the feature, then `docs: refresh stale comment in <file>`.
2. **Format drift after refactor.** A logic change reflows enough
   that `cargo fmt` rewrites unrelated lines. Two commits: the
   logic change with `cargo fmt` applied to *its* lines, then
   `chore: rustfmt — drift from <name>` for the rest. (See `3e5adac`
   on `main` for an example of this pattern after the mlua 0.11
   bump.)
3. **Multi-step migration.** Bumping a dep across multiple feature
   flag flips lands as several commits, not one — sandbox hardening
   first, then the dep bump, then the flag flip. (See the four
   commits between `432883e` and `f49a386` for the Lua 5.4 → 5.5
   migration.)

## When to bundle

Bundle when any of these are true:

- The change cannot compile or pass tests with only one half landed.
  (Example: bumping a trait signature and updating every call site.)
- The pieces are pure mechanical consequences of one decision and
  splitting them creates a confusing interim state.
- The bundle is small (under ~50 lines) and the logical components
  are tightly co-located.

## Commit message shape

The message body explains **why**, not **what**. The diff already
shows the what; `git log` is the most-read piece of project history,
and a future reader (often the original author six months later)
needs to know the *reason*.

A good shape:

```
subsystem: short verb-led summary in present tense

One- to three-paragraph body explaining why this change was needed
— what was wrong, what scenario surfaced it, what alternatives were
considered. Reference issue numbers, prior commits, or planning
docs where relevant.

If there are non-obvious tradeoffs or things deliberately deferred,
say so here. The diff doesn't preserve "I considered X but rejected
it because Y" — only the message body does.

Co-Authored-By: <agent identifier> <noreply@example.com>
```

Concrete example from `main`:

```
ui/engine: periodic ParameterSnapshot reconciles state drift

Closes the WARNING from the 2026-04-28 quality-skeptic review:
UI→engine SetParameter (and engine→UI ParameterChanged echo) flow
through bounded crossbeam channels with try_send_or_log!. On
saturation a message is silently dropped...

Cost on the audio thread: one ~3 KB Vec allocation per second
... If profiling ever flags this the buffer can move to a non-audio
thread via a snapshot-request channel.
```

Note what the body does: states the *why* (closes drift), names the
*specific scenario* (channel saturation), states the *cost*, and
documents what was *deferred* and under what condition that
deferral might be revisited.

## Subject-line conventions

- Lowercase first word after the prefix. `ui/persist: surface IO
  errors`, not `UI/Persist: Surface IO Errors`.
- Verb-led, present tense. `add`, `remove`, `fix`, `bump`,
  `refactor`, `extract`, `replace`, `clamp`, `gate`, `flush`. Not
  `added` or `adds`.
- Subsystem prefix matches the most-affected crate or area:
  `engine`, `ui/persist`, `control-model`, `transport`, `brumectl
  install`, `dsp-core`, `deploy/gadget`, `ci`. Multiple subsystems
  → pick the most-affected, mention the rest in the body.
- Cap at ~70 characters for the subject. The body has no width
  constraint but ~72 cols reads well in `git log`.
- No trailing period on the subject.

## What to do when you've already over-committed

If you've staged a sprawling diff and realize it should be multiple
commits, don't push it. Options:

- `git reset HEAD` and re-stage in pieces.
- `git add -p` for hunk-by-hunk staging.
- If you've already committed locally, `git reset --soft HEAD~1`
  unstages the commit's contents back into the working tree;
  re-stage in pieces.

Do **not** force-push to `main`. Do **not** amend a commit you've
already pushed unless explicitly told to. Commits on `main` are
a record; they don't get rewritten. (See `CONTRIBUTING.md` and
the user-facing memory's "Atomic Commits" entry — same rule
phrased two ways.)

## What this skill is not

This skill is about commit *granularity* and *message quality*. The
PR-flow rules (issue-first alignment, CHANGELOG.md, SPDX headers,
fmt + tests pre-push) live in `CONTRIBUTING.md`'s "Submitting
changes" section.
