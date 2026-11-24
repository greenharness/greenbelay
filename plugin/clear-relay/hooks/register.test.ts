import { expect, mock, test } from 'claude-code/testing'
import type { Engine, On } from 'claude-code/testing'
import { HANDOFF_CASES } from './handoff-cases'
import type { HandoffCase } from './handoff-cases'

const PROMPT = 'continue with Phase 2 of docs/plans/x.md'
const LINE = `safe to /clear; next: \`${PROMPT}\``
const STATE_DIR = '/state'
const OPTIONS = { options: { stateDir: STATE_DIR } }
const COUNTDOWN_MS = 10_000
const TWO_MINUTES_MS = 120_000

type World = {
  clock: ReturnType<typeof mock.clock>
  files: Map<string, string>
  env: Map<string, string>
  commands: string[]
  submitted: string[]
  toasts: string[]
  // Each toast event as the engine received it, for its options.
  toastEvents: string[]
  session: { id: string; draft: string }
}

type Refusals = { clear?: true; submit?: true }

// What the engine does beneath the plugin: a clock the test moves, a file
// system in memory, a /clear that ends the session and starts another.
function stage($test: Engine, on: On, refusals: Refusals = {}): World {
  const sessions = ['s1', 's2', 's3', 's4', 's5']
  const world: World = {
    clock: mock.clock(on, { now: 1_000_000 }),
    files: new Map(),
    env: new Map([['HOME', '/home/person']]),
    commands: [],
    submitted: [],
    toasts: [],
    toastEvents: [],
    session: { id: sessions[0] ?? 's1', draft: '' },
  }
  on('turn.complete', (_, e) => ({ text: e.answer }))
  on('turn.start', (_, e) => ({ turnId: e.turnId }))
  on('prompt.edit', (_, e) => ({ text: e.text, cursor: e.cursor }))
  on('session.end', (_, e) => ({ sessionId: e.sessionId }))
  on('session.id', () => ({ value: world.session.id }))
  on('session.cwd', () => ({ value: '/work/repo' }))
  on('env.get', (_, e) => ({ value: world.env.get(e.name) }))
  on('prompt.read', () => ({ value: { text: world.session.draft, cursor: 0 } }))
  on('command.register', () => ({ value: { command: 'stay' } }))
  on('ui.toast', (_, e) => {
    world.toasts.push(e.text)
    world.toastEvents.push(JSON.stringify(e))
    return { value: undefined }
  })
  on('ui.status', () => ({ value: undefined }))
  on('fs.exists', (_, e) => ({ value: world.files.has(e.path) }))
  on('fs.read', (_, e) => ({ value: world.files.get(e.path) ?? '' }))
  on('fs.write', (_, e) => {
    world.files.set(e.path, e.text)
    return { value: undefined }
  })
  on('command.run', { command: 'clear' }, async (_, e) => {
    world.commands.push(e.command)
    if (refusals.clear) throw new Error('clear refused')
    const ended = world.session.id
    world.session.id = sessions[sessions.indexOf(ended) + 1] ?? 's9'
    await $test.session.end({
      reason: 'clear',
      sessionId: ended,
      resume: { sessionId: ended },
    } as never)
    return { text: '' }
  })
  on('prompt.submit', (_, e) => {
    if (refusals.submit) return { drop: 'refused' }
    world.submitted.push(e.text)
    return { text: e.text }
  })
  return world
}

function answer($: Engine, text: string, extra: object = {}) {
  return $.turn.complete({
    answer: text,
    reason: 'answer',
    durationMs: 1,
    isAborted: false,
    turnId: 't1',
    ...extra,
  } as never)
}

function logLines(world: World, id: string): { event: string; cause?: string; session: string; cwd: string; prompt?: string; named?: unknown; reason?: string; line?: string }[] {
  const text = world.files.get(`${STATE_DIR}/clear-relay/${id}.jsonl`) ?? ''
  return text
    .split('\n')
    .filter(line => line !== '')
    .map(line => JSON.parse(line))
}

