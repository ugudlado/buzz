---
name: workflow-step
description: Execute one Buzz workflow step assigned by TeamLead, persist its machine-readable completion as YAML, and reply in plain teammate language. Use when TeamLead hands off a named workflow step for a ticket.
---

# Complete a workflow step

Keep workflow protocol out of Buzz chat. The worktree holds artifacts and the
completion record; people see only the outcome.

## Execute

1. Require the ticket ID and named step. Ask TeamLead for a missing value
   instead of guessing; do not ask for CLI or state metadata.
2. Treat the current working directory as the assigned shared checkout. Do not
   ask TeamLead for a path. Follow your agent persona's role instructions.
3. Write role artifacts under `spec/changes/<ticket-id-lower>/`. Do not paste artifact
   contents into chat as a substitute for writing them.
4. Before replying, create `spec/changes/<ticket-id-lower>/completions/` in the
   current worktree and write `<step-id>.yaml`. Overwrite that file on a retry.
5. Put the completion mapping at the YAML document root;
   omit the literal `COMPLETION:` wrapper. Include `step_id`, `status`, and any
   requested `artifacts` and `outputs`. `status` is only `completed` or `failed`.
6. Reply in the same thread with the outcome first, the key decision or blocker,
   and where useful details live. Use 2–5 plain sentences and end with
   `@TeamLead`.

## Chat boundary

Never paste YAML, JSON, completion blocks, state paths, step/phase/attempt
metadata, model or token usage, cost, raw CLI output, or file dumps into chat.
Machine completion always means writing the completion file and sending the
human summary described above.

If blocked, still write a completion file with `status: failed` and an
`outputs.reason`, then explain the blocker and the concrete unblocker plainly.
