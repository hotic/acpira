---
name: skill-creator
description: Create or update a skill, a folder with a SKILL.md that agents load when a task matches it. Use when the user asks to make, turn something into, or improve a skill.
---

A skill is a folder holding `SKILL.md` and whatever files its work needs:

```text
<name>/
  SKILL.md       frontmatter `name` and `description`, then the instructions
  scripts/       optional helpers the instructions run
  references/    optional detail read only when a step needs it
```

Where it goes, unless the user names a place:

- `~/.agents/skills/<name>/` for the user's own skills, offered in every project.
- `<project>/.agents/skills/<name>/` for one project's skills.

Acpira's own agent lists both in every new session.

Writing it:

- `name` is the folder name, lowercase words joined by `-`.
- The `description` is all an agent sees before loading the skill: say what it does and when it applies, in a sentence or two.
- The body holds what changes an agent's decisions: the steps, the real constraints, paths and commands. Leave out what a capable agent already knows.
- Keep `SKILL.md` short and move long, mode-specific detail into `references/`, linked from the step that needs it.

When it is written, read it back once as the agent that will load it, then tell the user its path and that new sessions pick it up.
