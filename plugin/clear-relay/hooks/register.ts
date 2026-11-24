import type { Hook, Register, Timer } from 'claude-code'

// The handoff line an answer ends with: safe to /clear; next: `<prompt>`, or
// safe to /clear; parked: <line>. greenbelay's clear-at-boundary hook reads the
// same grammar (`handoff_line`); handoff-cases.ts pins both.
const MARKER = 'safe to /clear'

const COUNTDOWN_MS = 10_000
const CLEAR_WAIT_MS = 5_000
const MIN_GAP_MS = 120_000
// In UTF-16 units, a string's `.length`; the hook counts the same units.
const MAX_PROMPT_UNITS = 2_000
// A near-miss log entry keeps its line, cut to this many UTF-16 units.
const MAX_LOGGED_LINE_UNITS = 2_000
const SHOWN_PROMPT_CHARS = 30
const FAILURE_TOAST_MS = 60_000

type Engine = Parameters<Hook<'turn.complete'>>[0]

type Relay = {
  prompt: string
  sessionId: string
  cwd: string
}

type State =
  | { kind: 'idle' }
  | { kind: 'armed'; relay: Relay; timer: Timer }
  | { kind: 'firing'; relay: Relay }

type DisarmCause =
  | 'keystroke'
  | 'typing'
  | 'prompt-submitted'
  | 'turn-started'
  | 'stay'
  | 'other-ending'
  | 'cleared-by-person'

type RelayFailure =
  | { kind: 'clear-refused'; detail: string }
  | { kind: 'clear-timeout' }
  | { kind: 'submit-dropped'; detail: string }
  | { kind: 'stay-unregistered'; detail: string }
  | { kind: 'threw'; detail: string }

// A handoff prompt that names one programme ticket (ADR-0078):
// `resume <repo> <programme> <ticket>: <gist>`. The ticket is its file's stem,
// as relais prints it: an optional prefix, a letter and digits, an optional
// letter, then slug words (`t26-sessions-claim-their-work`, `t08b`,
// `an-t01-state-inventory`). Three other words before a colon name no ticket.
// holds-until: every ticket stem matches this shape (321 of 321 on
// 2026-11-19); a new id scheme in relais widens it
const NAMED =
  /^resume (?<repo>[^\s:]+) (?<programme>[^\s:]+) (?<ticket>(?:[a-z]+-)?[a-z]\d+[a-z]?(?:-[a-z0-9]+)*):/

/** Why a line that starts with the marker does not arm: the hook's ids. */
type NearMissReason =
  | 'empty-parked'
  | 'not-next-or-parked'
  | 'empty-prompt'
  | 'missing-backticks'
  | 'inner-backtick'
  | 'overlong'

type HandoffLine =
  | { kind: 'next'; prompt: string }
  | { kind: 'parked' }
  | { kind: 'near-miss'; reason: NearMissReason; line: string; units?: number }

/** The ticket a submitted prompt names; `null` for any other prompt. */
type Named = { repo: string; programme: string; ticket: string } | null

/** One line of the relay's log. A failure carries the prompt, so it can be pasted from the log. */
type LogEntry =
  | { event: 'arm' | 'clear' }
  | { event: 'submit'; named: Named }
  | { event: 'disarm'; cause: DisarmCause }
  | { event: 'failure'; failure: RelayFailure; prompt: string }
  | { event: 'near-miss'; reason: NearMissReason; line: string }

/** Where a log line belongs: the session and its directory. */
type Place = { sessionId: string; cwd: string }

/**
 * What one registration of the module holds. `register` creates it and every
 * helper takes it: the hooks compiler wants the functions `$` is passed to at
 * the top of the file, so the state travels as this argument instead.
 */
type Relayer = {
  readonly isEnabled: boolean
  readonly stateDir: string
  state: State
  lastFiredAt: number | undefined
  clearEnded: (() => void) | undefined
  // One write chain per log file: the file is read and rewritten whole, so two
  // overlapping writes would each drop the other's line.
  readonly writes: Map<string, Promise<void>>
}

// Space, tab, CR and LF only, on both sides: JavaScript's trim() also strips
// U+FEFF and Rust's U+0085, so neither reader uses its own.
function trimmed(text: string): string {
  return text.replace(/^[ \t\r\n]+|[ \t\r\n]+$/g, '')
}

/**
 * The answer's handoff line: its last line that is not empty once trimmed.
 * The checks run in order and the first that fails names the near-miss, as
 * greenbelay's `handoff_line` does. Undefined when that line does not start
 * with the marker.
 */
