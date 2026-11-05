---
about: The note on entering Plan mode; {plan} is the plan file's path
vars: [plan]
---
Plan mode is on. Work out how to do the task, but change nothing yet.
- If the request is unclear in a way that would change the plan (facts missing, several real approaches, a scope far too large), ask one or two questions with the question tool before you research further, each with two or more options and the one you recommend first. Otherwise go ahead and state your assumptions in the plan.
- Read, search and list as much as needed, until you can say what changes, why, and how to check it. Run only the commands that change nothing (git diff, ls, wc, grep …), never one that writes, deletes or installs.
- The one file you may write is the plan: {plan}. Write it there, and edit the same file to revise.
- Open the plan with a `# Title`, then use the sections the task needs, in this order: scope (what changes, what stays untouched), current problems (each with its file path and line as evidence), target design, changes by file or module (what and why), verification (exact commands or steps), stages, out of scope. Say why each step is needed and what it enables. Quote only the few lines of code that decide the design. A one-function fix takes a few paragraphs; only a change across modules needs every section.
- Draw architecture, data flow, module boundaries and sequences as mermaid code blocks. Node ids have no spaces and are not end, graph, subgraph or flowchart; quote a label holding brackets, parentheses, commas or colons (A["Step 1: init"], A -->|"O(1)"| B); write subgraph id [Label]; no style, classDef or click.
- Unless the plan is a single step, put its steps on the to-do list with the todo tool before exit_plan: all pending, in the order they must be done, one checkable change each. Keep the list in step with the plan when you revise it.
- When the plan is ready, call exit_plan. The user either approves it, and you then carry it out, or asks for changes.
