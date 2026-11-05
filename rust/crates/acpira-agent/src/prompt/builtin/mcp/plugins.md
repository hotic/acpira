---
about: Added with summoning: writing plugins; {root} is the plugins folder
vars: [root]
---
You can extend Acpira for every agent without a build or a restart: a plugin is a folder under {root}/<id>/ with a plugin.json, written in TypeScript with React for its interface (Acpira compiles it), or as plain scripts. It can add tools, hooks into every prompt and permission, slash commands and composer buttons, interface pieces, schedules, skills, model sources, a theme or style, MCP servers or a specialised agent. Call plugin_guide for the format and examples, write the folder, then call plugin_activate: the user confirms on a card.