function handoffLine(answer: string): HandoffLine | undefined {
  const last = answer.split('\n').map(trimmed).filter(line => line !== '').pop()
  if (last === undefined || !last.startsWith(MARKER)) return undefined
  const rest = last.slice(MARKER.length)
  const near = (reason: NearMissReason, units?: number): HandoffLine => ({ kind: 'near-miss', reason, line: last, units })

  if (rest.startsWith('; parked:')) {
    const parked = rest.slice('; parked:'.length)
    if (trimmed(parked) === '') return near('empty-parked')
    return parked.startsWith(' ') ? { kind: 'parked' } : near('not-next-or-parked')
  }
  if (rest === '; next:') return near('empty-prompt')
  if (!rest.startsWith('; next: ')) return near('not-next-or-parked')
  const after = rest.slice('; next: '.length)
  if (trimmed(after) === '') return near('empty-prompt')
  if (after.length < 2 || !after.startsWith('`') || !after.endsWith('`')) return near('missing-backticks')
  const prompt = after.slice(1, -1)
  if (trimmed(prompt) === '') return near('empty-prompt')
  if (prompt.includes('`')) return near('inner-backtick')
  if (prompt.length > MAX_PROMPT_UNITS) return near('overlong', prompt.length)

  return { kind: 'next', prompt }
}

/** What is wrong with a near-miss line, in the hook's words. */
function nearMissText(miss: Extract<HandoffLine, { kind: 'near-miss' }>): string {
  switch (miss.reason) {
    case 'empty-parked':
      return 'the parked line after `parked:` is empty'
    case 'not-next-or-parked':
      return `\`${MARKER}\` is followed by neither \`; next: \` nor \`; parked: \``
    case 'empty-prompt':
      return 'the prompt is empty'
    case 'missing-backticks':
      return 'the prompt after `next: ` is not wrapped in backticks'
    case 'inner-backtick':
      return 'the prompt holds a backtick'
    case 'overlong':
      return `the prompt is ${miss.units ?? 0} UTF-16 units, over the 2,000 limit`
  }
}

/**
 * The ticket the prompt names, recorded so a count can tell a relay that named
 * one ticket from an open-ended one. Recorded only: nothing is refused on it.
 */
function namedTicket(prompt: string): Named {
  const groups = NAMED.exec(prompt)?.groups
  return groups?.repo !== undefined && groups.programme !== undefined && groups.ticket !== undefined
    ? { repo: groups.repo, programme: groups.programme, ticket: groups.ticket }
    : null
}

function shown(prompt: string): string {
  return prompt.length > SHOWN_PROMPT_CHARS
    ? `${prompt.slice(0, SHOWN_PROMPT_CHARS - 1)}…`
    : prompt
}

function describe(failure: RelayFailure): string {
  switch (failure.kind) {
    case 'clear-refused':
      return `/clear was refused (${failure.detail})`
    case 'clear-timeout':
      return `the session did not end within ${CLEAR_WAIT_MS / 1000} s of /clear`
    case 'submit-dropped':
      return `the prompt was refused (${failure.detail})`
    case 'stay-unregistered':
      return `/stay was not registered (${failure.detail})`
    case 'threw':
      return `the relay failed (${failure.detail})`
  }
}

// The directory greenbelay keeps its state in when `stateDir` is not set, the
// way its settings::state_root() finds it: the XDG state home, else a `state`
// directory beside a relocated Claude config, else ~/.local/state under HOME or
// USERPROFILE. With none of them there is no place to keep a log: `undefined`.
async function defaultStateDir($: Engine): Promise<string | undefined> {
  const xdg = (await $.env.get('XDG_STATE_HOME')) ?? ''
  if (xdg !== '') return `${xdg}/greenbelay`

  const config = (await $.env.get('CLAUDE_CONFIG_DIR')) ?? ''
  if (config !== '') return `${config}/state/greenbelay`

  const nonEmpty = (value: string | undefined) => (value === '' ? undefined : value)
  const home = nonEmpty(await $.env.get('HOME')) ?? nonEmpty(await $.env.get('USERPROFILE')) ?? ''
  return home !== '' ? `${home}/.local/state/greenbelay` : undefined
}

async function logFile($: Engine, relayer: Relayer, sessionId: string): Promise<string | undefined> {
  const base = relayer.stateDir !== '' ? relayer.stateDir : await defaultStateDir($)

  return base === undefined ? undefined : `${base}/clear-relay/${sessionId}.jsonl`
}

