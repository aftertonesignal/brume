# Brume agent skills

This directory holds procedural and architectural playbooks for AI
coding agents working on Brume. The format is Anthropic's
[Claude Code Skills](https://github.com/anthropics/skills) — a
directory per skill with a `SKILL.md` at the root and optional
`references/` for deeper material.

Claude Code reads skills from `.claude/skills/<name>/SKILL.md`. To
keep one canonical home for skills while staying compatible with
Claude Code's loader, the canonical content lives here under
`.agents/skills/`, and `.claude/skills/` is a directory symlink
back to it. Other agents (Codex, Cursor, etc.) can be pointed at
`.agents/skills/` directly.

## Skill format

Each skill is a directory whose name matches the `name` field in
its frontmatter. The `SKILL.md` opens with YAML frontmatter and a
markdown body:

```markdown
---
name: example-skill
description: >
  One- to three-sentence summary of what this skill covers.
  Triggers on "phrase one", "phrase two", "phrase three" — phrases
  the agent's intent router can match against the user's request to
  decide whether to load this skill.
---

# Example skill

Body content. Markdown. Whatever structure fits the topic — a
state-machine workflow with gates and verification, a checklist, a
design system with rules and examples, a procedural recipe.
```

### Frontmatter fields

- **`name`** — unique skill identifier; must equal the directory name.
  Stable; renaming is a breaking change for any external tool that
  references it by id.
- **`description`** — what the skill is for, and the trigger phrases
  that activate it. Claude Code's router compares these phrases to
  the user's intent at every turn and only loads matching skills,
  so the description is doing real load-bearing work — concrete
  trigger phrases beat abstract summaries. Aim for 50-150 words
  with at least 5-10 trigger phrases.

### Body conventions

- **Be procedural where the topic is procedural.** "Add a parameter"
  is a sequence of steps; document the steps. Use gates ("before
  starting, verify X exists") and on-failure paths.
- **Anchor advice with file paths and line refs.** "See
  `crates/engine-runtime/src/engine.rs:flush_parameter_echoes`" lands;
  "follow the echo pattern" doesn't.
- **Document negative space.** What this skill is NOT for, what
  alternative the agent should reach for instead, what's
  intentionally absent.
- **Include the why, not just the what.** A rule without rationale
  is brittle — agents and humans both work around rules they don't
  understand. Two-line "why" beats a stronger imperative.
- **Keep it tight.** Most skills land at 80-200 lines. Past that,
  consider splitting or pushing detail to `references/`.

### `references/` (optional)

For material that's too long for the body but useful when the agent
needs to go deep. Common patterns:

- `references/architecture.md` — diagrams, longer-form context.
- `references/examples.md` — annotated code samples.
- `references/why.md` — historical rationale for the current shape.

The `SKILL.md` body should link to references explicitly when the
agent might benefit from reading them; agents typically don't crawl
the directory unprompted.

## Authoring a new skill

1. Pick a name. Brume-specific skills get the `brume-` prefix
   (`brume-cm5-deploy`, `brume-add-parameter`). Cross-cutting rules
   that aren't specific to this codebase don't need it
   (`atomic-commits`).
2. Create `.agents/skills/<name>/SKILL.md` with frontmatter +
   body. Use an existing skill as a template.
3. Add the skill to the index in [`AGENTS.md`](../../AGENTS.md) so
   agents that don't auto-load skills can still find it.
4. Open a PR. The maintainer's review will catch trigger-phrase
   coverage gaps and tone drift.

## When to write a skill vs. update existing docs

- If the topic is **per-PR procedural** (how to add X, how to
  review Y), write a skill.
- If the topic is **always-true architectural** (crate map, hard
  rules, scope), it belongs in `AGENTS.md`.
- If the topic is **first-time-only setup** (CM5 bring-up, building
  the project), it belongs in `INSTALL.md` / `CONTRIBUTING.md` and
  the skill can link to it.
- If the topic is **historical / decision rationale** (why we picked
  X over Y), inline doc-comments at the call site are the better home.
