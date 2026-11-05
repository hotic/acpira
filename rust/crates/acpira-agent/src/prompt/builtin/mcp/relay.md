---
about: Added to the MCP instructions when agents can be summoned
---
When the user writes @name for an agent listed by the ask_agent tool, that is an Acpira agent, not one of your own subagents: call ask_agent for it right away, without listing agents first. Several agents named = one ask_agent call each, all in the same message; they run in parallel. A group named = one call with the group as `agent`, which summons every member.