const events = (world: World, id: string) => logLines(world, id).map(l => l.event)

test('the line arms; after 10 s the relay runs clear, then submits', OPTIONS, async ($, on) => {
  const world = stage($, on)
  await answer($, `Done.\n\n${LINE}\n`)
  expect(world.toasts[0]).toMatch(/^clear within 10 s \(type or \/stay to keep\)/)
  expect(world.toasts[0]).toContain(PROMPT.slice(0, 20))

  await world.clock.advance(COUNTDOWN_MS - 1)
  expect(world.commands).toEqual([])
  await world.clock.advance(1)
  expect(world.commands).toEqual(['clear'])
  expect(world.submitted).toEqual([PROMPT])
  expect(events(world, 's1')).toEqual(['arm', 'clear', 'submit'])
  expect(logLines(world, 's1')[0]?.cwd).toBe('/work/repo')
  expect(logLines(world, 's1')[2]?.named).toBeNull()
})

test('the submit line records the ticket a resume prompt names', OPTIONS, async ($, on) => {
  const world = stage($, on)
  const prompt = 'resume sre-agent cve-lane t20: bump the shared pin'
  await answer($, `Done.\n\nsafe to /clear; next: \`${prompt}\`\n`)
  await world.clock.advance(COUNTDOWN_MS)
  expect(world.submitted).toEqual([prompt])
  expect(logLines(world, 's1')[2]).toMatchObject({
    event: 'submit',
    named: { repo: 'sre-agent', programme: 'cve-lane', ticket: 't20' },
  })
})

test('a ticket id is the file stem relais prints, slug included', OPTIONS, async ($, on) => {
  const world = stage($, on)
  const prompt = 'resume decisions session-boundaries t26-sessions-claim-their-work: the week count'
  await answer($, `Done.\n\nsafe to /clear; next: \`${prompt}\`\n`)
  await world.clock.advance(COUNTDOWN_MS)
  expect(logLines(world, 's1')[2]).toMatchObject({
    event: 'submit',
    named: { repo: 'decisions', programme: 'session-boundaries', ticket: 't26-sessions-claim-their-work' },
  })
})

test('a prefixed ticket id is named too', OPTIONS, async ($, on) => {
  const world = stage($, on)
  const prompt = 'resume decisions agents-many-nodes an-t02-primary-node: pick the node'
  await answer($, `Done.\n\nsafe to /clear; next: \`${prompt}\`\n`)
  await world.clock.advance(COUNTDOWN_MS)
  expect(logLines(world, 's1')[2]).toMatchObject({
    event: 'submit',
    named: { repo: 'decisions', programme: 'agents-many-nodes', ticket: 'an-t02-primary-node' },
  })
})

test('a resume prompt whose third word is no ticket id names nothing', OPTIONS, async ($, on) => {
  const world = stage($, on)
  const prompt = 'resume homelab fix bug: the flapping link'
  await answer($, `Done.\n\nsafe to /clear; next: \`${prompt}\`\n`)
  await world.clock.advance(COUNTDOWN_MS)
  expect(world.submitted).toEqual([prompt])
  expect(logLines(world, 's1')[2]).toMatchObject({ event: 'submit', named: null })
})

// The two prompt forms of a size handoff (ADR-0083), as the greenbelay
// clear-at-boundary size Continue asks for them.
const SIZE_OPENING = 'size boundary at 450k: handing off mid-phase (off: git config --global greenbelay.clear-at-boundary.size-stance observe)'

test('a mid-phase size handoff in a programme arms and names its ticket', OPTIONS, async ($, on) => {
  const world = stage($, on)
  const prompt = 'resume greenbelay follow-ups t02-three-sessions-one-owner: mid-phase, the owner check'
  await answer($, `${SIZE_OPENING}\n\nHandoff committed.\n\nsafe to /clear; next: \`${prompt}\`\n`)
  await world.clock.advance(COUNTDOWN_MS)
  expect(world.submitted).toEqual([prompt])
  expect(logLines(world, 's1')[2]).toMatchObject({
    event: 'submit',
    named: { repo: 'greenbelay', programme: 'follow-ups', ticket: 't02-three-sessions-one-owner' },
  })
})

