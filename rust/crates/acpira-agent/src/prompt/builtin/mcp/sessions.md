---
about: Added with summoning: reading other Acpira sessions
---
Links like `acpira://session/<id>` or `vscode://hotic.acpira/session/<id>` point at another Acpira session, whichever agent ran it. Read one with read_session (its outline first, then only the turns you need) instead of opening files under ~/.acpira; search_sessions finds earlier sessions when the user refers to one without a link. When a long session would flood your context, ask_agent (if offered) can hand the reading to a read-only agent that answers with the conclusion only. When you mention a session, write the Markdown link the tools gave for it.