async function log($: Engine, relayer: Relayer, relay: Place, entry: LogEntry): Promise<void> {
  const file = await logFile($, relayer, relay.sessionId)
  if (file === undefined) {
    // The log is a record: with nowhere to keep it, say so and go on.
    await $.ui.log('clear-relay: no state directory (set stateDir, XDG_STATE_HOME or HOME): the log is not kept')
    return
  }
  const line = JSON.stringify({
    ...entry,
    session: relay.sessionId,
    cwd: relay.cwd,
    time: await $.clock.now(),
  })
  const previous = relayer.writes.get(file) ?? Promise.resolve()
  // The log is a record: a failed write is said in the UI log and never stops the relay.
  const written = previous
    .then(async () => {
      const before = (await $.fs.exists(file)) ? await $.fs.read(file) : ''
      await $.fs.write(file, `${before}${line}\n`)
    })
    .catch(error => $.ui.log(`clear-relay: log write to ${file} failed (${String(error)})`))
  relayer.writes.set(file, written)
  await written
}

async function disarm($: Engine, relayer: Relayer, cause: DisarmCause): Promise<void> {
  if (relayer.state.kind !== 'armed') return
  const { relay, timer } = relayer.state
  timer.cancel()
  relayer.state = { kind: 'idle' }
  $.ui.status(undefined)
  await log($, relayer, relay, { event: 'disarm', cause })
}

async function fail($: Engine, relayer: Relayer, relay: Relay, failure: RelayFailure): Promise<void> {
  relayer.state = { kind: 'idle' }
  $.ui.status(undefined)
  const file = await logFile($, relayer, relay.sessionId)
  $.ui.toast(
    `clear-relay: ${describe(failure)}. Paste this to carry on (also in ${file}): ${relay.prompt}`,
    { timeoutMs: FAILURE_TOAST_MS },
  )
  await log($, relayer, relay, { event: 'failure', failure, prompt: relay.prompt })
}

function waitForClear($: Engine, relayer: Relayer): { ended: Promise<boolean>; stop: () => void } {
  let timer: Timer | undefined
  const ended = new Promise<boolean>(resolve => {
    relayer.clearEnded = () => resolve(true)
    timer = $.clock.after(CLEAR_WAIT_MS, () => resolve(false))
  })

  return {
    ended,
    stop: () => {
      timer?.cancel()
      relayer.clearEnded = undefined
    },
  }
}

async function fire($: Engine, relayer: Relayer): Promise<void> {
  if (relayer.state.kind !== 'armed') return
  const { relay } = relayer.state
  relayer.state = { kind: 'firing', relay }

  try {
    const box = await $.prompt.read()
    if (box.text.trim() !== '') {
      relayer.state = { kind: 'idle' }
      $.ui.status(undefined)
      await log($, relayer, relay, { event: 'disarm', cause: 'typing' })
      return
    }
    relayer.lastFiredAt = await $.clock.now()

    const wait = waitForClear($, relayer)
    try {
      await $.command.run({ command: 'clear' })
    } catch (error) {
      wait.stop()
      await fail($, relayer, relay, { kind: 'clear-refused', detail: String(error) })
      return
    }
    const isCleared = await wait.ended
    wait.stop()
    if (!isCleared) {
      await fail($, relayer, relay, { kind: 'clear-timeout' })
      return
    }
    await log($, relayer, relay, { event: 'clear' })
    await declareStay($, relayer, relay)

    const submitted = await $.prompt.submit({ text: relay.prompt })
    if (submitted.drop !== undefined) {
      await fail($, relayer, relay, { kind: 'submit-dropped', detail: String(submitted.drop) })
      return
    }
    relayer.state = { kind: 'idle' }
    $.ui.status(undefined)
    await log($, relayer, relay, { event: 'submit', named: namedTicket(relay.prompt) })
  } catch (error) {
    await fail($, relayer, relay, { kind: 'threw', detail: String(error) })
  }
}

// No session.start follows a /clear, so the command is declared at every arm
// and again after a clear; declaring a name again replaces it.
async function declareStay($: Engine, relayer: Relayer, relay: Relay): Promise<void> {
  await $.command
    .register({
      name: 'stay',
      description: 'Keep this session: cancel the pending clear.',
    })
    .catch(error =>
      log($, relayer, relay, {
        event: 'failure',
        failure: { kind: 'stay-unregistered', detail: String(error) },
        prompt: relay.prompt,
      }),
    )
}