test('a mid-phase size handoff outside a programme arms and names nothing', OPTIONS, async ($, on) => {
  const world = stage($, on)
  const prompt = 'resume Phase 3: size kind; worktree /work/repo-wt-size, plan docs/plans/2026-11-23-p.md; read its ## Handoff'
  await answer($, `${SIZE_OPENING}\n\nHandoff committed.\n\nsafe to /clear; next: \`${prompt}\`\n`)
  await world.clock.advance(COUNTDOWN_MS)
  expect(world.submitted).toEqual([prompt])
  expect(logLines(world, 's1')[2]).toMatchObject({ event: 'submit', named: null })
})

// A regression guard, not this change's evidence: the park line of ADR-0078
// keeps `safe to /clear` and must never match the relay's `next:` pattern.
test('a handoff line with no next: prompt (a park) never arms', OPTIONS, async ($, on) => {
  const world = stage($, on)
  await answer($, 'Done.\n\nsafe to /clear; parked: next due 2026-11-24\n')
  await world.clock.advance(COUNTDOWN_MS)
  expect(world.commands).toEqual([])
  expect(world.submitted).toEqual([])
  expect(events(world, 's1')).toEqual([])
})

test('a keystroke disarms', OPTIONS, async ($, on) => {
  const world = stage($, on)
  await answer($, LINE)
  await $.prompt.edit({ origin: { kind: 'composer' }, text: 'x', cursor: 1, start: 0, end: 0, inputText: 'x' })
  await world.clock.advance(COUNTDOWN_MS)
  expect(world.commands).toEqual([])
  expect(logLines(world, 's1')[1]).toMatchObject({ event: 'disarm', cause: 'keystroke' })
})

test('a non-empty prompt box at the end of the countdown disarms', OPTIONS, async ($, on) => {
  const world = stage($, on)
  await answer($, LINE)
  world.session.draft = 'half a thought'
  await world.clock.advance(COUNTDOWN_MS)
  expect(world.commands).toEqual([])
  expect(logLines(world, 's1')[1]).toMatchObject({ event: 'disarm', cause: 'typing' })
})

test('a submitted prompt disarms', OPTIONS, async ($, on) => {
  const world = stage($, on)
  await answer($, LINE)
  await $.prompt.submit({ text: 'something else' })
  await world.clock.advance(COUNTDOWN_MS)
  expect(world.commands).toEqual([])
  expect(logLines(world, 's1')[1]).toMatchObject({ event: 'disarm', cause: 'prompt-submitted' })
})

test('a started turn disarms', OPTIONS, async ($, on) => {
  const world = stage($, on)
  await answer($, LINE)
  await $.turn.start({ text: 'go', turnId: 't2' })
  await world.clock.advance(COUNTDOWN_MS)
  expect(world.commands).toEqual([])
  expect(logLines(world, 's1')[1]).toMatchObject({ event: 'disarm', cause: 'turn-started' })
})

test('/stay disarms', OPTIONS, async ($, on) => {
  const world = stage($, on)
  await answer($, LINE)
  const stayed = await $.command.run({ command: 'stay' })
  expect(stayed.text).toContain('staying')
  await world.clock.advance(COUNTDOWN_MS)
  expect(world.commands).toEqual([])
  expect(logLines(world, 's1')[1]).toMatchObject({ event: 'disarm', cause: 'stay' })
})

test('an aborted turn, a subagent turn and a refusal do not arm', OPTIONS, async ($, on) => {
  const world = stage($, on)
  await answer($, LINE, { reason: 'aborted', isAborted: true })
  await answer($, LINE, { agentId: 'sub' })
  await answer($, LINE, { reason: 'refusal', refusal: { category: null, explanation: null } })
  await world.clock.advance(COUNTDOWN_MS)
  expect(world.toasts).toEqual([])
  expect(world.commands).toEqual([])
})

