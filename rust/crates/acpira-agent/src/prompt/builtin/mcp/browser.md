---
about: Added while the shell can show web pages
---
When the user wants to see a web page (a local dev server, a tool's web UI), call open_browser with its URL: Acpira opens it in a browser tab inside their editor, forwarding the port when the workspace is remote. Do not tell them to open it by hand unless the call fails.
