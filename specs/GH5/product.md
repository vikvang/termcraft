# Product specification: warp sup (#5)

## Summary

Issue #5 does not identify a symptom, requested capability, reproduction path, or affected part of TermCraft. Its title is `warp sup`, its body is `warp `, and its only reporter comment is a greeting to the factory agent. None of those terms maps to an existing TermCraft command or game concept.

**Recommendation:** treat this as a clarification-only specification. Do not change product behavior from the available evidence. A maintainer should replace this recommendation with a concrete requirement before implementation begins.

## Problem

There is not enough information to determine what user problem should be solved. Interpreting `warp sup` as a command, a requested game feature, a bug report, or a reference to the development environment would invent reporter intent and could produce unrelated behavior.

## Current behavior

- TermCraft is a Rust terminal sandbox game whose executable is `termcraft`.
- Running `termcraft` starts the saved or newly generated first-person 3D world. `termcraft --2d` starts the classic side-view mode.
- `src/main.rs` accepts the documented options `--2d`, `--creative`, `--new`, `--seed`, `--name`, `--solo`, `--open`, `--join`, `--help`, and `--version`, with the short forms `-h` and `-V`.
- An unrecognized argument is rejected as an unknown option with an instruction to try `--help`; the process exits with status 2.
- The product source under `src/` contains no `warp` or `sup` command, option, mode, or user-facing concept.

## Desired behavior

No new user-visible behavior is specified by issue #5.

Until the open questions below are answered and this specification is revised:

- Preserve the current executable name, CLI parsing, game modes, controls, persistence, and multiplayer behavior.
- Do not add a `warp` or `sup` command, option, alias, screen, message, or game mechanic.
- Do not infer a defect or acceptance test from the issue title, body, or factory greeting.

## User-visible details

There are no proposed user-visible changes in this revision. Existing invocations and gameplay should behave exactly as documented in `README.md` and implemented in `src/main.rs`.

## Acceptance criteria

1. Issue #5 results in no product-code, dependency, save-format, network-protocol, CLI, documentation, packaging, or release change based on the currently available report.
2. Existing `termcraft` behavior remains unchanged.
3. Before any implementation is scoped from issue #5, the specification is revised to identify:
   - the user problem or requested outcome;
   - the affected surface;
   - observable expected behavior;
   - at least one acceptance scenario.
4. Any revised specification distinguishes explicit maintainer decisions from assumptions made because the original report is incomplete.

## Out of scope

- Guessing that the issue requests integration with Warp or the Warp CLI.
- Adding a `warp sup` invocation or alias.
- Selecting an arbitrary TermCraft bug or feature to implement.
- Changing issue labels or workflow state.
- Modifying product code as part of this specification PR.

## Open questions

1. What problem or requested outcome does `warp sup` refer to?
2. Is `warp sup` intended as a literal command, shorthand for a TermCraft feature, a bug symptom, or only a greeting/test of the factory workflow?
3. If it is a TermCraft change, which surface is affected: startup CLI, 3D mode, 2D mode, multiplayer, saving, rendering, controls, installation, or another area?
4. What exact steps and observable result should define success?

These questions are blocking for product implementation but not for review of this clarification-only draft.