test('a line that is not last, an empty or a multi-line prompt do not arm', OPTIONS, async ($, on) => {
  const world = stage($, on)
  await answer($, `${LINE}\nand one more thing`)
  await answer($, 'safe to /clear; next: ``')
  await answer($, 'safe to /clear; next: `first\nsecond`')
  await answer($, `safe to /clear; next: \`${'x'.repeat(2_001)}\``)
  await world.clock.advance(COUNTDOWN_MS)
  // The empty and the overlong prompts are near-misses, said but not armed;
  // a line that is not last and a split prompt's `second`` are no handoff line.
  expect(world.toasts.map(t => t.split('. It arms on')[0])).toEqual([
    'clear-relay: not armed: the prompt is empty',
    'clear-relay: not armed: the prompt is 2001 UTF-16 units, over the 2,000 limit',
  ])
  expect(world.commands).toEqual([])
})

test('a refused clear toasts the prompt and does not submit', OPTIONS, async ($, on) => {
  const world = stage($, on, { clear: true })
  await answer($, LINE)
  await world.clock.advance(COUNTDOWN_MS)
  expect(world.submitted).toEqual([])
  expect(world.toasts.at(-1)).toContain(PROMPT)
  expect(events(world, 's1')).toEqual(['arm', 'failure'])
  expect(logLines(world, 's1')[1]).toMatchObject({
    failure: { kind: 'clear-refused' },
    prompt: PROMPT,
  })
})

test('a second line within 2 minutes does not arm', OPTIONS, async ($, on) => {
  const world = stage($, on)
  await answer($, LINE)
  await world.clock.advance(COUNTDOWN_MS)
  expect(world.commands).toEqual(['clear'])

  await world.clock.advance(TWO_MINUTES_MS - COUNTDOWN_MS - 1_000)
  await answer($, LINE)
  await world.clock.advance(COUNTDOWN_MS)
  expect(world.commands).toEqual(['clear'])

  await world.clock.advance(TWO_MINUTES_MS)
  await answer($, LINE)
  await world.clock.advance(COUNTDOWN_MS)
  expect(world.commands).toEqual(['clear', 'clear'])
})

test("the relay's own submit does not disarm it", OPTIONS, async ($, on) => {
  const world = stage($, on)
  await answer($, LINE)
  await world.clock.advance(COUNTDOWN_MS)
  expect(world.submitted).toEqual([PROMPT])
  expect(events(world, 's1')).toEqual(['arm', 'clear', 'submit'])
})

test("another plugin's submit during the countdown disarms", OPTIONS, async ($, on) => {
  const world = stage($, on)
  await answer($, LINE)
  await $.prompt.submit({ text: 'from elsewhere', origin: { kind: 'plugin', name: 'other' } } as never)
  await world.clock.advance(COUNTDOWN_MS)
  expect(world.commands).toEqual([])
})

test('a refused submit after the clear toasts the prompt', OPTIONS, async ($, on) => {
  const world = stage($, on, { submit: true })
  await answer($, LINE)
  await world.clock.advance(COUNTDOWN_MS)
  expect(world.commands).toEqual(['clear'])
  expect(world.toasts.at(-1)).toContain(PROMPT)
  expect(events(world, 's1')).toEqual(['arm', 'clear', 'failure'])
})

test('/stay works after a clear', OPTIONS, async ($, on) => {
  const world = stage($, on)
  await answer($, LINE)
  await world.clock.advance(COUNTDOWN_MS)
  await world.clock.advance(TWO_MINUTES_MS)
  await answer($, LINE)
  const stayed = await $.command.run({ command: 'stay' })
  expect(stayed.text).toContain('staying')
  await world.clock.advance(COUNTDOWN_MS)
  expect(world.commands).toEqual(['clear'])
})