// A near-miss arms nothing: it is logged, then said in a toast, so the person
// knows why no countdown started. A failed toast is the UI's, never the answer's.
async function nearMiss($: Engine, relayer: Relayer, miss: Extract<HandoffLine, { kind: 'near-miss' }>): Promise<void> {
  const place: Place = { sessionId: await $.session.id(), cwd: await $.session.cwd() }
  await log($, relayer, place, {
    event: 'near-miss',
    reason: miss.reason,
    line: miss.line.slice(0, MAX_LOGGED_LINE_UNITS),
  })
  try {
    await $.ui.toast(
      `clear-relay: not armed: ${nearMissText(miss)}. It arms on: ${MARKER}; next: \`<prompt>\`, or ${MARKER}; parked: <line>. Ask the agent to fix the line, or /clear yourself.`,
      { timeoutMs: FAILURE_TOAST_MS },
    )
  } catch (error) {
    await $.ui.log(`clear-relay: the near-miss toast failed (${String(error)})`)
  }
}

async function arm($: Engine, relayer: Relayer, prompt: string): Promise<void> {
  const box = await $.prompt.read()
  if (box.text.trim() !== '') return
  const now = await $.clock.now()
  if (relayer.lastFiredAt !== undefined && now - relayer.lastFiredAt < MIN_GAP_MS) return

  const relay: Relay = {
    prompt,
    sessionId: await $.session.id(),
    cwd: await $.session.cwd(),
  }
  // The way out first, so a cut line still says how to keep the session.
  const text = `clear within 10 s (type or /stay to keep), then: ${shown(prompt)}`
  const timer = $.clock.after(COUNTDOWN_MS, () => {
    void fire($, relayer)
  })
  relayer.state = { kind: 'armed', relay, timer }
  $.ui.toast(text, { timeoutMs: COUNTDOWN_MS })
  $.ui.status(text)
  await log($, relayer, relay, { event: 'arm' })

  await declareStay($, relayer, relay)
}

export const register: Register = (on, options) => {
  const relayer: Relayer = {
    // plugin.json types `enabled` as a boolean, and the engine refuses any other value.
    isEnabled: options.enabled !== false,
    stateDir: typeof options.stateDir === 'string' ? options.stateDir : '',
    state: { kind: 'idle' },
    lastFiredAt: undefined,
    clearEnded: undefined,
    writes: new Map(),
  }

  on('turn.complete', async ($, e, next) => {
    const completion = await next(e)
    if (e.agentId !== undefined || relayer.state.kind === 'firing') return completion

    try {
      const line = e.reason === 'answer' ? handoffLine(e.answer) : undefined
      if (line?.kind !== 'next') {
        await disarm($, relayer, 'other-ending')
        if (line?.kind === 'near-miss' && relayer.isEnabled) await nearMiss($, relayer, line)
      } else if (relayer.isEnabled && relayer.state.kind === 'idle') {
        await arm($, relayer, line.prompt)
      }
    } catch (error) {
      $.ui.log(`clear-relay: ${String(error)}`) // the answer stands whatever the relay does
    }

    return completion
  })

  on('prompt.edit', async ($, e, next) => {
    if (relayer.state.kind === 'armed') await disarm($, relayer, 'keystroke')

    return next(e)
  })

  on('prompt.submit', async ($, e, next) => {
    // The relay submits only while firing, which no disarm reaches; the origin
    // check also covers its own submit arriving after the state moved on.
    const isOwn = e.origin?.kind === 'plugin' && e.origin.name === $.plugin.name
    if (!isOwn && relayer.state.kind === 'armed') await disarm($, relayer, 'prompt-submitted')

    return next(e)
  })

  on('turn.start', async ($, e, next) => {
    if (relayer.state.kind === 'armed') await disarm($, relayer, 'turn-started')

    return next(e)
  })

  on('command.run', { command: 'stay' }, async $ => {
    if (relayer.state.kind === 'armed') {
      await disarm($, relayer, 'stay')

      return { text: 'clear-relay: staying in this session.' }
    }

    return { text: 'clear-relay: nothing to cancel.' }
  })

  on('session.end', async ($, e, next) => {
    if (e.reason === 'clear') {
      if (relayer.state.kind === 'armed') await disarm($, relayer, 'cleared-by-person')
      else relayer.clearEnded?.()
    }

    return next(e)
  })
}
