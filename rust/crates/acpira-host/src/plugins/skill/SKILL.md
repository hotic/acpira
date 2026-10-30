---
name: acpira-plugin
description: Write, change or fix an Acpira plugin, which extends Acpira for every agent with tools, prompt and permission hooks, slash commands and composer buttons, React interface pieces, schedules, skills, themes or a specialised agent. Use when the user asks to add something to Acpira itself, to automate something on a schedule, or to customise how Acpira looks or behaves.
---

1. Call the `plugin_guide` tool of Acpira's MCP server. It holds the exact `plugin.json` format, the commands with this machine's paths, and the state of every plugin with its last check output. Without that tool, run `acpira plugin list` and read `docs/dev/plugins.md` of the Acpira repository.
2. Start from the scaffold rather than an empty folder: `acpira plugin new <id> --kind <kinds>` (TypeScript by default). Pick only the kinds the request needs.
3. Edit the files. Keep every name the code uses (hooks, events, methods, tools, schedules, commands) listed in `plugin.json`, since the engine calls only what the manifest declares.
4. Call `plugin_activate` with the id. It freezes and checks the folder; fix what the check reports and call it again. The user enables the version from the card in the conversation, so never edit `state.json` or the frozen copies under `.versions`.
5. After a change, call `plugin_activate` again: the version in effect does not follow the folder.

Constraints worth knowing before writing code:

- There is no build step. Acpira compiles `.ts` / `.tsx`; React and `@acpira/plugin-ui` are provided to interface pieces, which run in a sandboxed frame without network.
- npm packages need `package.json`, a lockfile and `node_modules` installed in the folder; Acpira never installs anything.
- Another plugin's methods or code are reachable only through `dependencies` / `optionalDependencies`.
- Schedules run only while an Acpira engine runs, and act while nobody watches: say so to the user when a schedule sends prompts into sessions.
- Never put an API key in a plugin file; providers name an environment variable (`keyEnv`).