test('enabled false never arms', { options: { enabled: false, stateDir: STATE_DIR } }, async ($, on) => {
  const world = stage($, on)
  await answer($, LINE)
  await world.clock.advance(COUNTDOWN_MS)
  expect(world.toasts).toEqual([])
  expect(world.commands).toEqual([])
})

test('a turn complete returns the answer unchanged', OPTIONS, async ($, on) => {
  stage($, on)
  const reply = await answer($, `text\n${LINE}`)
  expect(reply.text).toBe(`text\n${LINE}`)
})

test('two sessions each keep all their log lines', OPTIONS, async ($, on) => {
  const world = stage($, on)
  await answer($, LINE)
  await $.command.run({ command: 'stay' })
  world.session.id = 'other'
  await world.clock.advance(TWO_MINUTES_MS)
  await answer($, LINE)
  await $.command.run({ command: 'stay' })
  expect(events(world, 's1')).toEqual(['arm', 'disarm'])
  expect(events(world, 'other')).toEqual(['arm', 'disarm'])
})

// Without a `stateDir` option the log goes where greenbelay keeps its state,
// found the way its settings::state_root() finds it.
const logPaths = (world: World) => [...world.files.keys()].filter(path => path.endsWith('.jsonl'))

test('no stateDir: the log is under ~/.local/state/greenbelay', { options: {} }, async ($, on) => {
  const world = stage($, on)
  await answer($, LINE)
  expect(logPaths(world)).toEqual(['/home/person/.local/state/greenbelay/clear-relay/s1.jsonl'])
})

test('no stateDir: XDG_STATE_HOME wins', { options: {} }, async ($, on) => {
  const world = stage($, on)
  world.env.set('XDG_STATE_HOME', '/xdg/state')
  world.env.set('CLAUDE_CONFIG_DIR', '/relocated')
  await answer($, LINE)
  expect(logPaths(world)).toEqual(['/xdg/state/greenbelay/clear-relay/s1.jsonl'])
})

test('no stateDir: a relocated Claude config keeps its state beside it', { options: {} }, async ($, on) => {
  const world = stage($, on)
  world.env.set('CLAUDE_CONFIG_DIR', '/relocated')
  await answer($, LINE)
  expect(logPaths(world)).toEqual(['/relocated/state/greenbelay/clear-relay/s1.jsonl'])
})

test('no stateDir: an empty XDG_STATE_HOME is ignored', { options: {} }, async ($, on) => {
  const world = stage($, on)
  world.env.set('XDG_STATE_HOME', '')
  await answer($, LINE)
  expect(logPaths(world)).toEqual(['/home/person/.local/state/greenbelay/clear-relay/s1.jsonl'])
})

test('no stateDir: an empty CLAUDE_CONFIG_DIR is ignored', { options: {} }, async ($, on) => {
  const world = stage($, on)
  world.env.set('CLAUDE_CONFIG_DIR', '')
  await answer($, LINE)
  expect(logPaths(world)).toEqual(['/home/person/.local/state/greenbelay/clear-relay/s1.jsonl'])
})

test('no stateDir: USERPROFILE stands in for a missing HOME', { options: {} }, async ($, on) => {
  const world = stage($, on)
  world.env.delete('HOME')
  world.env.set('USERPROFILE', '/users/person')
  await answer($, LINE)
  expect(logPaths(world)).toEqual(['/users/person/.local/state/greenbelay/clear-relay/s1.jsonl'])
})

test('no stateDir and no home at all: no log, no path at the root, and the relay still arms', { options: {} }, async ($, on) => {
  const world = stage($, on)
  world.env.delete('HOME')
  await answer($, LINE)
  expect(logPaths(world)).toEqual([])
  expect(world.toasts[0]).toMatch(/^clear within 10 s/)
})

test('no stateDir: an empty HOME falls through to USERPROFILE', { options: {} }, async ($, on) => {
  const world = stage($, on)
  world.env.set('HOME', '')
  world.env.set('USERPROFILE', '/users/person')
  await answer($, LINE)
  expect(logPaths(world)).toEqual(['/users/person/.local/state/greenbelay/clear-relay/s1.jsonl'])
})

