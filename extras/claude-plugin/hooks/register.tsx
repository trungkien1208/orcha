import { atom, read, update } from 'claude-code'
import type { EngineInterface, Register } from 'claude-code'

import type { BranchRow, DayStats, OrchaConfig, TaskInfo, WorkItem } from '../types'

// orcha for Claude Code: an optional task card above the prompt, a branches
// pane, /pr-link, /standup and little cheers. Colors, strings and repo rules come
// from ~/.config/orcha/gen/plugin.json (written by `orcha apply`); task titles
// from orcha's tracker.py (GitHub Issues / Azure DevOps). The guards and the
// check-on-stop are settings hooks (orcha/claude/hooks): those always run,
// while a plugin's tool.check / Stop hooks can be skipped by one above it.

const PANE = 'orcha-branches'
const MAX_BRANCHES = 60
const BATCH = 8

const branches = atom({ plugin: 'orcha', key: 'branches' } as const, [])
const isLoading = atom({ plugin: 'orcha', key: 'isLoading' } as const, false)
const updatedAt = atom({ plugin: 'orcha', key: 'updatedAt' } as const, null)
const task = atom({ plugin: 'orcha', key: 'task' } as const, null)
const isCardHidden = atom({ plugin: 'orcha', key: 'isCardHidden' } as const, false)

type $ = EngineInterface

const FALLBACK: OrchaConfig = {
  home: '',
  cache: '',
  colors: {
    accent: '#ffc4dc', red: '#ffa3bd', green: '#bfe8cc', blue: '#bccaf5', yellow: '#f5dbb5',
    magenta: '#f5bfe6', muted: '#a08d99', fg: '#fff0f6', peach: '#ffcdb5',
  },
  strings: {},
  repo: { branch_prefix: '', base: '', protected: '^(release/.+|develop|main|master)$' },
  widgets: { card: false, cheers: false },
}

// Read once per session start / reload: `orcha apply` changes are picked up
// by the next session.
let conf: OrchaConfig = FALLBACK

const loadConfig = async ($: $) => {
  try {
    const home = (await $.process.run(['sh', '-c', 'printf %s "${XDG_CONFIG_HOME:-$HOME/.config}"'])).stdout
    conf = { ...FALLBACK, ...(JSON.parse(await $.fs.read(`${home}/orcha/gen/plugin.json`)) as Partial<OrchaConfig>) }
  } catch {
    conf = FALLBACK
  }
}

// S('m_commit_nth', { n: 3 }): a string in the chosen language, {name} filled in
const S = (key: string, vars: Record<string, string | number> = {}) => {
  const raw = conf.strings[key]
  const text = Array.isArray(raw) ? raw[Math.floor(Math.random() * raw.length)] ?? '' : raw ?? key
  return text.replace(/\{(\w+)\}/g, (_, k: string) => String(vars[k] ?? ''))
}
const C = () => conf.colors

const git = async ($: $, args: string[], cwd?: string) => {
  const r = await $.process.run(['git', ...args], { cwd, timeoutMs: 20_000 })
  return { ok: r.exitCode === 0, out: r.stdout.trim() }
}

const currentBranch = async ($: $) => (await git($, ['branch', '--show-current'])).out

// Upstream of HEAD, or origin/<branch> when no upstream is set but it exists there.
const remoteOf = async ($: $, branch: string) => {
  const upstream = await git($, ['rev-parse', '--abbrev-ref', '@{u}'])
  if (upstream.ok) return upstream.out
  const same = await git($, ['rev-parse', '--verify', '--quiet', `refs/remotes/origin/${branch}`])
  return same.ok ? `origin/${branch}` : null
}

// The repo rules: config.json's "repo", with the repo's own .orcha.json on top.
const rulesOf = async ($: $, top: string) => {
  try {
    return { ...conf.repo, ...(JSON.parse(await $.fs.read(`${top}/.orcha.json`)) as Partial<OrchaConfig['repo']>) }
  } catch {
    return conf.repo
  }
}

