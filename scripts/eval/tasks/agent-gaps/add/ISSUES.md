# Open issues: the built-in agent

Bug reports against Acpira's built-in agent, the Rust crate in `rust/crates/acpira-agent` (`acpira agent`, described
in `docs/dev/builtin-agent.md`). Its tests drive it over ACP against a scripted model server.

1. Answering a bash permission card with "Always allow" adds a rule for the command's program and subcommand
   (`echo review *` for `echo review`). That rule also lets through a later command that starts the same way but runs
   another one: `echo review && printf unapproved > marker` ran without a card. A command-prefix allow rule must cover
   a single command only. A command line that chains or pipes another command (`;`, `&&`, `||`, `&`, `|`, a line
   break), redirects (`<`, `>`) or substitutes one (`$(…)`, backticks) has to ask again. Rules that allow every
   command (full access) and deny rules are not affected.
2. Writes to the agent's own configuration (the files and folders under its data home, such as `secrets.json` and
   `providers.json`) always ask, even under full access. Through a symlink they do not: with a folder in the workspace
   linking to the data home, a `write` to `alias/secrets.json` under full access changed the file without a card, and
   `permission::Guard::protects` answers false for that path. The guard must see through symlinked folders and files,
   including a link to a protected file that does not exist yet.
3. Some providers reuse tool-call ids across the replies of one turn (`c1` for a `read` in the first reply, `c1` again
   for a `bash` call in the next). When such a turn is cancelled while the second call runs, the next request carries
   the assistant message with the `bash` call but no `tool` message answering it (the earlier `c1` result seems to
   count for it), which strict providers reject. After a cancel, every call of the last reply must have a result in
   the history.
4. A control changed on a resumed session is lost on the next resume. Set approval to `full`, prompt, restart the agent
   and resume the session (`session/resume`), set approval to `ask` and the model to another one, prompt, then restart
   and resume again: the session comes back with `full` and the first model.
5. A session where no model was ever picked runs on the default model, the first configured one. After its first
   prompt, change the model sources so that another model comes first, then restart and resume the session: it now
   reports the new default instead of the model it ran on. A session that has prompted must keep its model.
6. When the model sources change while a session is open (`providers.json` saved by the settings page with a model
   added), the open session keeps offering the old model list: its client never hears of the change. Within a couple
   of seconds of a change, every open session whose options changed should get one `config_option_update` with its
   full `configOptions`. Sessions whose options did not change get none, and nothing is sent again until the next
   change.
7. A model source whose base URL is a full endpoint (`fullUrl`) gets its model list from the URL next to that
   endpoint: `…/v2/chat/completions` gives `…/v2/models`. A full Responses endpoint
   (`https://example.test/v1/responses`, format `openai-responses`) gets no models URL at all
   (`llm::discover::models_url` returns none), so its models cannot be listed.