// The grammar both readers share (handoff-cases.ts; greenbelay-guard's tests
// read the same block), each case through the engine on a fresh relay.
function caseAnswer(c: HandoffCase): { answer: string; prompt?: string } {
  if (c.next_repeat !== undefined) {
    const prompt = `${c.next_repeat.unit.repeat(c.next_repeat.count)}${c.next_repeat.tail ?? ''}`
    return { answer: `safe to /clear; next: \`${prompt}\``, prompt }
  }
  return { answer: c.answer ?? '', prompt: c.prompt }
}

test('the shared table holds at least 30 cases', () => {
  expect(HANDOFF_CASES.length).toBeGreaterThanOrEqual(30)
})

for (const c of HANDOFF_CASES) {
  test(`handoff case: ${c.name}`, OPTIONS, async ($, on) => {
    const world = stage($, on)
    const { answer: text, prompt } = caseAnswer(c)
    await answer($, text)
    await world.clock.advance(COUNTDOWN_MS)
    switch (c.kind) {
      case 'next':
        expect(world.commands).toEqual(['clear'])
        expect(world.submitted).toEqual([prompt])
        expect(events(world, 's1')).toEqual(['arm', 'clear', 'submit'])
        break
      case 'parked':
      case 'absent':
        expect(world.commands).toEqual([])
        expect(world.submitted).toEqual([])
        expect(world.toasts).toEqual([])
        expect(events(world, 's1')).toEqual([])
        break
      case 'near-miss':
        expect(world.commands).toEqual([])
        expect(world.submitted).toEqual([])
        expect(events(world, 's1')).toEqual(['near-miss'])
        expect(logLines(world, 's1')[0]?.reason).toBe(c.reason)
        expect(world.toasts.length).toBe(1)
        break
    }
  })
}

test('a near-miss logs its line, then toasts the reason for a minute', OPTIONS, async ($, on) => {
  const world = stage($, on)
  const bare = 'safe to /clear; next: resume decisions session-boundaries t17-inbox: file it'
  await answer($, `Closed t16.\n\n${bare}\n`)
  expect(logLines(world, 's1')[0]).toMatchObject({
    event: 'near-miss',
    reason: 'missing-backticks',
    line: bare,
    session: 's1',
    cwd: '/work/repo',
  })
  expect(world.toasts).toEqual([
    'clear-relay: not armed: the prompt after `next: ` is not wrapped in backticks. It arms on: safe to /clear; next: `<prompt>`, or safe to /clear; parked: <line>. Ask the agent to fix the line, or /clear yourself.',
  ])
  expect(world.toastEvents[0]).toContain('"timeoutMs":60000')
})

test('an overlong near-miss logs its line cut to 2,000 units', OPTIONS, async ($, on) => {
  const world = stage($, on)
  await answer($, `safe to /clear; next: \`${'a'.repeat(5000)}\``)
  const entry = logLines(world, 's1')[0]
  expect(entry?.reason).toBe('overlong')
  expect(entry?.line?.length).toBe(2000)
  expect(world.toasts[0]).toContain('the prompt is 5000 UTF-16 units, over the 2,000 limit')
})

test('a near-miss disarms a pending countdown first', OPTIONS, async ($, on) => {
  const world = stage($, on)
  await answer($, LINE)
  await answer($, 'safe to /clear; next: ``')
  await world.clock.advance(COUNTDOWN_MS)
  expect(world.commands).toEqual([])
  expect(events(world, 's1')).toEqual(['arm', 'disarm', 'near-miss'])
})

test('a disabled relay logs and shows nothing on a near-miss', { options: { enabled: false, stateDir: STATE_DIR } }, async ($, on) => {
  const world = stage($, on)
  await answer($, 'safe to /clear; next: no backticks')
  await world.clock.advance(COUNTDOWN_MS)
  expect(world.toasts).toEqual([])
  expect(events(world, 's1')).toEqual([])
})
