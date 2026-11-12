---
name: skill-creator
description: Create or update a skill, a folder with a SKILL.md that agents load when a task matches it. Use when the user asks to make, turn something into, or improve a skill.
---

A skill is a folder holding `SKILL.md` and whatever files its work needs:

```text
<name>/
  SKILL.md       frontmatter `name` and `description`, then the instructions
  bin/           optional commands the instructions run by name
  scripts/       optional helpers the instructions run by path
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
- Most skills are instructions alone and need no `bin/`. Add one only when the skill's work runs the same program again and again (a connector, a CLI wrapper): Acpira's own agent puts every skill's `bin/` on the PATH of its commands, so `<command> --flag` works from any folder with no absolute path. A one-off helper stays in `scripts/`, and a command line the agent can write itself needs neither. Make it executable with a shebang (on Windows a `.exe`, `.cmd` or `.ps1`), give it a name no system command has, and have `--help` say how to use it, so `SKILL.md` only names it. Say once that it lives in the skill's `bin/`: other agents run it by that path.

When it is written, read it back once as the agent that will load it, then tell the user its path and that new sessions pick it up.
