# Agent instructions

## Open questions

When work has an unresolved question, present it as a dialog of options rather than choosing
silently or writing around it in the summary. If several are open, ask them together in one dialog.

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

## Completion cue

When work ships something — a commit, or a verified implementation — play the Terminator 2 theme,
audio only:

```sh
mpv --no-video https://www.youtube.com/watch?v=pVZ2NShfCE8
```

- Run it in the background rather than blocking on playback.
- If `mpv` is not installed, print the link instead of playing it, and say that it was not played.
  Do not install anything to satisfy this, per the rule under Commands.
- Do not fire for replies that only answer a question, explain a state, audit something, or open a
  dialog. Playing after every response makes it noise rather than a cue.

## Verification

- Check numbers against the code or the running program before writing them down. Counts, sizes, and
  line numbers go stale, and a TODO that states a wrong number is worse than one that states none.
- Prefer an executed check over a reasoned one where either is cheap. If a harness is suspected
  rather than the program, say that the harness is the suspect and do not report the result either
  way.
- When formatting or linting is already inconsistent in the repo, compare against the baseline rather
  than requiring zero warnings, and do not reformat code the change did not touch.