// The branch PRs go into: the rule "base", else origin's default branch.
const baseOf = async ($: $, base: string) => {
  if (base) return base
  const head = await git($, ['symbolic-ref', '--short', 'refs/remotes/origin/HEAD'])
  return head.ok ? head.out.replace(/^origin\//, '') : 'main'
}

const isProtected = (branch: string, pattern: string) => {
  try {
    return !!pattern && new RegExp(pattern).test(branch)
  } catch {
    return false
  }
}

const mapInBatches = async <T, R>(items: T[], fn: (item: T) => Promise<R>) => {
  const out: R[] = []
  for (let i = 0; i < items.length; i += BATCH) out.push(...(await Promise.all(items.slice(i, i + BATCH).map(fn))))
  return out
}

const clip = (s: string, n: number) => (s.length > n ? s.slice(0, Math.max(1, n - 1)) + '…' : s)

const stateColor = (state: string | null | undefined) => {
  const s = (state ?? '').toLowerCase()
  if (/done|closed|resolved|merged/.test(s)) return C().green
  if (/removed|cancel/.test(s)) return C().red
  if (/progress|active|committed|doing|review|test/.test(s)) return C().yellow
  return C().blue
}

// ---------- tracker.py: task ids and titles (GitHub / Azure DevOps)

const tracker = async ($: $, args: string[]) => {
  if (!conf.home) return ''
  const r = await $.process
    .run(['python3', '-I', `${conf.home}/scripts/tracker.py`, ...args], { timeoutMs: 30_000 })
    .catch(() => null)
  return r && r.exitCode === 0 ? r.stdout.trim() : ''
}

const taskIdOf = async ($: $, branch: string) => (await tracker($, ['id', branch])) || null

const fetchItems = async ($: $, ids: string[], top: string): Promise<Map<string, WorkItem>> => {
  const found = new Map<string, WorkItem>()
  if (ids.length === 0) return found
  try {
    const data = JSON.parse((await tracker($, ['get', ...new Set(ids), '--repo', top])) || '{}') as Record<string, WorkItem>
    for (const [id, item] of Object.entries(data)) found.set(id, item)
  } catch {
    // no titles this time
  }
  return found
}

// ---------- today's stats, streak, badges

const DAY_MS = 86_400_000
const dayKey = (d: Date) => `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`

const loadDayStats = async ($: $): Promise<DayStats> => {
  const email = (await git($, ['config', 'user.email'])).out
  const isCleanCheck = (await $.store.get('check-clean-day')) === dayKey(new Date())
  if (!email) return { commits: 0, added: 0, removed: 0, streak: 0, firstHour: null, lastHour: null, isCleanCheck }
  const today = await git($, [
    'log', '--all', '--no-merges', '--since=midnight', `--author=${email}`, '--shortstat', '--format=@%H %ad', '--date=format:%H',
  ])
  let commits = 0
  let added = 0
  let removed = 0
  const hours: number[] = []
  for (const line of today.out.split('\n')) {
    if (line.startsWith('@')) {
      commits += 1
      hours.push(Number(line.split(' ')[1] ?? 0))
    }
    added += Number(line.match(/(\d+) insertion/)?.[1] ?? 0)
    removed += Number(line.match(/(\d+) deletion/)?.[1] ?? 0)
  }
  const firstHour = hours.length ? Math.min(...hours) : null
  const lastHour = hours.length ? Math.max(...hours) : null
  // streak: consecutive days with a commit, counting from today (or yesterday,
  // so the streak is not "broken" before you commit today)
  const days = new Set(
    (await git($, ['log', '--all', '--no-merges', '--since=90.days', `--author=${email}`, '--format=%cd', '--date=short']))
      .out.split('\n').filter(Boolean),
  )
  let streak = 0
  let day = new Date()
  if (!days.has(dayKey(day))) day = new Date(day.getTime() - DAY_MS)
  while (days.has(dayKey(day))) {
    streak += 1
    day = new Date(day.getTime() - DAY_MS)
  }
  return { commits, added, removed, streak, firstHour, lastHour, isCleanCheck }
}

// Badges for the task card, earned from today's stats.
const badgesOf = (d: DayStats) => {
  const out: string[] = []
  if (d.firstHour !== null && d.firstHour < 9) out.push(S('m_badge_early'))
  if (d.lastHour !== null && d.lastHour >= 22) out.push(S('m_badge_night'))
  if (d.removed >= 20 && d.removed > d.added) out.push(S('m_badge_cleanup'))
  if (d.isCleanCheck) out.push(S('m_badge_first_try'))
  if (d.commits >= 10) out.push(S('m_badge_fast'))
  if (d.streak >= 5) out.push(S('m_badge_streak', { n: d.streak }))
  return out
}

// The check Stop hook (orcha-check.sh) writes "<epoch> <fix rounds>" on a pass.
const checkPass = async ($: $) => {
  if (!conf.cache) return
  const text = await $.fs.read(`${conf.cache}/check-pass`).catch(() => '')
  const [atText = '0', roundsText = '1'] = text.trim().split(/\s+/)
  const at = Number(atText) || 0
  if (!at) return
  const seen = Number((await $.store.get('check-pass-seen')) ?? 0)
  if (at <= seen) return
  await $.store.set('check-pass-seen', at)
  // only for a fresh pass, not one from hours ago
  if (Date.now() / 1000 - at >= 600) return
  const isFirstTry = roundsText === '0'
  if (isFirstTry) await $.store.set('check-clean-day', dayKey(new Date()))
  if (conf.widgets.cheers) $.ui.toast(isFirstTry ? S('m_check_first_try') : S('m_cheer_check'))
}

const isFridayAfternoon = () => {
  const now = new Date()
  return now.getDay() === 5 && now.getHours() >= 15
}

// ---------- status line + task card

const refreshTask = async ($: $) => {
  const top = await git($, ['rev-parse', '--show-toplevel'])
  if (!top.ok) {
    await update($, task, () => null)
    return
  }
  const rules = await rulesOf($, top.out)
  const repo = top.out.split('/').pop() ?? ''
  const branch = (await currentBranch($)) || '(detached)'
  const id = await taskIdOf($, branch)
  const remote = await remoteOf($, branch)
  const unpushed = remote ? Number((await git($, ['rev-list', '--count', `${remote}..HEAD`])).out || 0) : null
  const isProt = isProtected(branch, rules.protected)
  const item = id ? (await fetchItems($, [id], top.out)).get(id) ?? null : null
  const today = await loadDayStats($)

  const info: TaskInfo = { today, repo, branch, id, item, unpushed, isProtected: isProt }
  await update($, task, () => info)
  void checkPass($)

  // no status line: the herdr header shows the focused pane's task, branch and
  // push state. The data above feeds the card (widget "card"), cheers, /pr-link
}

// ---------- your branches pane

const loadBranches = async ($: $) => {
  await update($, isLoading, () => true)
  try {
    const top = (await git($, ['rev-parse', '--show-toplevel'])).out
    const rules = await rulesOf($, top)
    const prefix = rules.branch_prefix
    const base = `origin/${await baseOf($, rules.base)}`
    const refs = await git($, [
      'for-each-ref', '--sort=-committerdate',
      '--format=%(refname:short)|%(upstream:short)|%(committerdate:relative)',
      `refs/heads/${prefix}`,
    ])
    const remote = new Set(
      (await git($, ['for-each-ref', '--format=%(refname:short)', `refs/remotes/origin/${prefix}`]))
        .out.split('\n').filter(Boolean),
    )
    const mergedTitles = (await git($, ['log', base, '--since=12.months', '--grep=^Merged', '--format=%s'])).out

    const lines = refs.out.split('\n').filter(Boolean)
      .filter(l => !isProtected(l.split('|')[0] ?? '', rules.protected)).slice(0, MAX_BRANCHES)
    const ids = await mapInBatches(lines, l => taskIdOf($, l.split('|')[0] ?? ''))
    const items = await fetchItems($, ids.filter((p): p is string => !!p), top)
    const rows: BranchRow[] = await mapInBatches(lines.map((l, i) => [l, ids[i] ?? null] as const), async ([line, id]) => {
      const [name = '', , age = ''] = line.split('|')
      const isPushed = remote.has(`origin/${name}`)
      const lr = (await git($, ['rev-list', '--left-right', '--count', `${base}...${name}`])).out.split(/\s+/)
      const behind = Number(lr[0] || 0)
      const ahead = Number(lr[1] || 0)
      const unpushed = isPushed
        ? Number((await git($, ['rev-list', '--count', `origin/${name}..${name}`])).out || 0)
        : ahead
      const item = id ? items.get(id) : undefined
      const merged: BranchRow['merged'] =
        ahead === 0 ? 'yes' : id && mergedTitles.includes(id) ? 'pr' : 'no'
      return { name, id, title: item?.title ?? null, state: item?.state ?? null, isPushed, unpushed, ahead, behind, merged, age, base }
    })
    await update($, branches, () => rows)
    await update($, updatedAt, () => Date.now())
  } finally {
    await update($, isLoading, () => false)
  }
}

// ---------- /pr-link: the create-PR page + a description

const prText = async ($: $, wanted: string) => {
  const top = (await git($, ['rev-parse', '--show-toplevel'])).out
  const rules = await rulesOf($, top)
  const target = wanted || (await baseOf($, rules.base))
  const branch = await currentBranch($)
  if (!branch) return S('m_pr_detached')
  if (isProtected(branch, rules.protected)) return S('m_pr_on_protected', { branch })
  const url = await tracker($, ['pr', branch, target, '--repo', top])
  if (!url) return S('m_pr_no_remote')

  const warn: string[] = []
  const remote = await remoteOf($, branch)
  if (!remote) warn.push(`⚠ ${S('m_pr_push_first', { branch })}`)
  else {
    const n = Number((await git($, ['rev-list', '--count', `${remote}..HEAD`])).out || 0)
    if (n > 0) warn.push(`⚠ ${S('m_pr_unpushed', { n })}`)
  }

  const commits = (await git($, ['log', '--no-merges', '--reverse', '--format=- %s', `origin/${target}..HEAD`])).out
  const id = await taskIdOf($, branch)
  const item = id ? (await fetchItems($, [id], top)).get(id) : undefined
  const slug = branch.split('/').pop()!.replace(/^\d+[-_]?/, '').replace(/[-_]+/g, ' ').trim()
  const title = `${id ? `${id} ` : ''}${item?.title ?? slug}`

  const description = [
    id ? `Task: #${id}${item ? ` — ${item.title}` : ''}${item?.url ? `\n${item.url}` : ''}` : null,
    item?.summary ? `\n> ${item.summary}` : null,
    '',
    '## Changes',
    commits || `- (no commits ahead of ${target})`,
  ].filter(l => l !== null).join('\n')

  await $.process.run(['open', url]).catch(() => undefined)

  return [
    ...warn,
    warn.length ? '' : null,
    S('m_pr_opened', { branch, target }),
    url,
    '',
    `Title: ${title}`,
    '',
    'Description:',
    '```',
    description,
    '```',
  ].filter(l => l !== null).join('\n')
}

// ---------- register

export const register: Register = on => {
  on('session.start', async ($, e, next) => {
    await loadConfig($)
    await $.command.register({ name: 'pr-link', description: S('m_cmd_pr') })
    await $.command.register({ name: 'branches', description: S('m_cmd_branches') })
    await $.command.register({ name: 'task', description: S('m_cmd_task') })
    await $.command.register({ name: 'standup', description: S('m_cmd_standup') })
    void refreshTask($)
    $.clock.every(30_000, () => void refreshTask($))
    return next(e)
  })

  on('turn.complete', async ($, e, next) => {
    void refreshTask($)
    return next(e)
  })

  // keep the card fresh after git commands, and cheer a little when a commit
  // or push lands
  on('tool.call', { tool: 'Bash' }, async ($, e, next) => {
    const ran = await next(e)
    if (!/\bgit\s+(commit|push|merge|rebase|reset|checkout|switch|fetch|pull)\b/.test(e.command)) return ran
    const isOk = ran.deny === undefined && !ran.isError
    const before = (await read($, task))?.today.commits ?? 0
    await refreshTask($)
    if (!isOk || !conf.widgets.cheers) return ran

    const info = await read($, task)
    if (/\bgit\s+commit\b/.test(e.command) && !/--dry-run/.test(e.command)) {
      const now = info?.today.commits ?? before + 1
      const milestone = { 5: 'm_milestone_5', 10: 'm_milestone_10', 15: 'm_milestone_15' }[now]
      $.ui.toast(milestone ? S(milestone) : `${S('m_cheer_commit')}  ·  ${S('m_commit_nth', { n: now })}`)
    }
    if (/\bgit\s+push\b/.test(e.command)) {
      const isRelease = /\brelease\//.test(e.command) || (info?.isProtected ?? false)
      $.ui.toast(isRelease && isFridayAfternoon() ? S('m_friday_release') : S('m_cheer_push'))
    }
    return ran
  })

  on('command.run', { command: 'task' }, async $ => {
    await update($, isCardHidden, v => !v)
    void refreshTask($)
    return { text: (await read($, isCardHidden)) ? S('m_card_hidden') : S('m_card_shown') }
  })

  // the task card above the prompt, for a branch with a task
  on('ui.render', { component: 'AbovePrompt' }, async ($, e, next) => {
    const info = await read($, task)
    if (!conf.widgets.card || e.props.hasSurvey || !info || !info.id || (await read($, isCardHidden))) return next(e)

    const { Box, Text, Button } = $.ui.resolve(e)
    const c = C()
    const width = Math.max(30, e.props.bodyColumns - 4)
    const item = info.item
    const push =
      info.unpushed === null
        ? { text: `⚠ ${S('m_not_pushed_origin')}`, color: c.yellow }
        : info.unpushed > 0
          ? { text: `↑ ${S('m_unpushed_n', { n: info.unpushed })}`, color: c.yellow }
          : { text: `✓ ${S('m_pushed')}`, color: c.green }
    const badges = badgesOf(info.today)

    return (
      <Box flexDirection="column" borderStyle="round" borderColor={info.isProtected ? c.red : c.accent} paddingX={1}>
        <Text>
          <Text color={c.accent} bold>󰄬 #{info.id}</Text>
          {item && <Text color={c.muted}>  {item.type}  </Text>}
          {item && <Text color={stateColor(item.state)} bold>● {item.state}</Text>}
          {item?.assignee && <Text color={c.muted}>  ·  {item.assignee}</Text>}
        </Text>
        {item && <Text color={c.fg} bold>{clip(item.title, width)}</Text>}
        {!item && <Text color={c.muted}>{S('m_no_title')}</Text>}
        {item?.summary && <Text color={c.muted}>{clip(item.summary, width * 2)}</Text>}
        <Box>
          <Text>
            <Text color={c.blue}> {clip(info.branch, Math.max(20, width - 40))}</Text>
            <Text color={c.muted}>  ·  </Text>
            <Text color={push.color}>{push.text}</Text>
            {info.isProtected && <Text color={c.red}>  ·  🔒 {S('m_protected')}</Text>}
            <Text color={c.muted}>  </Text>
          </Text>
          <Button key="hide" plain label={S('m_hide')}
            onPress={async () => { await update($, isCardHidden, () => true); void refreshTask($) }} />
        </Box>
        <Text>
          {info.today.commits === 0 ? (
            <Text color={c.muted}>{S('m_no_commit_today')}</Text>
          ) : (
            <Text>
              <Text color={c.magenta}>{S('m_today_commits', { n: info.today.commits })}</Text>
              <Text color={c.muted}>  ·  </Text>
              <Text color={c.green}>+{info.today.added}</Text>
              <Text color={c.muted}>/</Text>
              <Text color={c.red}>−{info.today.removed}</Text>
            </Text>
          )}
          {info.today.streak >= 2 && (
            <Text>
              <Text color={c.muted}>  ·  </Text>
              <Text color={c.yellow}>{S('m_streak', { n: info.today.streak })}</Text>
            </Text>
          )}
        </Text>
        {badges.length > 0 && (
          <Text>
            <Text color={c.muted}>🏅 </Text>
            <Text color={c.yellow}>{badges.join('   ')}</Text>
          </Text>
        )}
      </Box>
    )
  })

  // /standup: yesterday / today / blockers from your commits, grouped by task
  on('command.run', { command: 'standup' }, async $ => {
    const r = conf.home
      ? await $.process.run(['python3', '-I', `${conf.home}/scripts/worklog.py`, 'standup'], { timeoutMs: 90_000 }).catch(() => null)
      : null
    const text = r?.stdout.trim()
    if (!text) return { text: S('m_standup_failed') }
    await $.process.run(['pbcopy'], { stdin: text }).catch(() => undefined)
    $.ui.toast(S('m_standup_copied'))
    return { text: `${text}\n\n(${S('m_copied')})` }
  })

  on('command.run', { command: 'branches' }, async $ => {
    const top = (await git($, ['rev-parse', '--show-toplevel'])).out
    const prefix = (await rulesOf($, top)).branch_prefix
    await $.ui.open({ id: PANE, title: ` ${S('m_branches_title')} ${prefix ? `${prefix}*` : ''}` })
    void loadBranches($)
    return { text: S('m_branches_opened') }
  })

  on('ui.render', { component: 'Pane', requestId: PANE }, async ($, e) => {
    const { Box, Text, Button } = $.ui.resolve(e)
    const c = C()
    const rows = await read($, branches)
    const loading = await read($, isLoading)
    const at = await read($, updatedAt)
    const titleWidth = Math.max(16, e.props.bodyColumns - 6)
    // each branch takes two lines plus a blank one
    const room = Math.max(1, Math.floor(((e.viewport?.rows ?? 30) - 6) / 3))
    const open = rows.filter(r => r.merged === 'no').length

    return (
      <Box flexDirection="column">
        <Box>
          <Button key="refresh" variant="primary" label={loading ? S('m_loading') : S('m_refresh')} onPress={() => void loadBranches($)} />
          <Text> </Text>
          <Button
            key="fetch"
            label="git fetch"
            onPress={async () => {
              await update($, isLoading, () => true)
              await $.process.run(['git', 'fetch', 'origin', '--prune'], { timeoutMs: 120_000 }).catch(() => undefined)
              await loadBranches($)
            }}
          />
        </Box>
        <Text color={c.muted}>
          {at ? S('m_branches_summary', { n: rows.length, open, time: new Date(at).toLocaleTimeString() }) : ' '}
        </Text>
        <Text> </Text>
        {rows.length === 0 && <Text color={c.muted}>{loading ? S('m_loading') : S('m_no_branches')}</Text>}
        {rows.slice(0, room).map(r => {
          const isMerged = r.merged !== 'no'
          const head = r.title ? `${r.id ? `#${r.id} ` : ''}${r.title}` : r.name
          const push = !r.isPushed
            ? { text: `⚠ ${S('m_not_pushed')}`, color: c.yellow }
            : r.unpushed > 0
              ? { text: `↑${r.unpushed} ${S('m_not_pushed')}`, color: c.yellow }
              : { text: `✓ ${S('m_pushed')}`, color: c.muted }
          return (
            <Box flexDirection="column" marginBottom={1}>
              <Text>
                <Text color={isMerged ? c.green : c.accent}>{isMerged ? '✓ ' : '● '}</Text>
                <Text color={isMerged ? c.muted : c.fg} bold={!isMerged}>{clip(head, titleWidth)}</Text>
              </Text>
              <Text>
                <Text color={c.muted}>  </Text>
                {r.state && <Text color={stateColor(r.state)}>{r.state}  </Text>}
                <Text color={push.color}>{push.text}</Text>
                <Text color={c.muted}>  ·  +{r.ahead}/-{r.behind} {r.base.replace(/^origin\//, '')}  ·  </Text>
                <Text color={isMerged ? c.green : c.muted}>
                  {r.merged === 'yes' ? S('m_merged') : r.merged === 'pr' ? S('m_merged_pr') : S('m_not_merged')}
                </Text>
                <Text color={c.muted}>  ·  {r.age}</Text>
              </Text>
            </Box>
          )
        })}
        {rows.length > room && <Text color={c.muted}>{S('m_more_branches', { n: rows.length - room })}</Text>}
      </Box>
    )
  })

  on('command.run', { command: 'pr-link' }, async ($, e) => {
    const text = await prText($, e.args.trim())
    if (/pullrequestcreate|\/compare\//.test(text) && !text.startsWith('⚠') && conf.widgets.cheers) $.ui.toast(S('m_cheer_pr'))
    return { text }
  })
}
