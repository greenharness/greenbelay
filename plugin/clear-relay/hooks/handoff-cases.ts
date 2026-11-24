// The handoff line's grammar, as cases both readers must agree on: the relay
// (register.test.ts runs each through the engine) and greenbelay's hook
// (greenbelay-guard's `clear_at_boundary` tests parse the block between the
// two marker lines below as JSON). Keep that block strict JSON.
//
// A case gives `answer`, the whole final message, or `next_repeat`, whose line
// is `safe to /clear; next: `<unit × count><tail>``. `kind` is `next`,
// `parked`, `absent` or `near-miss`; a `next` case names the `prompt` the relay
// submits (a repeat derives it), and a `near-miss` case its `reason` id.

export type HandoffCase = {
  name: string
  answer?: string
  next_repeat?: { unit: string; count: number; tail?: string }
  kind: 'next' | 'parked' | 'absent' | 'near-miss'
  prompt?: string
  reason?: string
}

export const HANDOFF_CASES: HandoffCase[] =
// handoff-cases:begin
[
  {"name": "a backticked prompt arms", "answer": "Done.\n\nsafe to /clear; next: `resume decisions session-boundaries t30-x: build it`\n", "kind": "next", "prompt": "resume decisions session-boundaries t30-x: build it"},
  {"name": "the bare prompt of 836b5b71", "answer": "Closed t16.\n\nsafe to /clear; next: resume decisions session-boundaries t17-inbox: file it", "kind": "near-miss", "reason": "missing-backticks"},
  {"name": "an empty prompt between backticks", "answer": "safe to /clear; next: ``", "kind": "near-miss", "reason": "empty-prompt"},
  {"name": "a blank prompt between backticks", "answer": "safe to /clear; next: `   `", "kind": "near-miss", "reason": "empty-prompt"},
  {"name": "nothing after next:", "answer": "safe to /clear; next:", "kind": "near-miss", "reason": "empty-prompt"},
  {"name": "2,000 ASCII units arm", "next_repeat": {"unit": "a", "count": 2000}, "kind": "next"},
  {"name": "2,001 ASCII units are overlong", "next_repeat": {"unit": "a", "count": 2001}, "kind": "near-miss", "reason": "overlong"},
  {"name": "1,000 two-unit characters arm", "next_repeat": {"unit": "😀", "count": 1000}, "kind": "next"},
  {"name": "1,001 two-unit characters are overlong", "next_repeat": {"unit": "😀", "count": 1001}, "kind": "near-miss", "reason": "overlong"},
  {"name": "a backtick inside the prompt", "answer": "safe to /clear; next: `a`b`", "kind": "near-miss", "reason": "inner-backtick"},
  {"name": "2,001 units holding a backtick", "next_repeat": {"unit": "a", "count": 2000, "tail": "`"}, "kind": "near-miss", "reason": "inner-backtick"},
  {"name": "an opening backtick only", "answer": "safe to /clear; next: `abc", "kind": "near-miss", "reason": "missing-backticks"},
  {"name": "text after the closing backtick", "answer": "safe to /clear; next: `a` then more", "kind": "near-miss", "reason": "missing-backticks"},
  {"name": "no space after next:", "answer": "safe to /clear; next:`x`", "kind": "near-miss", "reason": "not-next-or-parked"},
  {"name": "two spaces after next:", "answer": "safe to /clear; next:  `x`", "kind": "near-miss", "reason": "missing-backticks"},
  {"name": "a park with a due date", "answer": "Done.\n\nsafe to /clear; parked: next due 2026-11-24\n", "kind": "parked"},
  {"name": "a park with nothing due", "answer": "safe to /clear; parked: nothing due", "kind": "parked"},
  {"name": "an empty park", "answer": "safe to /clear; parked:", "kind": "near-miss", "reason": "empty-parked"},
  {"name": "a park with no space", "answer": "safe to /clear; parked:x", "kind": "near-miss", "reason": "not-next-or-parked"},
  {"name": "the marker alone", "answer": "Done.\n\nsafe to /clear", "kind": "near-miss", "reason": "not-next-or-parked"},
  {"name": "the marker run into a word", "answer": "safe to /clearly done", "kind": "near-miss", "reason": "not-next-or-parked"},
  {"name": "the marker mid-text with another last line", "answer": "It is safe to /clear; next: `x` later.\nFirst the review.", "kind": "absent"},
  {"name": "an answer without the marker", "answer": "Done.", "kind": "absent"},
  {"name": "an empty answer", "answer": "", "kind": "absent"},
  {"name": "trailing blank lines", "answer": "safe to /clear; next: `go on`\n\n  \n\t\n", "kind": "next", "prompt": "go on"},
  {"name": "surrounding spaces and tabs", "answer": " \t safe to /clear; next: `go on` \t", "kind": "next", "prompt": "go on"},
  {"name": "CRLF line endings", "answer": "Done.\r\n\r\nsafe to /clear; next: `go on`\r\n", "kind": "next", "prompt": "go on"},
  {"name": "spaces inside the backticks are kept", "answer": "safe to /clear; next: ` go on `", "kind": "next", "prompt": " go on "},
  {"name": "a trailing U+FEFF is not trimmed", "answer": "safe to /clear; next: `go on`﻿", "kind": "near-miss", "reason": "missing-backticks"},
  {"name": "a trailing U+0085 is not trimmed", "answer": "safe to /clear; next: `go on`\u0085", "kind": "near-miss", "reason": "missing-backticks"},
  {"name": "a U+FEFF line after the handoff line is the last line", "answer": "safe to /clear; next: `go on`\n﻿", "kind": "absent"}
]
// handoff-cases:end
