# Agent instructions

## Open questions

When work has an unresolved question, present it as a dialog of options rather than choosing
silently or writing around it in the summary. If several are open, ask them together in one dialog.

When asked to implement something, ask the questions first, then carry the implementation through to
the end. Once the questions are answered, continue and do not stop to ask again. If an answer turns
out not to apply, or a fresh obstacle appears, note it in the report of the finished work rather
than pausing to ask, unless the obstacle genuinely prevents the work.

When work is pending and nothing is unresolved, offer it as a dialog rather than starting it or
waiting to be told. State the scope options concretely, such as one phase, one item within a phase,
or everything pending, so the choice is about how far to go rather than whether to ask.

When a piece of work is finished, offer to commit it in a dialog before offering the next pending
work. Commit only on an answer that says to, and keep unrelated changes in separate commits when
there are any to keep apart.

## Language

Keep the writing objective and verifiable.

- No editorial framing. Avoid openers like "Good news:", "Worth noting:", "Interesting:", and
  adjectives that praise or disparage work ("excellent", "the subtlest one", "a real find").
- Do not narrate the process. Write the finding, not "Let me look at…" or "I explored this and
  found…".
- State an inference as an inference and an observation as an observation, and name which is which.
  The distinction has repeatedly mattered here, as when an arithmetic model of a layout passed its
  own tests while the rendered screen was broken.
- Give a recommendation as a recommendation, with the reason, rather than asserting a verdict.
- Report a claim that turned out to be wrong as a correction, plainly and early.

## Commands

Never run `sudo`, or any other route to root privileges. Installing or configuring system packages
is a human action. When a tool needed for a task is missing, report what is missing and what it
would be used for, and stop there.

## Verification

- Check numbers against the code or the running program before writing them down. Counts, sizes, and
  line numbers go stale, and a TODO that states a wrong number is worse than one that states none.
- Prefer an executed check over a reasoned one where either is cheap. If a harness is suspected
  rather than the program, say that the harness is the suspect and do not report the result either
  way.
- When formatting or linting is already inconsistent in the repo, compare against the baseline rather
  than requiring zero warnings, and do not reformat code the change did not touch.
