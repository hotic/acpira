---
name: deepseek
version: 1
match: ["deepseek/*", "*deepseek*"]
---
## Tool calls
- A tool runs only when you call it through the tool interface. Never write a call out as text in the answer, as JSON
  or as tags.
- Continue from each result. Do not repeat a call that already succeeded.
- Once the task is done, stop calling tools and answer.
